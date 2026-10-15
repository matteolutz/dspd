use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::{AudioBlock, ParamReceiver, Processor, Rack};

/// Counters shared between the audio thread and observers (control API, CLI).
/// All updates are relaxed atomics, so they are safe in the audio callback.
#[derive(Default)]
pub struct EngineStats {
    blocks: AtomicU64,
    frames: AtomicU64,
    total_ns: AtomicU64,
    max_block_ns: AtomicU64,
    overruns: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatsSnapshot {
    pub blocks: u64,
    pub frames: u64,
    /// Blocks that took longer to process than the audio they represented.
    pub overruns: u64,
    pub max_block_us: f64,
    /// Average DSP load in percent (processing time / audio time).
    pub avg_load_percent: f64,
}

impl EngineStats {
    pub fn snapshot(&self, sample_rate: u32) -> StatsSnapshot {
        let frames = self.frames.load(Ordering::Relaxed);
        let total_ns = self.total_ns.load(Ordering::Relaxed);
        let audio_ns = frames as f64 / sample_rate.max(1) as f64 * 1e9;
        StatsSnapshot {
            blocks: self.blocks.load(Ordering::Relaxed),
            frames,
            overruns: self.overruns.load(Ordering::Relaxed),
            max_block_us: self.max_block_ns.load(Ordering::Relaxed) as f64 / 1e3,
            avg_load_percent: if audio_ns > 0.0 {
                total_ns as f64 / audio_ns * 100.0
            } else {
                0.0
            },
        }
    }
}

/// Audio-thread entry point: drains pending parameter changes, runs the rack
/// and records timing statistics.
pub struct Engine {
    rack: Rack,
    params: ParamReceiver,
    stats: Arc<EngineStats>,
    sample_rate: u32,
}

impl Engine {
    pub fn new(rack: Rack, params: ParamReceiver, sample_rate: u32) -> Self {
        Self {
            rack,
            params,
            stats: Arc::new(EngineStats::default()),
            sample_rate,
        }
    }

    pub fn stats(&self) -> Arc<EngineStats> {
        Arc::clone(&self.stats)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn rack(&self) -> &Rack {
        &self.rack
    }

    /// Shared bank of live meter values (see [`crate::MeterBank`]).
    pub fn meters(&self) -> Arc<crate::MeterBank> {
        Arc::clone(self.rack.meters())
    }

    /// What each slot of the meter bank is.
    pub fn meter_infos(&self) -> Arc<[crate::MeterInfo]> {
        Arc::clone(self.rack.meter_infos())
    }
}

impl Processor for Engine {
    fn name(&self) -> &str {
        "engine"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let start = Instant::now();

        while let Some(ev) = self.params.pop() {
            self.rack
                .set_param(ev.strip as usize, ev.stage as usize, ev.param_id, ev.value);
        }
        self.rack.process(input, output);

        let elapsed_ns = start.elapsed().as_nanos() as u64;
        let frames = input.frames() as u64;
        let deadline_ns = frames * 1_000_000_000 / self.sample_rate.max(1) as u64;

        self.stats.blocks.fetch_add(1, Ordering::Relaxed);
        self.stats.frames.fetch_add(frames, Ordering::Relaxed);
        self.stats.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        self.stats
            .max_block_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
        if elapsed_ns > deadline_ns {
            self.stats.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn latency_frames(&self) -> u32 {
        self.rack.latency_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{param_channel, Chain, Gain, ParamEvent, Stage, Strip};

    #[test]
    fn param_events_are_applied_before_processing() {
        let (mut tx, rx) = param_channel(8);
        let chain = Chain::new(
            1,
            64,
            vec![Stage {
                label: "g".into(),
                processor: Box::new(Gain::new(0.0)),
            }],
        );
        let rack = Rack::new(vec![Strip::new("s", 0, 0, chain)]);
        let mut engine = Engine::new(rack, rx, 48_000);
        tx.send(ParamEvent {
            strip: 0,
            stage: 0,
            param_id: 0,
            value: -120.0,
        })
        .unwrap();

        let mut input = AudioBlock::new(1, 64);
        input.channel_mut(0).fill(1.0);
        let mut output = AudioBlock::new(1, 64);
        // First block ramps towards silence, second block is fully attenuated.
        engine.process(&input, &mut output);
        engine.process(&input, &mut output);
        assert!(output.channel(0).iter().all(|s| s.abs() < 1e-3));

        let snap = engine.stats().snapshot(48_000);
        assert_eq!(snap.blocks, 2);
        assert_eq!(snap.frames, 128);
    }
}
