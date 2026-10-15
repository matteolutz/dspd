//! Live audio through the OS audio API (WASAPI, CoreAudio, ALSA/JACK) via cpal.
//!
//! This is the development backend for Windows and macOS and a fallback on
//! Linux. It is *not* the low-latency path for the real server: that will be
//! a JACK or direct-ALSA backend on the RT-tuned Linux box.
//!
//! Layout: an input stream pushes samples into a lock-free ring buffer; the
//! output stream callback pulls them, runs the processor and writes the
//! result. All buffers are allocated before the streams start.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, SampleRate, StreamConfig};
use dspd_engine::{AudioBlock, Processor};

use crate::{AudioBackend, RunReport};

pub struct CpalBackend {
    in_channels: usize,
    out_channels: usize,
    block_size: usize,
    sample_rate: u32,
    input_device: Option<String>,
    output_device: Option<String>,
    use_input: bool,
}

/// Names of the available (input, output) devices on the default host.
pub fn list_devices() -> Result<(Vec<String>, Vec<String>)> {
    let host = cpal::default_host();
    let names = |devices: Result<Vec<Device>, _>| -> Vec<String> {
        devices
            .unwrap_or_default()
            .iter()
            .filter_map(|d| d.name().ok())
            .collect()
    };
    Ok((
        names(host.input_devices().map(|d| d.collect())),
        names(host.output_devices().map(|d| d.collect())),
    ))
}

fn find_device(host: &cpal::Host, name: Option<&str>, input: bool) -> Result<Option<Device>> {
    let Some(name) = name else {
        return Ok(if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        });
    };
    let mut devices: Vec<Device> = if input {
        host.input_devices()?.collect()
    } else {
        host.output_devices()?.collect()
    };
    devices.retain(|d| d.name().is_ok_and(|n| n.contains(name)));
    devices.into_iter().next().map(Some).ok_or_else(|| {
        anyhow!(
            "no {} device matching `{name}`",
            if input { "input" } else { "output" }
        )
    })
}

impl CpalBackend {
    /// `in_channels` and `out_channels` are opened on the devices as asked; a
    /// device that cannot provide that many channels makes `run` fail.
    pub fn new(
        in_channels: usize,
        out_channels: usize,
        block_size: usize,
        sample_rate: u32,
    ) -> Self {
        Self {
            in_channels,
            out_channels,
            block_size,
            sample_rate,
            input_device: None,
            output_device: None,
            use_input: true,
        }
    }

    /// Select the input device by (partial) name instead of the system default.
    pub fn input_device(mut self, name: impl Into<String>) -> Self {
        self.input_device = Some(name.into());
        self
    }

    /// Select the output device by (partial) name instead of the system default.
    pub fn output_device(mut self, name: impl Into<String>) -> Self {
        self.output_device = Some(name.into());
        self
    }

    /// Disable capture; the processor is fed silence. Useful with plugins that generate sound.
    pub fn without_input(mut self) -> Self {
        self.use_input = false;
        self
    }
}

impl AudioBackend for CpalBackend {
    fn run(
        &mut self,
        mut processor: Box<dyn Processor + Send>,
        stop: Arc<AtomicBool>,
    ) -> Result<RunReport> {
        let started = Instant::now();
        let host = cpal::default_host();

        let out_dev = find_device(&host, self.output_device.as_deref(), false)?
            .context("no output device available")?;
        let out_config = StreamConfig {
            channels: self.out_channels as u16,
            sample_rate: SampleRate(self.sample_rate),
            buffer_size: BufferSize::Fixed(self.block_size as u32),
        };

        // Input is optional: no device (or disabled) means the chain gets silence.
        let in_dev = if self.use_input {
            find_device(&host, self.input_device.as_deref(), true)?
        } else {
            None
        };
        let in_channels = if in_dev.is_some() {
            self.in_channels
        } else {
            1
        };

        let errors = Arc::new(AtomicU64::new(0));
        let blocks = Arc::new(AtomicU64::new(0));
        let frames = Arc::new(AtomicU64::new(0));

        // Ring buffer between the input callback and the output callback:
        // room for about 16 blocks of interleaved samples.
        let (mut producer, mut consumer) =
            rtrb::RingBuffer::<f32>::new(self.block_size * in_channels * 16);

        let in_stream = match &in_dev {
            Some(dev) => {
                let cfg = StreamConfig {
                    channels: in_channels as u16,
                    sample_rate: SampleRate(self.sample_rate),
                    buffer_size: BufferSize::Fixed(self.block_size as u32),
                };
                let errs = Arc::clone(&errors);
                let stream = dev
                    .build_input_stream(
                        &cfg,
                        move |data: &[f32], _| {
                            for &s in data {
                                // If the ring is full the output side is stalled; drop the sample.
                                let _ = producer.push(s);
                            }
                        },
                        move |e| {
                            errs.fetch_add(1, Ordering::Relaxed);
                            eprintln!("input stream error: {e}");
                        },
                        None,
                    )
                    .with_context(|| format!("opening input stream ({in_channels} ch, {} Hz, block {}); the device may not support that as f32", self.sample_rate, self.block_size))?;
                Some(stream)
            }
            None => None,
        };
        let has_input = in_stream.is_some();

        let out_channels = self.out_channels;
        let block = self.block_size;
        let mut input = AudioBlock::new(in_channels, block);
        let mut output = AudioBlock::new(out_channels, block);
        let (blocks_cb, frames_cb, errs) = (
            Arc::clone(&blocks),
            Arc::clone(&frames),
            Arc::clone(&errors),
        );

        let out_stream = out_dev
            .build_output_stream(
                &out_config,
                move |data: &mut [f32], _| {
                    let total = data.len() / out_channels;
                    let mut offset = 0;
                    while offset < total {
                        let n = (total - offset).min(block);
                        input.set_frames(n);
                        output.set_frames(n);
                        if has_input {
                            for f in 0..n {
                                for c in 0..in_channels {
                                    input.channel_mut(c)[f] = consumer.pop().unwrap_or(0.0);
                                }
                            }
                        }
                        processor.process(&input, &mut output);
                        for f in 0..n {
                            for c in 0..out_channels {
                                data[(offset + f) * out_channels + c] = output.channel(c)[f];
                            }
                        }
                        offset += n;
                        blocks_cb.fetch_add(1, Ordering::Relaxed);
                    }
                    frames_cb.fetch_add(total as u64, Ordering::Relaxed);
                },
                move |e| {
                    errs.fetch_add(1, Ordering::Relaxed);
                    eprintln!("output stream error: {e}");
                },
                None,
            )
            .with_context(|| format!("opening output stream ({out_channels} ch, {} Hz, block {}); the device may not support that as f32", self.sample_rate, self.block_size))?;

        if let Some(s) = &in_stream {
            s.play().context("starting input stream")?;
        }
        out_stream.play().context("starting output stream")?;

        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
            if errors.load(Ordering::Relaxed) > 1000 {
                bail!("audio device keeps reporting errors; giving up");
            }
        }
        drop(out_stream);
        drop(in_stream);

        let f = frames.load(Ordering::Relaxed);
        Ok(RunReport {
            frames_in: if has_input { f } else { 0 },
            frames_out: f,
            blocks: blocks.load(Ordering::Relaxed),
            wall: started.elapsed(),
            stream_errors: errors.load(Ordering::Relaxed),
        })
    }
}
