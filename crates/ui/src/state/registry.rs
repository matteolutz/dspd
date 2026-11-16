use dspd_api::ParamUnit;
use std::collections::BTreeMap;

use crate::utils::map_range;

#[derive(Debug, Copy, Clone, Default)]
pub enum ParamUnitScaling {
    #[default]
    Linear,
    NatLog,
}

impl ParamUnitScaling {
    pub fn to_scaled(&self, norm_value: f32, min: f32, max: f32) -> f32 {
        match self {
            Self::Linear => map_range(norm_value, 0.0, 1.0, min, max),
            Self::NatLog => min * f32::powf(max / min, norm_value),
        }
    }

    pub fn to_normalized(&self, value: f32, min: f32, max: f32) -> f32 {
        match self {
            Self::Linear => map_range(value, min, max, 0.0, 1.0),
            Self::NatLog => (value / min).ln() / (max / min).ln(),
        }
    }
}

pub trait ParamUnitToString {
    fn as_str(&self) -> &'static str;
}

impl ParamUnitToString for ParamUnit {
    fn as_str(&self) -> &'static str {
        match self {
            ParamUnit::Linear => "",
            ParamUnit::Decibels => "dB",
            ParamUnit::Percent => "%",
            ParamUnit::Seconds => "s",
            ParamUnit::Milliseconds => "ms",
            ParamUnit::Hertz => "Hz",
        }
    }
}

pub trait GetParamUnitScaling {
    fn scaling(&self) -> ParamUnitScaling;
}

impl GetParamUnitScaling for ParamUnit {
    fn scaling(&self) -> ParamUnitScaling {
        match self {
            Self::Hertz => ParamUnitScaling::NatLog,
            _ => ParamUnitScaling::Linear,
        }
    }
}

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
    pub unit: ParamUnit,
}

impl ParamDef {
    pub fn value_to_scaled(&self, value: f32) -> f32 {
        self.unit
            .scaling()
            .to_scaled(value, self.min as f32, self.max as f32)
    }

    pub fn value_to_normalized(&self, value: f32) -> f32 {
        self.unit
            .scaling()
            .to_normalized(value, self.min as f32, self.max as f32)
    }
}

/// Everything the host needs to list and instantiate a native plugin.
#[derive(Clone, serde::Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamDef>,
}

impl PluginDescriptor {
    pub fn find_param(&self, id: u32) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
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
