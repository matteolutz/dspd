//! Platform-independent core of dspd: audio blocks, the `Processor` trait,
//! processing chains, the rack of mono/stereo strips, the parameter queue and
//! the serializable session spec.
//!
//! Nothing in this crate touches an audio device, a plugin format or the OS.

mod chain;
mod dual_mono;
mod edit;
mod engine;
mod live;
mod params;
mod rack;
pub mod session;

mod ir_loader;
mod registry;

pub use chain::{Chain, Stage, MAX_STAGES};
pub use dual_mono::DualMono;
pub use edit::{edit_channel, EditBatch, EditOp, EditReceiver, EditSender, Retired};
pub use engine::{Engine, EngineStats, StatsSnapshot};
pub use live::{EditError, Editor, Inserted};
pub use params::{param_channel, ParamEvent, ParamReceiver, ParamSender};
pub use rack::{DelayLine, Rack, Strip};
pub use registry::Registry;
pub use session::SessionSpec;

pub use dspd_api::{
    AudioBlock, BoxedProcessor, ClapSpec, MeterBank, MeterInfo, MeterSpec, MeterWriter, NativeSpec,
    ParamDef, PluginContext, PluginDescriptor, Processor, StageSpec, StripSpec, Width,
};
