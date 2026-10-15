//! Platform-independent core of dspd: audio blocks, the `Processor` trait,
//! processing chains, the rack of mono/stereo strips, the parameter queue and
//! the serializable session spec.
//!
//! Nothing in this crate touches an audio device, a plugin format or the OS.

mod block;
mod chain;
mod dual_mono;
mod engine;
mod meters;
mod params;
mod peak_meter;
mod processor;
mod rack;
pub mod session;

mod dyneq;
mod gain;

pub use block::AudioBlock;
pub use chain::{Chain, Stage};
pub use dual_mono::DualMono;
pub use engine::{Engine, EngineStats, StatsSnapshot};
pub use gain::{Gain, GAIN_PARAM_DB};
pub use meters::{MeterBank, MeterInfo, MeterWriter};
pub use params::{param_channel, ParamEvent, ParamReceiver, ParamSender};
pub use peak_meter::{PeakMeter, PEAK_METER_ID};
pub use processor::{BoxedProcessor, MeterSpec, Processor};
pub use rack::{Rack, Strip};
pub use session::{ClapSpec, GainSpec, SessionSpec, StageSpec, StripSpec, Width};
