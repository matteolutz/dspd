use crate::{block::AudioBlock, MeterWriter};

/// One value a processor publishes for display (a meter, gain reduction, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeterSpec {
    /// Stable id of the value inside this processor (the CLAP parameter id for CLAP plugins).
    pub param_id: u32,
    pub name: String,
}

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

    /// The values this processor publishes, in a fixed order. Called once while
    /// the rack is built (so it may allocate); the order defines the indices
    /// of the `MeterWriter` passed to `attach_meters`.
    fn meter_specs(&self) -> Vec<MeterSpec> {
        Vec::new()
    }

    /// Receive the writer for this processor's meters: `writer.set(i, v)` publishes
    /// the value declared at `meter_specs()[i]`. Called once, after `meter_specs`.
    /// Keep the writer and call `set` from `process`; it is real-time safe.
    fn attach_meters(&mut self, _writer: MeterWriter) {}
}

/// A processor that can be moved to the audio thread.
pub type BoxedProcessor = Box<dyn Processor + Send>;
