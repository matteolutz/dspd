//! Serializable session description: the thing you save, recall and snapshot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{BoxedProcessor, Chain, Gain, Stage, GAIN_PARAM_DB};

fn default_sample_rate() -> u32 {
    48_000
}
fn default_channels() -> usize {
    2
}
fn default_max_block() -> usize {
    256
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSpec {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: usize,
    /// Largest block the engine will be asked to process.
    #[serde(default = "default_max_block")]
    pub max_block: usize,
    pub chain: Vec<StageSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StageSpec {
    Gain(GainSpec),
    Clap(ClapSpec),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GainSpec {
    #[serde(default)]
    pub gain_db: f32,
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
            k.parse::<u32>()
                .map(|id| (id, v))
                .map_err(|_| D::Error::custom(format!("parameter id `{k}` is not a non-negative integer")))
        })
        .collect()
}

impl StageSpec {
    pub fn label(&self) -> String {
        match self {
            StageSpec::Gain(_) => "gain".to_string(),
            StageSpec::Clap(c) => c
                .plugin_id
                .clone()
                .unwrap_or_else(|| c.path.display().to_string()),
        }
    }
}

impl SessionSpec {
    pub fn from_json_str(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json_str(&text).map_err(std::io::Error::other)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = self.to_json_pretty().map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Record a parameter change in the spec so that a later save/recall sees it.
    pub fn apply_param(&mut self, stage: usize, param_id: u32, value: f64) -> bool {
        match self.chain.get_mut(stage) {
            Some(StageSpec::Gain(g)) if param_id == GAIN_PARAM_DB => {
                g.gain_db = value as f32;
                true
            }
            Some(StageSpec::Clap(c)) => {
                c.params.insert(param_id, value);
                true
            }
            _ => false,
        }
    }

    /// Instantiate the chain. Built-in stages are created here; CLAP stages are
    /// delegated to `load_clap`, which keeps this crate free of plugin-host code.
    /// Stored parameter values are applied to every stage after creation.
    pub fn build_chain<E>(
        &self,
        mut load_clap: impl FnMut(&ClapSpec) -> Result<BoxedProcessor, E>,
    ) -> Result<Chain, E> {
        let mut stages = Vec::with_capacity(self.chain.len());
        for spec in &self.chain {
            let mut processor: BoxedProcessor = match spec {
                StageSpec::Gain(g) => Box::new(Gain::new(g.gain_db)),
                StageSpec::Clap(c) => load_clap(c)?,
            };
            if let StageSpec::Clap(c) = spec {
                for (id, value) in &c.params {
                    processor.set_param(*id, *value);
                }
            }
            stages.push(Stage {
                label: spec.label(),
                processor,
            });
        }
        Ok(Chain::new(self.channels, self.max_block, stages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_with_defaults() {
        let json = r#"{
            "chain": [
                {"type": "gain", "gain_db": -3.0},
                {"type": "clap", "path": "plug.clap", "params": {"7": 0.25}}
            ]
        }"#;
        let spec = SessionSpec::from_json_str(json).unwrap();
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.chain.len(), 2);

        let again = SessionSpec::from_json_str(&spec.to_json_pretty().unwrap()).unwrap();
        match &again.chain[1] {
            StageSpec::Clap(c) => assert_eq!(c.params.get(&7), Some(&0.25)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn apply_param_updates_spec() {
        let mut spec = SessionSpec {
            sample_rate: 48_000,
            channels: 2,
            max_block: 128,
            chain: vec![StageSpec::Gain(GainSpec { gain_db: 0.0 })],
        };
        assert!(spec.apply_param(0, GAIN_PARAM_DB, -9.0));
        assert!(!spec.apply_param(5, 0, 1.0));
        match &spec.chain[0] {
            StageSpec::Gain(g) => assert_eq!(g.gain_db, -9.0),
            _ => unreachable!(),
        }
    }

    #[test]
    fn build_chain_creates_builtin_stages() {
        let spec = SessionSpec::from_json_str(r#"{"chain":[{"type":"gain","gain_db":0}]}"#).unwrap();
        let chain = spec
            .build_chain(|_| -> Result<BoxedProcessor, String> { Err("no clap".into()) })
            .unwrap();
        assert_eq!(chain.stages().len(), 1);
        assert_eq!(chain.channels(), 2);
    }
}
