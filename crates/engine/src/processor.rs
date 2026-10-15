use crate::block::AudioBlock;

/// One audio-processing unit (a plugin, a built-in effect, a whole chain).
///
/// `process` runs on the audio thread and must not allocate, lock, log or do
/// any I/O. `input` and `output` always have the same channel count and
/// capacity, and `input.frames()` is the number of frames to process.
pub trait Processor {
    fn name(&self) -> &str;

    /// Read `input`, write `output`. Implementations should leave
    /// `output.frames() == input.frames()`.
    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock);

    /// Latency introduced by this processor, in frames.
    fn latency_frames(&self) -> u32 {
        0
    }

    /// Set a parameter. Called on the audio thread just before `process`,
    /// so it must be real-time safe. Unknown ids are ignored.
    fn set_param(&mut self, _id: u32, _value: f64) {}
}

/// A processor that can be moved to the audio thread.
pub type BoxedProcessor = Box<dyn Processor + Send>;
