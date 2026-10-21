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

use std::path::{Path, PathBuf};

use dspd_api::{PluginContext, StageSpec, StripSpec};
use serde::{Deserialize, Serialize};

use crate::{BoxedProcessor, Chain, Rack, Registry, Stage, Strip};

/// Upper bound on device channels in a session; guards against typos allocating gigabytes.
pub const MAX_CHANNELS: usize = 1024;

fn default_sample_rate() -> u32 {
    48_000
}
fn default_max_block() -> usize {
    256
}

/// Why a session could not be turned into a rack.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The spec names an unknown plugin or setting.
    #[error("{0}")]
    Spec(String),

    /// A CLAP stage failed to load.
    #[error("{0}")]
    Factory(String),
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IrLoaderSpec {
    ir: PathBuf,
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
            if s.chain.len() > crate::MAX_STAGES {
                return Err(format!(
                    "strip {i} has {} stages; the limit is {}",
                    s.chain.len(),
                    crate::MAX_STAGES
                ));
            }

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
    /// Returns false if the strip, stage or parameter does not exist.
    pub fn apply_param(
        &mut self,
        strip: usize,
        stage_idx: usize,
        param_id: u32,
        value: f64,
        registry: &Registry,
    ) -> bool {
        match self
            .strips
            .get_mut(strip)
            .and_then(|s| s.chain.get_mut(stage_idx))
        {
            Some(StageSpec::Native(n))
                if registry
                    .get(&n.plugin)
                    .is_some_and(|desc| desc.param_by_id(param_id).is_some()) =>
            {
                n.settings.insert(param_id, value);
                true
            }
            _ => false,
        }
    }

    /// Check that every stage names a registered plugin and only sets
    /// parameters it has
    pub fn check_plugins(&self, registry: &Registry) -> Result<(), String> {
        for (si, strip) in self.strips.iter().enumerate() {
            for (ti, stage) in strip.chain.iter().enumerate() {
                let StageSpec::Native(n) = stage else {
                    continue;
                };

                let at = format!("strip {si} stage {ti}");
                let desc = registry.get(&n.plugin).ok_or_else(|| {
                    let known: Vec<_> = registry.iter().map(|d| d.id).collect();
                    format!(
                        "{at}: unknown plugin type `{}` (plugins: {})",
                        n.plugin,
                        known.join(", ")
                    )
                })?;

                for &id in n.settings.keys() {
                    if desc.param_by_id(id).is_none() {
                        let ids: Vec<_> = desc.params.iter().map(|p| p.id.to_string()).collect();

                        return Err(format!(
                            "{at}: plugin `{}` has no setting with id {id} (it has: {})",
                            n.plugin,
                            if ids.is_empty() {
                                "none".to_string()
                            } else {
                                ids.join(", ")
                            }
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Build the processor for one stage of this spec: native stages come from
    /// `registry` (their resources are loaded from `assets`), CLAP stages from
    /// `stage_factory`. Stored parameter values are applied before it is returned.
    /// The caller must have run [`SessionSpec::check_plugins`] on this spec.
    pub fn build_stage(
        &self,
        registry: &Registry,
        strip: usize,
        stage_idx: usize,
    ) -> Result<BoxedProcessor, BuildError> {
        let width = self.strips[strip].width.channels();
        let processor = match &self.strips[strip].chain[stage_idx] {
            StageSpec::Native(n) => {
                // check_plugins has already proven the lookups succeed.
                let desc = registry.get(&n.plugin).expect("checked by check_plugins");
                let ctx = PluginContext {
                    sample_rate: self.sample_rate,
                    max_block: self.max_block,
                    channels: width,
                };

                let mut processor = (desc.create)(&ctx);

                for (&id, value) in &n.settings {
                    let def = desc.param_by_id(id).expect("checked by check_plugins");
                    processor.set_param(def.id, *value);
                }

                processor
            }
        };
        Ok(processor)
    }

    /// Instantiate every strip. Built-in stages are created here; CLAP stages are
    /// delegated to `load_clap`, which keeps this crate free of plugin-host code.
    /// `load_clap` also receives the strip's channel count (1 or 2), so the host
    /// can decide how to run a plugin whose own port layout differs.
    /// Stored parameter values are applied to every stage after creation.
    pub fn build_rack(&self, registry: &Registry) -> Result<Rack, BuildError> {
        self.check_plugins(registry).map_err(BuildError::Spec)?;

        let mut strips = Vec::with_capacity(self.strips.len());
        for (index, strip) in self.strips.iter().enumerate() {
            let width = strip.width.channels();
            let mut stages = Vec::with_capacity(strip.chain.len());
            for spec in &strip.chain {
                let processor: BoxedProcessor = match spec {
                    StageSpec::Native(n) => {
                        // check_plugins has already proven the lookups succeed.
                        let desc = registry.get(&n.plugin).expect("checked by check_plugins");
                        let ctx = PluginContext {
                            sample_rate: self.sample_rate,
                            max_block: self.max_block,
                            channels: width,
                        };

                        let mut processor = (desc.create)(&ctx);

                        for (&id, value) in &n.settings {
                            let def = desc.param_by_id(id).expect("checked by check_plugins");
                            processor.set_param(def.id, *value);
                        }

                        processor
                    }
                };

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
    use dspd_api::NativeSpec;
    use dspd_plugin_gain::GAIN_PARAM_DB;

    fn registry() -> Registry {
        Registry::from_descriptors([
            dspd_plugin_gain::descriptor(),
            dspd_plugin_peak_meter::descriptor(),
        ])
        .unwrap()
    }

    fn two_strip_json() -> &'static str {
        r#"{
            "strips": [
                {"name": "vox", "input": 0, "chain": [
                    {"type": "gain", "0": -3.0}
                ]},
                {"width": "stereo", "input": 1, "output": 4}
            ]
        }"#
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
    fn peak_meter_stage_parses_builds_and_has_no_params() {
        let mut spec = SessionSpec::from_json_str(
            r#"{"strips":[{"input":0,"chain":[{"type":"peak_meter"}]}]}"#,
        )
        .unwrap();
        assert!(
            !spec.apply_param(0, 0, 0, 1.0, &registry()),
            "a meter has no params"
        );
        let rack = spec.build_rack(&registry()).unwrap();
        assert_eq!(rack.meter_infos().len(), 1);
        // round-trips through JSON
        spec.strips[0].name = Some("x".into());
        let again = SessionSpec::from_json_str(&spec.to_json_pretty().unwrap()).unwrap();
        assert!(
            matches!(&again.strips[0].chain[0], StageSpec::Native(n) if n.plugin == "peak_meter")
        );
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
        assert!(spec.apply_param(0, 0, GAIN_PARAM_DB, -9.0, &registry()));
        assert!(!spec.apply_param(0, 0, 99, 1.0, &registry())); // gain has only param 0
        assert!(!spec.apply_param(5, 0, 0, 1.0, &registry())); // no such strip
        assert!(!spec.apply_param(1, 0, 0, 1.0, &registry())); // strip 1 has no stages
        match &spec.strips[0].chain[0] {
            StageSpec::Native(n) => assert_eq!(n.settings[&GAIN_PARAM_DB], -9.0),
        }
    }

    #[test]
    fn build_rack_creates_all_strips_with_their_widths() {
        let spec = SessionSpec::from_json_str(
            r#"{"strips":[
                {"input":0,"chain":[{"type":"gain","0":0}]},
                {"width":"stereo","input":1,"chain":[]}
            ]}"#,
        )
        .unwrap();
        let rack = spec.build_rack(&registry()).unwrap();
        assert_eq!(rack.strips().len(), 2);
        assert_eq!(rack.strips()[0].width(), 1);
        assert_eq!(rack.strips()[1].width(), 2);
        assert_eq!(rack.latency_frames(), 0);
    }

    #[test]
    fn native_stage_roundtrips_and_applies_stored_settings() {
        let spec = SessionSpec::from_json_str(
            r#"{"strips":[{"input":0,"chain":[{"type":"gain","0":-6.0206}]}]}"#,
        )
        .unwrap();
        let again = SessionSpec::from_json_str(&spec.to_json_pretty().unwrap()).unwrap();
        match &again.strips[0].chain[0] {
            StageSpec::Native(n) => {
                assert_eq!(*n, NativeSpec::new("gain").with(GAIN_PARAM_DB, -6.0206))
            }
        }
        // The serialized form keeps the flat shape.
        let v: serde_json::Value = serde_json::from_str(&spec.to_json_pretty().unwrap()).unwrap();
        assert_eq!(v["strips"][0]["chain"][0]["type"], "gain");
        assert_eq!(v["strips"][0]["chain"][0]["0"], -6.0206);

        // The stored value is applied before the first block, without a ramp.
        let mut rack = spec.build_rack(&registry()).unwrap();
        let mut input = crate::AudioBlock::new(1, 8);
        input.channel_mut(0).fill(1.0);
        let mut output = crate::AudioBlock::new(1, 8);
        rack.process(&input, &mut output);
        assert!(
            output.channel(0).iter().all(|s| (s - 0.5).abs() < 1e-3),
            "{:?}",
            output.channel(0)
        );
    }

    #[test]
    fn unknown_plugin_or_setting_is_a_clear_error() {
        let spec =
            SessionSpec::from_json_str(r#"{"strips":[{"input":0,"chain":[{"type":"reverb"}]}]}"#)
                .unwrap();
        let e = spec.build_rack(&registry()).err().unwrap().to_string();
        assert!(
            e.contains("strip 0 stage 0")
                && e.contains("unknown plugin type `reverb`")
                && e.contains("gain"),
            "{e}"
        );

        let spec = SessionSpec::from_json_str(
            r#"{"strips":[{"input":0,"chain":[{"type":"gain","1":1}]}]}"#,
        )
        .unwrap();
        let e = spec.build_rack(&registry()).err().unwrap().to_string();
        assert!(e.contains("no setting with id 1") && e.contains("0"), "{e}");

        let e = SessionSpec::from_json_str(
            r#"{"strips":[{"input":0,"chain":[{"type":"gain","0":"loud"}]}]}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("must be a number"), "{e}");
    }
}
