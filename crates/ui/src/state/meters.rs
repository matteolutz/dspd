use std::ops::Deref;

use dspd_api::MeterInfo;

#[derive(Debug)]
pub struct MeterInfoWithValue {
    info: MeterInfo,
    pub(crate) value: Option<f32>,
}

impl From<MeterInfo> for MeterInfoWithValue {
    fn from(info: MeterInfo) -> Self {
        Self { info, value: None }
    }
}

impl Deref for MeterInfoWithValue {
    type Target = MeterInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[derive(Debug, Default)]
pub struct MetersState {
    meters: Vec<MeterInfoWithValue>,
}

impl Deref for MetersState {
    type Target = [MeterInfoWithValue];

    fn deref(&self) -> &Self::Target {
        &self.meters
    }
}

impl MetersState {
    pub(super) fn update_values(&mut self, values: Vec<f32>) {
        if self.meters.len() != values.len() {
            return;
        }

        for (meter, value) in self.meters.iter_mut().zip(values) {
            meter.value = Some(value);
        }
    }

    pub(super) fn update_layout(&mut self, meters: impl IntoIterator<Item = MeterInfo>) {
        self.meters = meters.into_iter().map(|m| m.into()).collect();
    }

    pub(crate) fn meters_for_stage(
        &self,
        strip_idx: usize,
        stage_id: u32,
    ) -> impl Iterator<Item = (u32, &String, Option<f32>)> {
        self.meters.iter().filter_map(move |m| {
            if m.strip == strip_idx && m.stage == stage_id {
                Some((m.param_id, &m.name, m.value))
            } else {
                None
            }
        })
    }
}
