//! A rack of independent strips.
//!
//! Each strip is one channel (mono) or one channel pair (stereo) with its own
//! plugin chain. A strip reads `width` consecutive input channels starting at
//! `input` and writes `width` consecutive output channels starting at
//! `output`. Output channels that no strip writes are silent.
//!
//! Strips are processed one after another on the calling thread. They share no
//! state, so a later scheduler can run them on several cores without changes
//! to this data model.

use std::sync::Arc;

use crate::{AudioBlock, Chain, MeterBank, MeterInfo, MeterWriter, Processor};

/// Fixed delay used to line up strips whose chains report different latencies.
struct DelayLine {
    buf: Vec<Vec<f32>>,
    pos: usize,
    len: usize,
}

impl DelayLine {
    fn new(channels: usize, frames: usize) -> Self {
        Self {
            buf: vec![vec![0.0; frames]; channels],
            pos: 0,
            len: frames,
        }
    }

    fn process(&mut self, block: &mut AudioBlock) {
        if self.len == 0 {
            return;
        }
        let n = block.frames();
        for (c, line) in self.buf.iter_mut().enumerate() {
            let mut p = self.pos;
            for s in block.channel_mut(c) {
                std::mem::swap(s, &mut line[p]);
                p += 1;
                if p == self.len {
                    p = 0;
                }
            }
        }
        self.pos = (self.pos + n) % self.len;
    }
}

/// One mono or stereo channel strip.
pub struct Strip {
    name: String,
    input: usize,
    output: usize,
    chain: Chain,
    in_block: AudioBlock,
    out_block: AudioBlock,
    compensation: DelayLine,
}

