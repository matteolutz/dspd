//! A backend with no device: generates a test signal and discards the output.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use dspd_engine::{AudioBlock, Processor};

use crate::{AudioBackend, RunReport};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    Silence,
    /// Sine at the given frequency in Hz, -6 dBFS.
    Sine(f32),
    /// White noise, -12 dBFS. Exercises plugins with real signal at every sample.
    Noise,
}

pub struct NullBackend {
    in_channels: usize,
    out_channels: usize,
    block_size: usize,
    sample_rate: u32,
    signal: Signal,
    duration: Option<Duration>,
    realtime: bool,
}

impl NullBackend {
    /// `in_channels` of generated signal go in; `out_channels` come back out and are discarded.
    pub fn new(in_channels: usize, out_channels: usize, block_size: usize, sample_rate: u32) -> Self {
        Self {
            in_channels,
            out_channels,
            block_size,
            sample_rate,
            signal: Signal::Sine(440.0),
            duration: None,
            realtime: false,
        }
    }

    pub fn signal(mut self, signal: Signal) -> Self {
        self.signal = signal;
        self
    }

    /// Stop after this much audio. Without it the run lasts until `stop` is set.
    pub fn duration(mut self, d: Duration) -> Self {
        self.duration = Some(d);
        self
    }

    /// Pace blocks against the wall clock like a real device would. Off by
    /// default, so a benchmark runs as fast as the CPU allows.
    pub fn realtime(mut self, on: bool) -> Self {
        self.realtime = on;
        self
    }
}

impl AudioBackend for NullBackend {
    fn run(&mut self, mut processor: Box<dyn Processor + Send>, stop: Arc<AtomicBool>) -> Result<RunReport> {
        let started = Instant::now();
        let limit = self
            .duration
            .map(|d| (d.as_secs_f64() * f64::from(self.sample_rate)) as u64);

        let mut input = AudioBlock::new(self.in_channels, self.block_size);
        let mut output = AudioBlock::new(self.out_channels, self.block_size);
        let mut phase = 0.0f32;
        let phase_step = match self.signal {
            Signal::Sine(f) => std::f32::consts::TAU * f / self.sample_rate as f32,
            _ => 0.0,
        };
        let mut rng = 0x9E37_79B9u32;

        let mut report = RunReport::default();
        let mut frames_done = 0u64;
        while !stop.load(Ordering::Relaxed) && limit.map_or(true, |l| frames_done < l) {
            let n = limit.map_or(self.block_size as u64, |l| (l - frames_done).min(self.block_size as u64)) as usize;
            input.set_frames(n);
            output.set_frames(n);

            for f in 0..n {
                let s = match self.signal {
                    Signal::Silence => 0.0,
                    Signal::Sine(_) => {
                        phase += phase_step;
                        if phase > std::f32::consts::TAU {
                            phase -= std::f32::consts::TAU;
                        }
                        0.5 * phase.sin()
                    }
                    Signal::Noise => {
                        // xorshift32
                        rng ^= rng << 13;
                        rng ^= rng >> 17;
                        rng ^= rng << 5;
                        0.25 * ((rng as f32 / u32::MAX as f32) * 2.0 - 1.0)
                    }
                };
                for c in 0..self.in_channels {
                    input.channel_mut(c)[f] = s;
                }
            }

            processor.process(&input, &mut output);
            frames_done += n as u64;
            report.blocks += 1;

            if self.realtime {
                let due = started + Duration::from_secs_f64(frames_done as f64 / f64::from(self.sample_rate));
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }

        report.frames_in = frames_done;
        report.frames_out = frames_done;
        report.wall = started.elapsed();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspd_engine::Gain;

    #[test]
    fn stops_after_duration_with_partial_last_block() {
        let mut b = NullBackend::new(2, 2, 100, 1000).duration(Duration::from_millis(1050));
        let r = b.run(Box::new(Gain::new(0.0)), Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(r.frames_in, 1050);
        assert_eq!(r.blocks, 11);
    }

    #[test]
    fn stop_flag_ends_open_ended_run() {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&stop);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            s2.store(true, Ordering::Relaxed);
        });
        let mut b = NullBackend::new(1, 1, 64, 48_000).realtime(true);
        let r = b.run(Box::new(Gain::new(0.0)), stop).unwrap();
        t.join().unwrap();
        assert!(r.blocks > 0);
        assert!(r.wall < Duration::from_secs(2));
    }
}
