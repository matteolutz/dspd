//! The interface between dspd and its native plugins.
//!
//! A plugin crate depends on this crate only (not on the engine): it gets the
//! [`Processor`] trait, [`AudioBlock`], the meter types and the
//! [`PluginDescriptor`] it exports.

mod block;
mod meters;
mod plugin;
mod processor;
mod spec;

pub use block::AudioBlock;
pub use meters::{MeterBank, MeterInfo, MeterWriter, OptionalMeterWriter};
pub use plugin::{ParamDef, PluginContext, PluginDescriptor};
pub use processor::{BoxedProcessor, MeterSpec, Processor};
pub use spec::{ClapSpec, NativeSpec, StageSpec, StripSpec, Width};
