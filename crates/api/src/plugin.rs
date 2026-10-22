//! How a native plugin describes itself to the host.
//!
//! A plugin crate exports one `static` [`PluginDescriptor`]. The host collects
//! descriptors in a registry (see `dspd_engine::Registry`), so the engine never
//! has to know which plugins exist.
//!
//! ```ignore
//! pub static DESCRIPTOR: PluginDescriptor = PluginDescriptor {
//!     id: "gain",
//!     name: "Gain",
//!     description: "Simple gain stage",
//!     params: &[ParamDef { id: 0, key: "gain_db", name: "Gain", min: -60.0, max: 24.0, default: 0.0 }],
//!     create: |_ctx| Box::new(Gain::new(0.0)),
//! };
//! ```

use crate::processor::BoxedProcessor;

/// One settable parameter of a native plugin.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ParamDef {
    /// Id passed to `Processor::set_param` and used by `POST /params`.
    pub id: u32,
    /// Key in the session JSON, e.g. `"gain_db"` in `{"type":"gain","gain_db":-3}`.
    /// unique inside the plugin.
    pub key: &'static str,
    /// Human-readable name for a frontend.
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// What a plugin may need to know when it is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginContext {
    pub sample_rate: u32,
    /// Largest block `process` will ever see.
    pub max_block: usize,
    /// Channels of the strip the plugin sits on (1 or 2).
    pub channels: usize,
}

/// Everything the host needs to list and instantiate a native plugin.
#[derive(serde::Serialize)]
pub struct PluginDescriptor {
    /// Stable identifier, used as `"type"` in session files. Lowercase snake_case.
    pub id: &'static str,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamDef>,
    /// Build a processor with every parameter at its default. The host then
    /// applies the values stored in the session through `set_param`.
    #[serde(skip)]
    pub create: Box<dyn Fn(&PluginContext) -> BoxedProcessor + Send + Sync>,
}

impl PluginDescriptor {
    pub fn param_by_key(&self, key: &str) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.key == key)
    }

    pub fn param_by_id(&self, id: u32) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
}
