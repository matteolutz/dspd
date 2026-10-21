use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::PluginDescriptor;

/// Number of channels a strip carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Width {
    #[default]
    Mono,
    Stereo,
}

impl Width {
    pub fn channels(self) -> usize {
        match self {
            Width::Mono => 1,
            Width::Stereo => 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub width: Width,
    /// First input channel (0-based). A stereo strip also reads `input + 1`.
    pub input: usize,
    /// First output channel. Defaults to the same number as `input`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<usize>,
    #[serde(default)]
    pub chain: Vec<StageSpec>,
}

impl StripSpec {
    pub fn output_channel(&self) -> usize {
        self.output.unwrap_or(self.input)
    }

    pub fn label(&self, index: usize) -> String {
        self.name.clone().unwrap_or_else(|| format!("strip{index}"))
    }
}

/// Wire form of a stage: `type` plus everything else.
#[derive(Serialize, Deserialize)]
struct RawStage {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// One processing stage of a strip.
///
/// In JSON a stage is an object with a `"type"`. `"clap"` loads a CLAP file;
/// any other type names a native plugin from the [`Registry`], and the other
/// keys are that plugin's parameter values:
///
/// ```json
/// {"type": "gain", "gain_db": -3.0}
/// {"type": "clap", "path": "comp.clap", "params": {"7": 0.25}}
/// ```
#[derive(Clone, Debug)]
pub enum StageSpec {
    Native(NativeSpec),
    Clap(ClapSpec),
}

impl Serialize for StageSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let raw = match self {
            StageSpec::Native(n) => {
                let rest = n
                    .settings
                    .iter()
                    .map(|(k, v)| {
                        serde_json::Number::from_f64(*v)
                            .map(|num| (k.to_string(), serde_json::Value::Number(num)))
                            .ok_or_else(|| {
                                S::Error::custom(format!("setting `{k}` is not a finite number"))
                            })
                    })
                    .collect::<Result<_, _>>()?;
                RawStage {
                    kind: n.plugin.clone(),
                    rest,
                }
            }
            StageSpec::Clap(c) => match serde_json::to_value(c).map_err(S::Error::custom)? {
                serde_json::Value::Object(rest) => RawStage {
                    kind: "clap".into(),
                    rest,
                },
                _ => return Err(S::Error::custom("clap spec is not an object")),
            },
        };
        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StageSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = RawStage::deserialize(deserializer)?;

        if raw.kind == "clap" {
            let clap = serde_json::from_value(serde_json::Value::Object(raw.rest))
                .map_err(D::Error::custom)?;
            return Ok(StageSpec::Clap(clap));
        }

        let mut settings = BTreeMap::new();
        for (k, v) in raw.rest {
            let k = k.parse::<u32>().map_err(|_| {
                D::Error::custom(format!(
                    "setting `{k}` of `{}` must be a non-negative integer",
                    raw.kind
                ))
            })?;

            let n = v.as_f64().ok_or_else(|| {
                D::Error::custom(format!(
                    "setting `{k}` of `{}` must be a number, got {v}",
                    raw.kind
                ))
            })?;
            settings.insert(k, n);
        }
        Ok(StageSpec::Native(NativeSpec {
            plugin: raw.kind,
            settings,
        }))
    }
}

/// A native plugin and its stored parameter values, keyed by `ParamDef::key`.
/// Parameters that are absent keep the plugin's default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeSpec {
    /// The plugin's `PluginDescriptor::id`.
    pub plugin: String,
    pub settings: BTreeMap<u32, f64>,
}

impl NativeSpec {
    pub fn new(plugin: impl Into<String>) -> Self {
        Self {
            plugin: plugin.into(),
            settings: BTreeMap::new(),
        }
    }

    /// Builder-style helper: `NativeSpec::new("gain").with(GAIN_PARAM_DB, -3.0)`.
    pub fn with(mut self, id: u32, value: f64) -> Self {
        self.settings.insert(id, value);
        self
    }

    /// Builder-style helper: `NativeSpec::new("gain").with("gain_db", -3.0)`.
    pub fn with_key(mut self, key: &str, value: f64, descriptor: &PluginDescriptor) -> Self {
        let Some(param) = descriptor.param_by_key(key) else {
            return self;
        };

        self.settings.insert(param.id, value);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClapSpec {
    /// Path to the `.clap` file (or bundle on macOS).
    pub path: PathBuf,
    /// Plugin id inside the file. If absent, the first plugin is used.
    #[serde(default)]
    pub plugin_id: Option<String>,
    /// Parameter values by CLAP param id, in the plugin's own units.
    /// JSON object keys are strings, so ids are written as `"7": 0.25`.
    #[serde(default, deserialize_with = "deserialize_param_map")]
    pub params: BTreeMap<u32, f64>,
}

/// Accept string keys for `BTreeMap<u32, f64>`.
///
/// serde_json can parse integer map keys on its own, but not inside an
/// internally tagged enum such as `StageSpec`: serde buffers the content
/// first and the buffered keys stay strings. Parsing them here works in both cases.
fn deserialize_param_map<'de, D>(deserializer: D) -> Result<BTreeMap<u32, f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    BTreeMap::<String, f64>::deserialize(deserializer)?
        .into_iter()
        .map(|(k, v)| {
            k.parse::<u32>().map(|id| (id, v)).map_err(|_| {
                D::Error::custom(format!("parameter id `{k}` is not a non-negative integer"))
            })
        })
        .collect()
}

impl StageSpec {
    pub fn label(&self) -> String {
        match self {
            StageSpec::Native(n) => n.plugin.clone(),
            StageSpec::Clap(c) => c
                .plugin_id
                .clone()
                .unwrap_or_else(|| c.path.display().to_string()),
        }
    }
}
