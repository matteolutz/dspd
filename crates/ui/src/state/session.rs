use dspd_api::StageSpec;
use gpui_kit::{App, AppContext, Context, Entity, SharedString, WeakEntity};

use crate::state::{registry::Registry, MetersState};

#[derive(serde::Deserialize)]
pub struct SerializableSession {
    sample_rate: u32,
    max_block: usize,

    strips: Vec<SerializableStrip>,
}

pub struct Session {
    pub(super) sample_rate: u32,
    pub(super) max_block: usize,

    pub(super) strips: Vec<Entity<Strip>>,
}

impl Session {
    pub fn from_serializable(
        value: SerializableSession,
        meters: WeakEntity<MetersState>,
        registry: WeakEntity<Registry>,
        cx: &mut App,
    ) -> Self {
        Self {
            sample_rate: value.sample_rate,
            max_block: value.max_block,
            strips: value
                .strips
                .into_iter()
                .enumerate()
                .map(|(idx, strip)| {
                    cx.new(|cx| {
                        Strip::from_serializable(idx, strip, meters.clone(), registry.clone(), cx)
                    })
                })
                .collect(),
        }
    }

    pub fn set_param(
        &self,
        strip_idx: usize,
        stage_id: u32,
        param_id: u32,
        value: f64,
        cx: &mut App,
    ) {
        let Some(strip) = self.strips.get(strip_idx) else {
            return;
        };

        strip.update(cx, |strip, cx| {
            strip.set_param(stage_id, param_id, value, cx)
        });
    }

    pub fn insert_stage(
        &self,
        strip_idx: usize,
        stage: StageSpec,
        stage_id: u32,
        position: usize,
        cx: &mut App,
    ) {
        let Some(strip) = self.strips.get(strip_idx) else {
            return;
        };

        strip.update(cx, |strip, cx| {
            strip.insert_stage(stage, stage_id, position, cx);
        });
    }

    pub fn remove_stage(&self, strip_idx: usize, stage_id: u32, cx: &mut App) {
        let Some(strip) = self.strips.get(strip_idx) else {
            return;
        };

        strip.update(cx, |strip, cx| {
            strip.remove_stage(stage_id, cx);
        });
    }

    pub fn move_stage(&self, strip_idx: usize, stage_id: u32, new_position: usize, cx: &mut App) {
        let Some(strip) = self.strips.get(strip_idx) else {
            return;
        };

        strip.update(cx, |strip, cx| {
            strip.move_stage(stage_id, new_position, cx);
        });
    }

    pub fn set_stage_order(&self, strip_idx: usize, order: Vec<u32>, cx: &mut App) {
        let Some(strip) = self.strips.get(strip_idx) else {
            return;
        };

        strip.update(cx, |strip, cx| {
            strip.set_stage_order(order, cx);
        });
    }

    pub fn strips(&self) -> impl Iterator<Item = &Entity<Strip>> {
        self.strips.iter()
    }
}

#[derive(serde::Deserialize)]
pub struct SerializableStrip {
    name: String,

    width: dspd_api::Width,

    input: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<usize>,

    chain: Vec<SerializableStage>,
}

pub struct Strip {
    // ref to the meters state entity, so we can query the meters for this strip
    meters: WeakEntity<MetersState>,

    registry: WeakEntity<Registry>,

    /// The strips idx in the strip list
    pub(crate) strip_idx: usize,

    pub(crate) name: SharedString,
    pub(crate) input: usize,
    pub(crate) width: dspd_api::Width,

    chain: Vec<Entity<Stage>>,
}