impl Strip {
    /// The strip's width is the chain's channel count (1 = mono, 2 = stereo).
    pub fn new(name: impl Into<String>, input: usize, output: usize, chain: Chain) -> Self {
        let width = chain.channels();
        let max = chain.max_frames();
        Self {
            name: name.into(),
            input,
            output,
            in_block: AudioBlock::new(width, max),
            out_block: AudioBlock::new(width, max),
            compensation: DelayLine::new(width, 0),
            chain,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn width(&self) -> usize {
        self.chain.channels()
    }

    pub fn input(&self) -> usize {
        self.input
    }

    pub fn output(&self) -> usize {
        self.output
    }

    pub fn chain(&self) -> &Chain {
        &self.chain
    }

    /// Latency of the strip's own chain, before alignment with other strips.
    pub fn chain_latency_frames(&self) -> u32 {
        self.chain.total_latency_frames()
    }

    /// Extra delay added so this strip lines up with the slowest one.
    pub fn compensation_frames(&self) -> usize {
        self.compensation.len
    }
}

pub struct Rack {
    strips: Vec<Strip>,
    latency: u32,

    meters: Arc<MeterBank>,
    meter_infos: Arc<[MeterInfo]>,
}

impl Rack {
    /// Build a rack and align all strips to the one with the highest latency,
    /// so every output channel is delayed by the same amount.
    pub fn new(mut strips: Vec<Strip>) -> Self {
        let latency = strips
            .iter()
            .map(Strip::chain_latency_frames)
            .max()
            .unwrap_or(0);
        for s in &mut strips {
            let extra = (latency - s.chain_latency_frames()) as usize;
            s.compensation = DelayLine::new(s.width(), extra);
        }

        // Ask every stage which meters it has, allocate one bank for all of them,
        // and hand each stage the writer for its own window.
        let mut infos = Vec::new();
        let mut windows = Vec::new(); // (strip, stage, first slot, count)
        for (si, strip) in strips.iter().enumerate() {
            for (ti, stage) in strip.chain.stages().iter().enumerate() {
                let specs = stage.processor.meter_specs();
                if specs.is_empty() {
                    continue;
                }
                windows.push((si, ti, infos.len(), specs.len()));
                for spec in specs {
                    infos.push(MeterInfo {
                        slot: infos.len(),
                        strip: si,
                        stage: ti,
                        param_id: spec.param_id,
                        name: spec.name,
                    });
                }
            }
        }
        let meters = MeterBank::new(infos.len());
        for (si, ti, base, len) in windows {
            let writer = MeterWriter::new(Arc::clone(&meters), base, len);
            strips[si].chain.stages_mut()[ti]
                .processor
                .attach_meters(writer);
        }

        Self {
            strips,
            latency,
            meters,
            meter_infos: infos.into(),
        }
    }

    /// The bank all meters of this rack write into. Share it with the control side.
    pub fn meters(&self) -> &Arc<MeterBank> {
        &self.meters
    }

    /// What each slot of [`Rack::meters`] is: strip, stage, name.
    pub fn meter_infos(&self) -> &Arc<[MeterInfo]> {
        &self.meter_infos
    }

    pub fn strips(&self) -> &[Strip] {
        &self.strips
    }

    /// Route a parameter change to one stage of one strip. Out-of-range
    /// addresses are ignored.
    pub fn set_param(&mut self, strip: usize, stage: usize, id: u32, value: f64) {
        if let Some(s) = self.strips.get_mut(strip) {
            s.chain.set_param(stage, id, value);
        }
    }
}

impl Processor for Rack {
    fn name(&self) -> &str {
        "rack"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let n = input.frames();
        output.set_frames(n);
        output.clear();

        for s in &mut self.strips {
            s.in_block.set_frames(n);
            for k in 0..s.in_block.num_channels() {
                let dst = s.in_block.channel_mut(k);
                let src = s.input + k;
                if src < input.num_channels() {
                    dst.copy_from_slice(input.channel(src));
                } else {
                    dst.fill(0.0); // the device has fewer inputs than the session asks for
                }
            }

            s.out_block.set_frames(n);
            s.chain.process(&s.in_block, &mut s.out_block);
            s.compensation.process(&mut s.out_block);

            for k in 0..s.out_block.num_channels() {
                let dst = s.output + k;
                if dst < output.num_channels() {
                    output
                        .channel_mut(dst)
                        .copy_from_slice(s.out_block.channel(k));
                }
            }
        }
    }

    /// Latency of the whole rack: the slowest strip. All strips are aligned to it.
    fn latency_frames(&self) -> u32 {
        self.latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Stage;
    use dspd_plugin_gain::Gain;

    fn strip(name: &str, width: usize, input: usize, output: usize, db: f32) -> Strip {
        let stage = Stage {
            label: "g".into(),
            processor: Box::new(Gain::new(db)),
        };
        Strip::new(name, input, output, Chain::new(width, 64, vec![stage]))
    }

    /// Delays its input by `n` frames and reports that latency.
    struct Latent {
        n: u32,
        ring: Vec<Vec<f32>>,
        pos: usize,
    }

    impl Latent {
        fn new(n: u32, channels: usize) -> Self {
            Self {
                n,
                ring: vec![vec![0.0; n.max(1) as usize]; channels],
                pos: 0,
            }
        }
    }

    impl Processor for Latent {
        fn name(&self) -> &str {
            "latent"
        }
        fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
            output.copy_from(input);
            let len = self.ring[0].len();
            for (c, line) in self.ring.iter_mut().enumerate() {
                let mut p = self.pos;
                for s in output.channel_mut(c) {
                    if self.n > 0 {
                        std::mem::swap(s, &mut line[p]);
                        p = (p + 1) % len;
                    }
                }
            }
            self.pos = (self.pos + input.frames()) % len;
        }
        fn latency_frames(&self) -> u32 {
            self.n
        }
    }

    #[test]
    fn thirty_two_mono_strips_are_independent() {
        let strips = (0..32)
            .map(|i| strip(&format!("s{i}"), 1, i, i, -(i as f32)))
            .collect();
        let mut rack = Rack::new(strips);

        let mut input = AudioBlock::new(32, 16);
        for c in 0..32 {
            input.channel_mut(c).fill(1.0);
        }
        let mut output = AudioBlock::new(32, 16);
        rack.process(&input, &mut output);

        for c in 0..32 {
            let want = 10f32.powf(-(c as f32) / 20.0);
            assert!(
                output.channel(c).iter().all(|s| (s - want).abs() < 1e-4),
                "channel {c}"
            );
        }
    }

    #[test]
    fn mono_and_stereo_strips_share_a_rack_and_unassigned_outputs_are_silent() {
        // ch0: mono, ch1+2: stereo pair, ch3: not used by any strip
        let mut rack = Rack::new(vec![
            strip("mono", 1, 0, 0, -6.0206),
            strip("pair", 2, 1, 1, -6.0206),
        ]);
        let mut input = AudioBlock::new(4, 8);
        for (c, v) in [0.1f32, 0.2, 0.3, 0.4].iter().enumerate() {
            input.channel_mut(c).fill(*v);
        }
        let mut output = AudioBlock::new(4, 8);
        rack.process(&input, &mut output);

        let expect = [0.05, 0.1, 0.15, 0.0];
        for (c, want) in expect.iter().enumerate() {
            assert!(
                output.channel(c).iter().all(|s| (s - want).abs() < 1e-3),
                "channel {c}"
            );
        }
    }

    #[test]
    fn strip_can_route_to_a_different_output_channel() {
        let mut rack = Rack::new(vec![strip("moved", 1, 2, 5, 0.0)]);
        let mut input = AudioBlock::new(4, 4);
        input.channel_mut(2).fill(0.7);
        let mut output = AudioBlock::new(8, 4);
        rack.process(&input, &mut output);
        assert!(output.channel(5).iter().all(|s| (s - 0.7).abs() < 1e-6));
        assert!(output.channel(2).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn missing_input_channels_read_as_silence() {
        let mut rack = Rack::new(vec![strip("far", 1, 5, 0, 0.0)]);
        let input = AudioBlock::new(2, 4); // only 2 input channels exist
        let mut output = AudioBlock::new(1, 4);
        output.channel_mut(0).fill(9.0);
        rack.process(&input, &mut output);
        assert!(output.channel(0).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn strips_with_different_latency_are_aligned() {
        let slow = Strip::new(
            "slow",
            0,
            0,
            Chain::new(
                1,
                64,
                vec![Stage {
                    label: "l".into(),
                    processor: Box::new(Latent::new(10, 1)),
                }],
            ),
        );
        let fast = strip("fast", 1, 1, 1, 0.0);
        let mut rack = Rack::new(vec![slow, fast]);
        assert_eq!(rack.latency_frames(), 10);
        assert_eq!(rack.strips()[0].compensation_frames(), 0);
        assert_eq!(rack.strips()[1].compensation_frames(), 10);

        // An impulse on both inputs, processed over several uneven blocks.
        let mut seen = [Vec::new(), Vec::new()];
        for (i, len) in [8usize, 5, 11].into_iter().enumerate() {
            let mut input = AudioBlock::new(2, 16);
            input.set_frames(len);
            if i == 0 {
                input.channel_mut(0)[0] = 1.0;
                input.channel_mut(1)[0] = 1.0;
            }
            let mut output = AudioBlock::new(2, 16);
            rack.process(&input, &mut output);
            for (c, heard) in seen.iter_mut().enumerate() {
                heard.extend_from_slice(output.channel(c));
            }
        }
        for (c, heard) in seen.iter().enumerate() {
            let hit = heard.iter().position(|s| *s == 1.0);
            assert_eq!(hit, Some(10), "channel {c}: {heard:?}");
        }
    }

    #[test]
    fn meters_are_collected_per_strip_and_stage() {
        use dspd_plugin_peak_meter::PeakMeter;
        let metered = |name: &str, width: usize, input: usize, db: f32| {
            let stages = vec![
                Stage {
                    label: "g".into(),
                    processor: Box::new(Gain::new(db)),
                },
                Stage {
                    label: "m".into(),
                    processor: Box::new(PeakMeter::new(48_000, width)),
                },
            ];
            Strip::new(name, input, input, Chain::new(width, 64, stages))
        };
        let plain = strip("plain", 1, 1, 1, 0.0); // no meter: contributes no slots
        let mut rack = Rack::new(vec![
            metered("a", 1, 0, -6.0206),
            plain,
            metered("b", 1, 2, 0.0),
        ]);

        let infos = rack.meter_infos().to_vec();
        assert_eq!(infos.len(), 2);
        assert_eq!(
            (
                infos[0].slot,
                infos[0].strip,
                infos[0].stage,
                infos[0].name.as_str()
            ),
            (0, 0, 1, "Peak")
        );
        assert_eq!((infos[1].slot, infos[1].strip, infos[1].stage), (1, 2, 1));

        let mut input = AudioBlock::new(3, 16);
        for c in 0..3 {
            input.channel_mut(c).fill(1.0);
        }
        let mut output = AudioBlock::new(3, 16);
        rack.process(&input, &mut output);

        let bank = rack.meters();
        assert!(
            (bank.get(0) - (-6.02)).abs() < 0.05,
            "strip a: {}",
            bank.get(0)
        );
        assert!(bank.get(1).abs() < 0.05, "strip b: {}", bank.get(1));
    }

    #[test]
    fn param_is_routed_to_the_addressed_strip_only() {
        let mut rack = Rack::new(vec![strip("a", 1, 0, 0, 0.0), strip("b", 1, 1, 1, 0.0)]);
        rack.set_param(1, 0, dspd_plugin_gain::GAIN_PARAM_DB, -120.0);
        rack.set_param(9, 0, dspd_plugin_gain::GAIN_PARAM_DB, -120.0); // out of range: ignored

        let mut input = AudioBlock::new(2, 8);
        input.channel_mut(0).fill(1.0);
        input.channel_mut(1).fill(1.0);
        let mut output = AudioBlock::new(2, 8);
        rack.process(&input, &mut output); // ramp block
        rack.process(&input, &mut output);
        assert!(output.channel(0).iter().all(|s| (s - 1.0).abs() < 1e-6));
        assert!(output.channel(1).iter().all(|s| s.abs() < 1e-3));
    }
}
