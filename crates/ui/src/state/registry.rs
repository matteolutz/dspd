use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct ParamDef {
    /// Id passed to `Processor::set_param` and used by `POST /params`.
    pub id: u32,
    /// Key in the session JSON, e.g. `"gain_db"` in `{"type":"gain","gain_db":-3}`.
    pub key: String,
    /// Human-readable name for a frontend.
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// Everything the host needs to list and instantiate a native plugin.
#[derive(serde::Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamDef>,
}

#[derive(serde::Deserialize)]
pub struct SerializableRegistry {
    plugins: Vec<PluginDescriptor>,
}

pub struct Registry {
    plugins: BTreeMap<String, PluginDescriptor>,
}

impl Registry {
    pub fn from_serializable(raw: SerializableRegistry) -> Self {
        let plugins = raw.plugins.into_iter().map(|p| (p.id.clone(), p)).collect();
        Self { plugins }
    }
}

impl Registry {
    pub fn get_plugin(&self, id: &str) -> Option<&PluginDescriptor> {
        self.plugins.get(id)
    }

    pub fn iter_plugins(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins.values()
    }
}

impl<'de> serde::Deserialize<'de> for Registry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = SerializableRegistry::deserialize(deserializer)?;
        Ok(Self::from_serializable(raw))
    }
}
