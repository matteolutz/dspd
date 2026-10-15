//! A minimal headless CLAP host.
//!
//! * [`inspect`] lists the plugins, parameters and audio ports in a `.clap` file.
//! * [`ClapProcessor`] runs one plugin as a `dspd_engine::Processor`.
//!
//! Loading the same `.clap` file repeatedly (one instance per strip) shares one
//! loaded library, so the plugin's entry `init`/`deinit` run once.
//!
//! Not implemented yet: plugin GUIs, MIDI/note events, transport, host
//! extensions (log, thread-check, latency-changed, ...) and state save/load.

mod entry;
mod inspect;
mod processor;

pub use inspect::{inspect, ParamInfo, PluginInfo};
pub use processor::ClapProcessor;