impl Strip {
    pub fn get_input_level_meters<'a>(
        &self,
        cx: &'a App,
    ) -> Option<impl Iterator<Item = f32> + 'a> {
        if self.chain.len() <= 1 {
            // even if the one plugin we have in the chain would
            // be a peak_meter, it would be considered the output level meter
            return None;
        }

        let first_stage = self.chain.first()?.read(cx);

        if !first_stage.is_native_plugin("peak_meter") {
            return None;
        }

        Some(
            first_stage
                .get_meters(cx)
                .map(|(_, _, meter_value)| meter_value.unwrap_or(0.0)),
        )
    }

    pub fn get_output_level_meters<'a>(
        &self,
        cx: &'a App,
    ) -> Option<impl Iterator<Item = f32> + 'a> {
        let last_stage = self.chain.last()?.read(cx);

        if !last_stage.is_native_plugin("peak_meter") {
            return None;
        }

        Some(
            last_stage
                .get_meters(cx)
                .map(|(_, _, meter_value)| meter_value.unwrap_or(0.0)),
        )
    }

    pub fn get_input_channel_str(&self) -> String {
        match self.width {
            dspd_api::Width::Mono => self.input.to_string(),
            dspd_api::Width::Stereo => format!("{}+{}", self.input, self.input + 1),
        }
    }

    pub fn set_param(&self, stage_id: u32, param_id: u32, value: f64, cx: &mut App) {
        let Some(stage) = self.chain.iter().find(|s| s.read(cx).stage_id == stage_id) else {
            return;
        };

        stage.update(cx, |stage, cx| stage.set_param(param_id, value, cx));
    }

    pub fn insert_stage(
        &mut self,
        stage: StageSpec,
        stage_id: u32,
        position: usize,
        cx: &mut Context<Self>,
    ) {
        let new_stage = Stage {
            meters: self.meters.clone(),
            registry: self.registry.clone(),
            spec: stage,
            stage_id,
            strip_idx: self.strip_idx,
        };

        self.chain.insert(position, cx.new(|_| new_stage));
        cx.notify();
    }

    pub fn remove_stage(&mut self, stage_id: u32, cx: &mut Context<Self>) {
        self.chain
            .retain(|stage| stage.read(cx).stage_id != stage_id);
        cx.notify();
    }

    pub fn move_stage(&mut self, stage_id: u32, new_position: usize, cx: &mut Context<Self>) {
        if let Some(pos) = self
            .chain
            .iter()
            .position(|s| s.read(cx).stage_id == stage_id)
        {
            let stage = self.chain.remove(pos);
            self.chain.insert(new_position, stage);
            cx.notify();
        }
    }

    pub fn set_stage_order(&mut self, order: Vec<u32>, cx: &mut Context<Self>) {
        let mut new_chain = Vec::with_capacity(self.chain.len());

        for stage_id in order {
            if let Some(pos) = self
                .chain
                .iter()
                .position(|s| s.read(cx).stage_id == stage_id)
            {
                let stage = self.chain.remove(pos);
                new_chain.push(stage);
            }
        }

        // Append any stages that were not in the order list to the end of the chain
        new_chain.append(&mut self.chain);

        self.chain = new_chain;
        cx.notify();
    }

    pub fn stages(&self) -> impl Iterator<Item = &Entity<Stage>> {
        self.chain.iter()
    }

    pub fn stages_without_io_meters<'a>(
        &'a self,
        cx: &'a App,
    ) -> impl Iterator<Item = &'a Entity<Stage>> {
        // skip the first element if it's a peak_meter (input level meter)
        // and skip the last element if it's a peak_meter

        self.chain
            .iter()
            .enumerate()
            .filter(move |(idx, stage)| {
                let stage = stage.read(cx);
                let is_first = *idx == 0;
                let is_last = *idx == self.chain.len() - 1;

                if is_first && stage.is_native_plugin("peak_meter") {
                    return false;
                }

                if is_last && stage.is_native_plugin("peak_meter") {
                    return false;
                }

                true
            })
            .map(|(_, stage)| stage)
    }
}

impl Strip {
    pub fn from_serializable(
        strip_idx: usize,
        strip: SerializableStrip,
        meters: WeakEntity<MetersState>,
        registry: WeakEntity<Registry>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            meters: meters.clone(),
            registry: registry.clone(),
            strip_idx,
            name: SharedString::from(strip.name),
            input: strip.input,
            width: strip.width,
            chain: strip
                .chain
                .into_iter()
                .map(|stage| {
                    cx.new(|cx| {
                        Stage::from_serializable(
                            strip_idx,
                            stage,
                            meters.clone(),
                            registry.clone(),
                            cx,
                        )
                    })
                })
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

    // ref to the registry entity, so we can query the registry for this stage
    registry: WeakEntity<Registry>,

    /// The strip idx in the strip list of a session
    strip_idx: usize,

    stage_id: u32,
    spec: dspd_api::StageSpec,
}

impl Stage {
    pub fn id_str(&self) -> &str {
        match &self.spec {
            dspd_api::StageSpec::Native(native_spec) => native_spec.plugin.as_str(),
        }
    }

    pub fn name<'a>(&'a self, cx: &'a App) -> Option<&'a str> {
        match &self.spec {
            dspd_api::StageSpec::Native(native_spec) => {
                let registry = self
                    .registry
                    .upgrade()
                    .expect("Registry entity has been dropped")
                    .read(cx);

                registry
                    .get_plugin(&native_spec.plugin)
                    .map(|desc| desc.name.as_str())
            }
        }
    }

    pub fn is_native_plugin(&self, plugin_name: &str) -> bool {
        match &self.spec {
            dspd_api::StageSpec::Native(native_spec) => native_spec.plugin == plugin_name,
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

    pub fn set_param(&mut self, param_id: u32, value: f64, cx: &mut Context<Self>) {
        match &mut self.spec {
            dspd_api::StageSpec::Native(native_spec) => {
                native_spec.settings.insert(param_id, value);
            }
        }

        cx.notify();
    }
}

impl Stage {
    pub fn from_serializable(
        strip_idx: usize,
        stage: SerializableStage,
        meters: WeakEntity<MetersState>,
        registry: WeakEntity<Registry>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            meters,
            registry,
            strip_idx,
            stage_id: stage.id,
            spec: stage.spec,
        }
    }
}
