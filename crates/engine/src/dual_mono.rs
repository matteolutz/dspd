use crate::{AudioBlock, BoxedProcessor, MeterSpec, MeterWriter, Processor};

/// Runs two instances of a mono processor as one stereo processor.
///
/// This is how a mono-only plugin is used on a stereo strip: the left channel
/// goes through one instance and the right channel through the other. Both get
/// the same parameter values (linked), and the latency is the larger of the two.
pub struct DualMono {
    left: BoxedProcessor,
    right: BoxedProcessor,
    in_l: AudioBlock,
    in_r: AudioBlock,
    out_l: AudioBlock,
    out_r: AudioBlock,
    max_frames: usize,
}

impl DualMono {
    pub fn new(left: BoxedProcessor, right: BoxedProcessor, max_frames: usize) -> Self {
        Self {
            left,
            right,
            in_l: AudioBlock::new(1, max_frames),
            in_r: AudioBlock::new(1, max_frames),
            out_l: AudioBlock::new(1, max_frames),
            out_r: AudioBlock::new(1, max_frames),
            max_frames,
        }
    }
}

impl Processor for DualMono {
    fn name(&self) -> &str {
        self.left.name()
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let n = input.frames();
        debug_assert!(n <= self.max_frames);
        // A mono input block is fed to both sides, like a stereo one with L == R.
        let right_src = if input.num_channels() > 1 { 1 } else { 0 };

        for b in [
            &mut self.in_l,
            &mut self.in_r,
            &mut self.out_l,
            &mut self.out_r,
        ] {
            b.set_frames(n);
        }
        self.in_l.channel_mut(0).copy_from_slice(input.channel(0));
        self.in_r
            .channel_mut(0)
            .copy_from_slice(input.channel(right_src));

        self.left.process(&self.in_l, &mut self.out_l);
        self.right.process(&self.in_r, &mut self.out_r);

        output.set_frames(n);
        output.channel_mut(0).copy_from_slice(self.out_l.channel(0));
        if output.num_channels() > 1 {
            output.channel_mut(1).copy_from_slice(self.out_r.channel(0));
        }
    }

    fn latency_frames(&self) -> u32 {
        self.left.latency_frames().max(self.right.latency_frames())
    }

    fn set_param(&mut self, id: u32, value: f64) {
        self.left.set_param(id, value);
        self.right.set_param(id, value);
    }

    /// Both instances report their own values: all of the left side's meters
    /// (named "... L"), then all of the right side's ("... R").
    fn meter_specs(&self) -> Vec<MeterSpec> {
        let side = |specs: Vec<MeterSpec>, tag: &str| -> Vec<MeterSpec> {
            specs
                .into_iter()
                .map(|s| MeterSpec {
                    param_id: s.param_id,
                    name: format!("{} {tag}", s.name),
                })
                .collect()
        };
        let mut all = side(self.left.meter_specs(), "L");
        all.extend(side(self.right.meter_specs(), "R"));
        all
    }

    fn attach_meters(&mut self, writer: MeterWriter) {
        let left = self.left.meter_specs().len();
        let right = self.right.meter_specs().len();
        self.left.attach_meters(writer.slice(0, left));
        self.right.attach_meters(writer.slice(left, right));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Gain;

    #[test]
    fn left_and_right_stay_independent() {
        let mut dm = DualMono::new(
            Box::new(Gain::new(-6.0206)),
            Box::new(Gain::new(-6.0206)),
            16,
        );
        let mut input = AudioBlock::new(2, 16);
        input.channel_mut(0).fill(1.0);
        input.channel_mut(1).fill(0.25);
        let mut output = AudioBlock::new(2, 16);
        dm.process(&input, &mut output);
        assert!(output.channel(0).iter().all(|s| (s - 0.5).abs() < 1e-3));
        assert!(output.channel(1).iter().all(|s| (s - 0.125).abs() < 1e-3));
    }

    #[test]
    fn params_reach_both_instances() {
        let mut dm = DualMono::new(Box::new(Gain::new(0.0)), Box::new(Gain::new(0.0)), 8);
        dm.set_param(crate::GAIN_PARAM_DB, -120.0);
        let mut input = AudioBlock::new(2, 8);
        input.channel_mut(0).fill(1.0);
        input.channel_mut(1).fill(1.0);
        let mut output = AudioBlock::new(2, 8);
        dm.process(&input, &mut output); // ramp
        dm.process(&input, &mut output);
        assert!(output
            .channel(0)
            .iter()
            .chain(output.channel(1))
            .all(|s| s.abs() < 1e-3));
    }

    #[test]
    fn mono_input_feeds_both_sides() {
        let mut dm = DualMono::new(Box::new(Gain::new(0.0)), Box::new(Gain::new(0.0)), 4);
        let mut input = AudioBlock::new(1, 4);
        input.channel_mut(0).fill(0.3);
        let mut output = AudioBlock::new(2, 4);
        dm.process(&input, &mut output);
        assert_eq!(output.channel(0), output.channel(1));
    }
}
