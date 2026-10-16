//! Platform-independent core of dspd: audio blocks, the `Processor` trait,
//! processing chains, the rack of mono/stereo strips, the parameter queue and
//! the serializable session spec.
//!
//! Nothing in this crate touches an audio device, a plugin format or the OS.

mod chain;
mod dual_mono;
mod engine;
mod params;
mod rack;
pub mod session;

mod ir_loader;
mod registry;

pub use chain::{Chain, Stage};
pub use dual_mono::DualMono;
pub use engine::{Engine, EngineStats, StatsSnapshot};
pub use params::{param_channel, ParamEvent, ParamReceiver, ParamSender};
pub use rack::{Rack, Strip};
pub use registry::Registry;
pub use session::{ClapSpec, GainSpec, SessionSpec, StageSpec, StripSpec, Width};

pub use dspd_api::{
    AudioBlock, BoxedProcessor, MeterBank, MeterInfo, MeterSpec, MeterWriter, ParamDef,
    PluginContext, PluginDescriptor, Processor,
};
