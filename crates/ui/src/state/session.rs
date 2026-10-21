use gpui_kit::{App, SharedString, WeakEntity};

use crate::state::MetersState;

#[derive(serde::Deserialize)]
pub struct SerializableSession {
    sample_rate: u32,
    max_block: usize,

    strips: Vec<SerializableStrip>,
}

pub struct Session {
    pub(super) sample_rate: u32,
    pub(super) max_block: usize,

    pub(super) strips: Vec<Strip>,
}

impl Session {
    pub fn from_serializable(value: SerializableSession, meters: WeakEntity<MetersState>) -> Self {
        Self {
            sample_rate: value.sample_rate,
            max_block: value.max_block,
            strips: value
                .strips
                .into_iter()
                .enumerate()
                .map(|(idx, strip)| Strip::from_serializable(idx, strip, meters.clone()))
                .collect(),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripWidth {
    Mono,
    Stereo,
}

#[derive(serde::Deserialize)]
pub struct SerializableStrip {
    name: String,

    width: StripWidth,

    input: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<usize>,

    chain: Vec<SerializableStage>,
}

pub struct Strip {
    // ref to the meters state entity, so we can query the meters for this strip
    meters: WeakEntity<MetersState>,

    /// The strips idx in the strip list
    strip_idx: usize,

    pub(crate) name: SharedString,
    pub(crate) input: usize,

    chain: Vec<Stage>,
}

impl Strip {
    pub fn get_level_meters<'a>(&self, cx: &'a App) -> Option<impl Iterator<Item = f32> + 'a> {
        let last_stage = self.chain.last()?;

        if !last_stage.is_native_plugin("peak_meter") {
            return None;
        }

        Some(
            last_stage
                .get_meters(cx)
                .map(|(_, _, meter_value)| meter_value.unwrap_or(0.0)),
        )
    }
}

impl Strip {
    pub fn from_serializable(
        strip_idx: usize,
        strip: SerializableStrip,
        meters: WeakEntity<MetersState>,
    ) -> Self {
        Self {
            meters: meters.clone(),
            strip_idx,
            name: SharedString::from(strip.name),
            input: strip.input,
            chain: strip
                .chain
                .into_iter()
                .map(|stage| Stage::from_serializable(strip_idx, stage, meters.clone()))
                .collect(),
        }
    }
}

#[derive(serde::Deserialize)]
pub struct SerializableStage {
    id: u32,
    #[serde(flatten)]
    spec: dspd_api::StageSpec,
}

pub struct Stage {
    // ref to the meters state entity, so we can query the meters for this stage
    meters: WeakEntity<MetersState>,

    /// The strip idx in the strip list of a session
    strip_idx: usize,

    stage_id: u32,
    spec: dspd_api::StageSpec,
}

impl Stage {
    pub fn is_native_plugin(&self, plugin_name: &str) -> bool {
        match &self.spec {
            dspd_api::StageSpec::Native(native_spec) => native_spec.plugin == plugin_name,
            _ => false,
        }
    }

    pub fn get_meters<'a>(
        &self,
        cx: &'a App,
    ) -> impl Iterator<Item = (u32, &'a String, Option<f32>)> {
        self.meters
            .upgrade()
            .expect("MetersState entity has been dropped")
            .read(cx)
            .meters_for_stage(self.strip_idx, self.stage_id)
    }
}

impl Stage {
    pub fn from_serializable(
        strip_idx: usize,
        stage: SerializableStage,
        meters: WeakEntity<MetersState>,
    ) -> Self {
        Self {
            meters,
            strip_idx,
            stage_id: stage.id,
            spec: stage.spec,
        }
    }
}
