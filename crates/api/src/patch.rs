use std::{any::Any, borrow::Cow, sync::Arc};

pub trait AudioDevice: Send + Sync {
    fn display_name(&self) -> Cow<'_, str>;
    fn num_channels(&self) -> usize;

    fn as_any(&self) -> &dyn Any;
}

pub struct PatchEntry {
    device: Arc<dyn AudioDevice>,
    channel_idx: usize,
}

pub struct PatchState {
    pub inputs: Vec<Option<PatchEntry>>,
    pub outputs: Vec<Option<PatchEntry>>,
}

pub trait AudioPatching: Send + Sync {
    /// Connect an external capture/source port to a DSP input.
    fn connect_input(
        &self,
        input_device: &dyn AudioDevice,
        input_device_channel_idx: usize,
        dspd_input_idx: usize,
    ) -> anyhow::Result<()>;

    /// Connect a DSP output to an external playback/destination port.
    fn connect_output(
        &self,
        dspd_output_idx: usize,
        output_device: &dyn AudioDevice,
        output_device_channel_idx: usize,
    ) -> anyhow::Result<()>;

    /// Disconnect the current connection from a DSP input.
    fn disconnect_input(&self, dspd_input_idx: usize) -> anyhow::Result<()>;

    /// Disconnect the current connection from a DSP output.
    fn disconnect_output(&self, dspd_output_idx: usize) -> anyhow::Result<()>;

    /// Return the current external routing, indexed by DSP port.
    fn current_patch(&self) -> anyhow::Result<PatchState>;

    /// Return all available external capture/source ports.
    fn capture_devices(&self) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>>;

    /// Return all available external playback/destination ports.
    fn playback_devices(&self) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>>;
}
