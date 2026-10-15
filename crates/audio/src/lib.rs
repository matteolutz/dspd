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

pub use null::{NullBackend, Signal};
pub use offline::{probe_wav, OfflineBackend, WavInfo};

#[cfg(feature = "cpal-backend")]
pub use cpal_backend::{list_devices, CpalBackend};

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

/// Something that feeds audio through a processor.
pub trait AudioBackend {
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
