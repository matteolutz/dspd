//! WAV file in, WAV file out, as fast as the CPU allows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use dspd_engine::{AudioBlock, Processor};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::{AudioBackend, RunReport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

/// Read just the header of a WAV file.
pub fn probe_wav(path: &Path) -> Result<WavInfo> {
    let reader =
        WavReader::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let spec = reader.spec();
    Ok(WavInfo {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        frames: u64::from(reader.duration()),
    })
}

/// Renders `input` through the processor into `output` (32-bit float WAV).
pub struct OfflineBackend {
    input: PathBuf,
    output: PathBuf,
    out_channels: usize,
    block_size: usize,
    tail_frames: u64,
    compensate_latency: bool,
}

impl OfflineBackend {
    /// * `out_channels` must equal the chain's channel count.
    /// * `block_size` must not exceed the session's `max_block`.
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        out_channels: usize,
        block_size: usize,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            out_channels,
            block_size,
            tail_frames: 0,
            compensate_latency: true,
        }
    }

    /// Keep rendering this many frames of silence after the input ends
    /// (reverb and delay tails).
    pub fn tail_frames(mut self, frames: u64) -> Self {
        self.tail_frames = frames;
        self
    }

    /// When on (the default) the processor's reported latency is rendered as
    /// extra silence at the end and trimmed from the start, so the output is
    /// time-aligned with the input.
    pub fn compensate_latency(mut self, on: bool) -> Self {
        self.compensate_latency = on;
        self
    }
}

/// Streams a WAV file as f32 samples regardless of its on-disk format.
fn sample_stream(
    reader: WavReader<std::io::BufReader<std::fs::File>>,
) -> Result<Box<dyn Iterator<Item = hound::Result<f32>>>> {
    let spec = reader.spec();
    Ok(match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => Box::new(reader.into_samples::<f32>()),
        (SampleFormat::Int, bits @ 1..=32) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            Box::new(
                reader
                    .into_samples::<i32>()
                    .map(move |s| s.map(|v| v as f32 * scale)),
            )
        }
        (fmt, bits) => bail!("unsupported WAV sample format: {fmt:?} {bits}-bit"),
    })
}

impl AudioBackend for OfflineBackend {
    fn run(
        &mut self,
        mut processor: Box<dyn Processor + Send>,
        stop: Arc<AtomicBool>,
    ) -> Result<RunReport> {
        let started = Instant::now();

        let reader = WavReader::open(&self.input)
            .with_context(|| format!("cannot open {}", self.input.display()))?;
        let in_spec = reader.spec();
        let in_channels = in_spec.channels as usize;
        let in_frames = u64::from(reader.duration());
        let mut samples = sample_stream(reader)?;

        let latency = if self.compensate_latency {
            u64::from(processor.latency_frames())
        } else {
            0
        };
        let total_frames = in_frames + self.tail_frames + latency;
        let mut to_skip = latency;

        let mut writer = WavWriter::create(
            &self.output,
            WavSpec {
                channels: self.out_channels as u16,
                sample_rate: in_spec.sample_rate,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
        )
        .with_context(|| format!("cannot create {}", self.output.display()))?;

        // All buffers are allocated once, before the loop.
        let mut input = AudioBlock::new(in_channels, self.block_size);
        let mut output = AudioBlock::new(self.out_channels, self.block_size);
        let mut interleaved = vec![0.0f32; self.block_size * in_channels];

        let mut report = RunReport::default();
        let mut done = 0u64;
        while done < total_frames && !stop.load(Ordering::Relaxed) {
            let n = (total_frames - done).min(self.block_size as u64) as usize;

            // Pull up to n frames from the file; anything missing is silence.
            let wanted = n * in_channels;
            let mut got = 0;
            while got < wanted {
                match samples.next() {
                    Some(s) => {
                        interleaved[got] = s.context("error reading WAV data")?;
                        got += 1;
                    }
                    None => break,
                }
            }
            interleaved[got..wanted].fill(0.0);
            let got_frames = got / in_channels;

            input.set_frames(n);
            for c in 0..in_channels {
                let ch = input.channel_mut(c);
                for (f, d) in ch.iter_mut().enumerate() {
                    *d = interleaved[f * in_channels + c];
                }
            }
            report.frames_in += got_frames as u64;

            output.set_frames(n);
            processor.process(&input, &mut output);
            report.blocks += 1;

            for f in 0..n {
                if to_skip > 0 {
                    to_skip -= 1;
                    continue;
                }
                for c in 0..self.out_channels {
                    writer
                        .write_sample(output.channel(c)[f])
                        .context("error writing WAV data")?;
                }
                report.frames_out += 1;
            }
            done += n as u64;
        }

        writer.finalize().context("error finalising output WAV")?;
        report.wall = started.elapsed();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspd_engine::Gain;

    fn write_wav(path: &Path, channels: u16, frames: usize, value: i16) {
        let mut w = WavWriter::create(
            path,
            WavSpec {
                channels,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..frames * channels as usize {
            w.write_sample(value).unwrap();
        }
        w.finalize().unwrap();
    }

    fn read_all(path: &Path) -> (WavSpec, Vec<f32>) {
        let mut r = WavReader::open(path).unwrap();
        let spec = r.spec();
        (spec, r.samples::<f32>().map(Result::unwrap).collect())
    }

    #[test]
    fn renders_gain_and_keeps_length() {
        let dir = tempfile::tempdir().unwrap();
        let (i, o) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
        write_wav(&i, 2, 1000, 16384); // 0.5 full scale

        let mut backend = OfflineBackend::new(&i, &o, 2, 128);
        let report = backend
            .run(
                Box::new(Gain::new(-6.0206)),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(report.frames_in, 1000);
        assert_eq!(report.frames_out, 1000);

        let (spec, data) = read_all(&o);
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (2, 44_100, 32)
        );
        assert_eq!(data.len(), 2000);
        assert!(
            data.iter().all(|s| (s - 0.25).abs() < 1e-3),
            "{:?}",
            &data[..4]
        );
    }

    #[test]
    fn tail_extends_output_with_silence() {
        let dir = tempfile::tempdir().unwrap();
        let (i, o) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
        write_wav(&i, 1, 100, 8192);
        let mut backend = OfflineBackend::new(&i, &o, 1, 64).tail_frames(50);
        let report = backend
            .run(Box::new(Gain::new(0.0)), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(report.frames_out, 150);
        let (_, data) = read_all(&o);
        assert!(data[..100].iter().all(|s| *s > 0.2));
        assert!(data[100..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn mono_input_is_adapted_to_stereo_output() {
        let dir = tempfile::tempdir().unwrap();
        let (i, o) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
        write_wav(&i, 1, 10, 16384);
        let mut backend = OfflineBackend::new(&i, &o, 2, 8);
        backend
            .run(Box::new(Gain::new(0.0)), Arc::new(AtomicBool::new(false)))
            .unwrap();
        let (spec, data) = read_all(&o);
        assert_eq!(spec.channels, 2);
        assert_eq!(data.len(), 20);
    }

    struct Delay(u32);
    impl Processor for Delay {
        fn name(&self) -> &str {
            "delay"
        }
        fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
            output.copy_from(input)
        }
        fn latency_frames(&self) -> u32 {
            self.0
        }
    }

    #[test]
    fn latency_is_trimmed_so_length_matches_input() {
        let dir = tempfile::tempdir().unwrap();
        let (i, o) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
        write_wav(&i, 1, 300, 1000);
        let mut backend = OfflineBackend::new(&i, &o, 1, 64);
        let report = backend
            .run(Box::new(Delay(37)), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(report.frames_out, 300);
    }
}
