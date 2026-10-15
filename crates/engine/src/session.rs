//! Serializable session description: the thing you save, recall and snapshot.
//!
//! A session is a list of strips. Each strip is mono or stereo, reads its
//! input from the device's input channels, runs a chain of processors, and
//! writes to the device's output channels:
//!
//! ```json
//! {
//!   "sample_rate": 48000, "max_block": 256, "inputs": 32, "outputs": 32,
//!   "strips": [
//!     { "name": "kick",  "width": "mono",   "input": 0, "chain": [ ... ] },
//!     { "name": "keys",  "width": "stereo", "input": 1, "chain": [ ... ] }
//!   ]
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dyneq::DynamicEq;
use crate::{BoxedProcessor, Chain, Gain, PeakMeter, Rack, Stage, Strip, GAIN_PARAM_DB};

/// Upper bound on device channels in a session; guards against typos allocating gigabytes.
pub const MAX_CHANNELS: usize = 1024;

fn default_sample_rate() -> u32 {
    48_000
}
fn default_max_block() -> usize {
    256
}

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
pub struct SessionSpec {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Largest block the engine will be asked to process.
    #[serde(default = "default_max_block")]
    pub max_block: usize,
    /// Device input channels. Defaults to just enough for the strips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<usize>,
    /// Device output channels. Defaults to just enough for the strips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<usize>,
    pub strips: Vec<StripSpec>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StageSpec {
    Gain(GainSpec),
    DynEq(DynEqSpec),
    Clap(ClapSpec),

    /// Pass-through that publishes the peak level in dBFS as a meter.
    PeakMeter,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GainSpec {
    #[serde(default)]
    pub gain_db: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DynEqBandSpec {
    pub freq: f32,
    pub q: f32,
    pub static_gain_db: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub range_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DynEqSpec {
    #[serde(default)]
    bands: Vec<DynEqBandSpec>,
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
            StageSpec::Gain(_) => "gain".to_string(),
            StageSpec::DynEq(_) => "dyneq".to_string(),
            StageSpec::PeakMeter => "peak_meter".to_string(),
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

    /// Read and validate a session file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let spec = Self::from_json_str(&text).map_err(std::io::Error::other)?;
        spec.validate().map_err(std::io::Error::other)?;
        Ok(spec)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = self.to_json_pretty().map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Smallest number of input channels that covers every strip.
    fn required_inputs(&self) -> usize {
        self.strips
            .iter()
            .map(|s| s.input + s.width.channels())
            .max()
            .unwrap_or(0)
    }

    /// Smallest number of output channels that covers every strip.
    fn required_outputs(&self) -> usize {
        self.strips
            .iter()
            .map(|s| s.output_channel() + s.width.channels())
            .max()
            .unwrap_or(0)
    }

    /// Device input channels the session expects.
    pub fn input_channels(&self) -> usize {
        self.inputs.unwrap_or_else(|| self.required_inputs())
    }

    /// Device output channels the session produces.
    pub fn output_channels(&self) -> usize {
        self.outputs.unwrap_or_else(|| self.required_outputs())
    }

    /// Check the session for mistakes that would otherwise fail confusingly later.
    pub fn validate(&self) -> Result<(), String> {
        if self.sample_rate == 0 {
            return Err("sample_rate must be positive".into());
        }
        if self.max_block == 0 {
            return Err("max_block must be positive".into());
        }
        if self.required_inputs().max(self.required_outputs()) > MAX_CHANNELS {
            return Err(format!("sessions are limited to {MAX_CHANNELS} channels"));
        }
        if let Some(n) = self.inputs {
            if n < self.required_inputs() {
                return Err(format!(
                    "`inputs` is {n} but the strips read up to channel {}",
                    self.required_inputs() - 1
                ));
            }
        }
        if let Some(n) = self.outputs {
            if n < self.required_outputs() {
                return Err(format!(
                    "`outputs` is {n} but the strips write up to channel {}",
                    self.required_outputs() - 1
                ));
            }
        }
        // Two strips on the same output channel would silently overwrite each other.
        let mut owner: Vec<Option<usize>> = vec![None; self.required_outputs()];
        for (i, s) in self.strips.iter().enumerate() {
            for k in 0..s.width.channels() {
                let ch = s.output_channel() + k;
                if let Some(prev) = owner[ch] {
                    return Err(format!(
                        "strips {prev} and {i} both write output channel {ch}"
                    ));
                }
                owner[ch] = Some(i);
            }
        }
        Ok(())
    }

    /// Record a parameter change in the spec so that a later save/recall sees it.
    pub fn apply_param(&mut self, strip: usize, stage: usize, param_id: u32, value: f64) -> bool {
        match self
            .strips
            .get_mut(strip)
            .and_then(|s| s.chain.get_mut(stage))
        {
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

    /// Instantiate every strip. Built-in stages are created here; CLAP stages are
    /// delegated to `load_clap`, which keeps this crate free of plugin-host code.
    /// `load_clap` also receives the strip's channel count (1 or 2), so the host
    /// can decide how to run a plugin whose own port layout differs.
    /// Stored parameter values are applied to every stage after creation.
    pub fn build_rack<E>(
        &self,
        mut load_clap: impl FnMut(&ClapSpec, usize) -> Result<BoxedProcessor, E>,
    ) -> Result<Rack, E> {
        let mut strips = Vec::with_capacity(self.strips.len());
        for (index, strip) in self.strips.iter().enumerate() {
            let width = strip.width.channels();
            let mut stages = Vec::with_capacity(strip.chain.len());
            for spec in &strip.chain {
                let mut processor: BoxedProcessor = match spec {
                    StageSpec::Gain(g) => Box::new(Gain::new(g.gain_db)),
                    StageSpec::DynEq(eq) => Box::new(DynamicEq::new_with_bands(
                        self.sample_rate as f32,
                        width,
                        &eq.bands,
                    )),
                    StageSpec::PeakMeter => Box::new(PeakMeter::new(self.sample_rate)),
                    StageSpec::Clap(c) => load_clap(c, width)?,
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
            let chain = Chain::new(width, self.max_block, stages);
            strips.push(Strip::new(
                strip.label(index),
                strip.input,
                strip.output_channel(),
                chain,
            ));
        }
        Ok(Rack::new(strips))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Processor;

    fn two_strip_json() -> &'static str {
        r#"{
            "strips": [
                {"name": "vox", "input": 0, "chain": [
                    {"type": "gain", "gain_db": -3.0},
                    {"type": "clap", "path": "plug.clap", "params": {"7": 0.25}}
                ]},
                {"width": "stereo", "input": 1, "output": 4}
            ]
        }"#
    }

    #[test]
    fn json_roundtrip_with_defaults() {
        let spec = SessionSpec::from_json_str(two_strip_json()).unwrap();
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.max_block, 256);
        assert_eq!(spec.strips[0].width, Width::Mono);
        assert_eq!(spec.strips[1].width, Width::Stereo);
        assert_eq!(spec.strips[1].output_channel(), 4);
        assert_eq!(spec.strips[0].output_channel(), 0);

        let again = SessionSpec::from_json_str(&spec.to_json_pretty().unwrap()).unwrap();
        match &again.strips[0].chain[1] {
            StageSpec::Clap(c) => assert_eq!(c.params.get(&7), Some(&0.25)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn channel_counts_default_to_what_the_strips_need() {
        let mut spec = SessionSpec::from_json_str(two_strip_json()).unwrap();
        assert_eq!(spec.input_channels(), 3); // mono at 0, stereo at 1..=2
        assert_eq!(spec.output_channels(), 6); // stereo at 4..=5
        spec.inputs = Some(32);
        spec.outputs = Some(32);
        assert_eq!((spec.input_channels(), spec.output_channels()), (32, 32));
        spec.validate().unwrap();
    }

    #[test]
    fn validation_catches_common_mistakes() {
        let mut spec = SessionSpec::from_json_str(two_strip_json()).unwrap();
        spec.validate().unwrap();

        spec.outputs = Some(3);
        assert!(spec.validate().unwrap_err().contains("`outputs` is 3"));
        spec.outputs = None;

        spec.strips[1].output = Some(0); // collides with the mono strip on channel 0
        assert!(spec
            .validate()
            .unwrap_err()
            .contains("both write output channel 0"));
        spec.strips[1].output = None;

        spec.strips[0].input = 5000;
        assert!(spec.validate().unwrap_err().contains("limited to"));
    }

    #[test]
    fn two_strips_may_read_the_same_input() {
        let spec = SessionSpec::from_json_str(
            r#"{"strips":[{"input":0,"output":0},{"input":0,"output":1}]}"#,
        )
        .unwrap();
        spec.validate().unwrap();
    }

    #[test]
    fn apply_param_updates_spec() {
        let mut spec = SessionSpec::from_json_str(two_strip_json()).unwrap();
        assert!(spec.apply_param(0, 0, GAIN_PARAM_DB, -9.0));
        assert!(spec.apply_param(0, 1, 3, 0.5));
        assert!(!spec.apply_param(0, 0, 99, 1.0)); // gain has only param 0
        assert!(!spec.apply_param(5, 0, 0, 1.0)); // no such strip
        assert!(!spec.apply_param(1, 0, 0, 1.0)); // strip 1 has no stages
        match &spec.strips[0].chain[0] {
            StageSpec::Gain(g) => assert_eq!(g.gain_db, -9.0),
            _ => unreachable!(),
        }
    }

    #[test]
    fn build_rack_creates_all_strips_with_their_widths() {
        let spec = SessionSpec::from_json_str(
            r#"{"strips":[
                {"input":0,"chain":[{"type":"gain","gain_db":0}]},
                {"width":"stereo","input":1,"chain":[]}
            ]}"#,
        )
        .unwrap();
        let rack = spec
            .build_rack(|_, _| -> Result<BoxedProcessor, String> { Err("no clap".into()) })
            .unwrap();
        assert_eq!(rack.strips().len(), 2);
        assert_eq!(rack.strips()[0].width(), 1);
        assert_eq!(rack.strips()[1].width(), 2);
        assert_eq!(rack.latency_frames(), 0);
    }

    #[test]
    fn clap_loader_is_told_the_strip_width() {
        let spec = SessionSpec::from_json_str(
            r#"{"strips":[
                {"input":0,"chain":[{"type":"clap","path":"a.clap"}]},
                {"width":"stereo","input":1,"chain":[{"type":"clap","path":"a.clap"}]}
            ]}"#,
        )
        .unwrap();
        let mut widths = Vec::new();
        let _ = spec.build_rack(|_, w| -> Result<BoxedProcessor, String> {
            widths.push(w);
            Ok(Box::new(Gain::new(0.0)))
        });
        assert_eq!(widths, vec![1, 2]);
    }
}
