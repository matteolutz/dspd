//! Audio backends for dspd.
//!
//! The engine never talks to a device. Instead a backend owns the audio
//! thread (or loop) and drives a `Processor`:
//!
//! | backend            | use                                                   |
//! |--------------------|-------------------------------------------------------|
//! | [`OfflineBackend`] | WAV in -> engine -> WAV out; deterministic, CI-friendly |
//! | [`NullBackend`]    | generated signal, no I/O; benchmarks and soak tests   |
//! | `CpalBackend`      | live I/O via WASAPI/CoreAudio/ALSA (`cpal-backend` feature) |

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dspd_engine::Processor;

mod null;
mod offline;

#[cfg(feature = "cpal-backend")]
mod cpal_backend;

#[cfg(feature = "jack-backend")]
mod jack_backend;

#[cfg(feature = "aes67-backend")]
mod aes67_backend;

pub use null::{NullBackend, Signal};
pub use offline::{probe_wav, OfflineBackend, WavInfo};

#[cfg(feature = "cpal-backend")]
pub use cpal_backend::{list_device_configs, list_devices, list_hosts, CpalBackend};

#[cfg(feature = "jack-backend")]
pub use jack_backend::{
    JackBackend, JackBackendDaemonOptions, JackDaemon, JackDaemonConfig, JackDaemonRealtimeConfig,
    JackPatching,
};

/// What a backend run produced.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunReport {
    pub frames_in: u64,
    pub frames_out: u64,
    pub blocks: u64,
    /// Wall-clock time the run took.
    pub wall: Duration,
    /// Device-level glitches reported by the backend (live backends only).
    pub stream_errors: u64,
}

/// The current external routing for a backend's DSP inputs and outputs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PatchState {
    /// External source connected to each DSP input, by input index.
    pub inputs: Vec<Option<String>>,
    /// External destination connected to each DSP output, by output index.
    pub outputs: Vec<Option<String>>,
}

/// Optional routing capability for an audio backend.
///
/// DSP-owned ports are addressed by index. External ports are identified by
/// backend-defined names. Methods are safe to call from a control thread while
/// the audio backend is running.
pub trait AudioPatching: Send + Sync {
    /// Connect an external capture/source port to a DSP input.
    fn connect_input(&self, source: &str, input: usize) -> Result<()>;

    /// Connect a DSP output to an external playback/destination port.
    fn connect_output(&self, output: usize, destination: &str) -> Result<()>;

    /// Disconnect the current connection from a DSP input.
    fn disconnect_input(&self, input: usize) -> Result<()>;

    /// Disconnect the current connection from a DSP output.
    fn disconnect_output(&self, output: usize) -> Result<()>;

    /// Return the current external routing, indexed by DSP port.
    fn current_patch(&self) -> Result<PatchState>;

    /// Return all available external capture/source ports.
    fn capture_devices(&self) -> Result<Vec<String>>;

    /// Return all available external playback/destination ports.
    fn playback_devices(&self) -> Result<Vec<String>>;
}

/// Something that feeds audio through a processor.
pub trait AudioBackend {
    /// Return a live patching handle when this backend supports routing.
    fn patching(&self) -> Option<Arc<dyn AudioPatching>> {
        None
    }

    /// Run until the source is exhausted (offline, null with a duration) or
    /// `stop` becomes true (live backends and open-ended null runs).
    ///
    /// The processor is moved in because live backends hand it to a
    /// device callback thread.
    fn run(
        &mut self,
        processor: Box<dyn Processor + Send>,
        stop: Arc<AtomicBool>,
    ) -> Result<RunReport>;
}
