//! A minimal headless CLAP host.
//!
//! * [`inspect`] lists the plugins, parameters and audio ports in a `.clap` file.
//! * [`ClapProcessor`] runs one plugin as a `dspd_engine::Processor`.
//!
//! Not implemented yet: plugin GUIs, MIDI/note events, transport, host
//! extensions (log, thread-check, latency-changed, ...), state save/load, and
//! sharing one loaded library between several instances of the same file.

mod entry;
mod inspect;
mod processor;

pub use inspect::{inspect, ParamInfo, PluginInfo};
pub use processor::ClapProcessor;
