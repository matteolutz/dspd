use crate::{AudioBlock, BoxedProcessor, Processor};

/// One processor in a chain, with a human-readable label.
pub struct Stage {
    pub label: String,
    pub processor: BoxedProcessor,
}

/// Processors run in series on a fixed channel count.
///
/// Two scratch blocks are ping-ponged between stages, so `process` never
/// allocates. The block passed in must not exceed `max_frames`.
pub struct Chain {
    stages: Vec<Stage>,
    a: AudioBlock,
    b: AudioBlock,
    channels: usize,
}

impl Chain {
    pub fn new(channels: usize, max_frames: usize, stages: Vec<Stage>) -> Self {
        Self {
            stages,
            a: AudioBlock::new(channels, max_frames),
            b: AudioBlock::new(channels, max_frames),
            channels,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn max_frames(&self) -> usize {
        self.a.capacity()
    }

    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    pub fn stages_mut(&mut self) -> &mut [Stage] {
        &mut self.stages
    }

    /// Sum of the latencies of all stages.
    pub fn total_latency_frames(&self) -> u32 {
        self.stages
            .iter()
            .map(|s| s.processor.latency_frames())
            .sum()
    }

    /// Route a parameter change to one stage. Out-of-range stages are ignored.
    pub fn set_param(&mut self, stage: usize, id: u32, value: f64) {
        if let Some(s) = self.stages.get_mut(stage) {
            s.processor.set_param(id, value);
        }
    }
}

impl Processor for Chain {
    fn name(&self) -> &str {
        "chain"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let n = input.frames();
        self.a.copy_from(input);
        for stage in &mut self.stages {
            self.b.set_frames(n);
            stage.processor.process(&self.a, &mut self.b);
            std::mem::swap(&mut self.a, &mut self.b);
        }
        output.copy_from(&self.a);
    }

    fn latency_frames(&self) -> u32 {
        self.total_latency_frames()
    }

    fn set_param(&mut self, _id: u32, _value: f64) {
        // Chains are addressed per stage; see `Chain::set_param`.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspd_plugin_gain::Gain;

    fn stage(db: f32) -> Stage {
        Stage {
            label: format!("gain{db}"),
            processor: Box::new(Gain::new(db)),
        }
    }

    #[test]
    fn stages_apply_in_series() {
        let mut chain = Chain::new(2, 32, vec![stage(-6.0206), stage(-6.0206)]);
        let mut input = AudioBlock::new(2, 16);
        for c in 0..2 {
            input.channel_mut(c).fill(1.0);
        }
        let mut output = AudioBlock::new(2, 16);
        chain.process(&input, &mut output);
        for s in output.channel(0) {
            assert!((s - 0.25).abs() < 1e-3, "got {s}");
        }
    }

    #[test]
    fn empty_chain_is_passthrough() {
        let mut chain = Chain::new(2, 8, vec![]);
        let mut input = AudioBlock::new(2, 8);
        input.channel_mut(0).fill(0.3);
        input.channel_mut(1).fill(0.7);
        let mut output = AudioBlock::new(2, 8);
        chain.process(&input, &mut output);
        assert_eq!(output.channel(0), input.channel(0));
        assert_eq!(output.channel(1), input.channel(1));
    }

    #[test]
    fn partial_blocks_work() {
        let mut chain = Chain::new(1, 64, vec![stage(0.0)]);
        let mut input = AudioBlock::new(1, 64);
        input.set_frames(5);
        input.channel_mut(0).fill(1.0);
        let mut output = AudioBlock::new(1, 64);
        chain.process(&input, &mut output);
        assert_eq!(output.frames(), 5);
    }
}
