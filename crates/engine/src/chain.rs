use crate::{AudioBlock, BoxedProcessor, Processor};

/// One processor in a chain, with a human-readable label.
pub struct Stage {
    pub label: String,
    pub processor: BoxedProcessor,
}

/// Most stages a chain can hold. Room for this many is reserved up front, so
/// inserting a stage on the audio thread never allocates.
pub const MAX_STAGES: usize = 64;

/// Processors run in series on a fixed channel count.
///
/// Two scratch blocks are ping-ponged between stages, so `process` never
/// allocates. The block passed in must not exceed `max_frames`.
pub struct Chain {
    stages: Vec<Stage>,
    stage_order: Vec<u32>,

    a: AudioBlock,
    b: AudioBlock,
    channels: usize,
}

impl Chain {
    pub fn new(channels: usize, max_frames: usize, mut stages: Vec<Stage>) -> Self {
        let n = stages.len();
        stages.reserve(MAX_STAGES.saturating_sub(n));
        let mut stage_order = Vec::with_capacity(MAX_STAGES.max(n));
        stage_order.extend(0..n as u32);

        Self {
            stages,
            stage_order,
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

    pub fn stage_order(&self) -> &[u32] {
        &self.stage_order
    }

    /// Where the stage with the given ID is in the chain.
    /// Returns `None` if the stage is not in the chain.
    pub fn position_of(&self, stage_id: u32) -> Option<usize> {
        self.stage_order.iter().position(|&id| id == stage_id)
    }

    /// Sum of the latencies of all stages.
    pub fn total_latency_frames(&self) -> u32 {
        self.stages
            .iter()
            .map(|s| s.processor.latency_frames())
            .sum()
    }

    /// Route a parameter change to the stage with id `stage`. Unknown ids are ignored.
    pub fn set_param(&mut self, stage_id: u32, id: u32, value: f64) {
        if let Some(pos) = self.position_of(stage_id) {
            self.stages[pos].processor.set_param(id, value);
        }
    }

    /// Insert `stage` at `index` (clamped to the end) under a new id.
    /// Never allocates: if the reserved room is used up, or the id is already
    /// taken, the stage is handed back.
    pub fn insert(&mut self, index: usize, id: u32, stage: Stage) -> Result<(), Stage> {
        let full = self.stages.len() >= self.stages.capacity()
            || self.stage_order.len() >= self.stage_order.capacity();
        if full || self.position_of(id).is_some() {
            return Err(stage);
        }

        let index = index.min(self.stages.len());

        self.stages.insert(index, stage);
        self.stage_order.insert(index, id);

        Ok(())
    }

    /// Take a stage out of the chain. The caller must dispose of it off the audio thread.
    pub fn remove(&mut self, id: u32) -> Option<Stage> {
        let pos = self.position_of(id)?;
        self.stage_order.remove(pos);
        Some(self.stages.remove(pos))
    }

    /// Move a stage so it ends up at position `to` (clamped). Returns false for an unknown id.
    pub fn move_to(&mut self, id: u32, to_pos: usize) -> bool {
        let Some(from_pos) = self.position_of(id) else {
            return false;
        };

        let to_pos = to_pos.min(self.stages.len() - 1);
        if from_pos < to_pos {
            self.stages[from_pos..=to_pos].rotate_left(1);
            self.stage_order[from_pos..=to_pos].rotate_left(1);
        } else if to_pos < from_pos {
            self.stages[to_pos..=from_pos].rotate_right(1);
            self.stage_order[to_pos..=from_pos].rotate_right(1);
        }

        true
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
