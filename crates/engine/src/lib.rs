//! Platform-independent core of dspd: audio blocks, the `Processor` trait,
//! processing chains, the parameter queue and the serializable session spec.
//!
//! Nothing in this crate touches an audio device, a plugin format or the OS.

mod block;
mod chain;
mod engine;
mod gain;
mod params;
mod processor;
pub mod session;

pub use block::AudioBlock;
pub use chain::{Chain, Stage};
pub use engine::{Engine, EngineStats, StatsSnapshot};
pub use gain::{Gain, GAIN_PARAM_DB};
pub use params::{param_channel, ParamEvent, ParamReceiver, ParamSender};
pub use processor::{BoxedProcessor, Processor};
pub use session::{ClapSpec, GainSpec, SessionSpec, StageSpec};
