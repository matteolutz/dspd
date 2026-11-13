use std::{collections::HashSet, ops::Deref};

use dspd_api::{MeterInfo, MeterType};
use gpui_kit::{Context, EventEmitter, Subscription};

use crate::state::UiState;

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

pub enum MetersStateEvent {
    MeterValuesUpdated { updated_meters: Vec<usize> },
}

#[derive(Debug, Default)]
pub struct MetersState {
    meters: Vec<MeterInfoWithValue>,
}

impl EventEmitter<MetersStateEvent> for MetersState {}

impl MetersState {
    pub(super) fn max_slot(&self) -> Option<usize> {
        self.meters.iter().map(|m| m.slot).max()
    }

    pub(super) fn update_values(&mut self, values: Vec<f32>, cx: &mut Context<Self>) {
        if self
            .max_slot()
            .is_some_and(|max_slot| values.len() != max_slot + 1)
        {
            println!("WARN: values.len() != max_slot + 1");
            return;
        }

        let mut updated_meters = Vec::new();
        for (idx, (new_value, meter)) in self
            .meters
            .iter_mut()
            .map(|meter| (values[meter.slot], meter))
            .enumerate()
        {
            if meter.value.is_some_and(|old_value| old_value == new_value) {
                continue;
            }

            meter.value = Some(new_value);
            updated_meters.push(idx);
        }

        cx.notify();
        cx.emit(MetersStateEvent::MeterValuesUpdated { updated_meters });
    }

    pub(super) fn update_layout(
        &mut self,
        meters: impl IntoIterator<Item = MeterInfo>,
        cx: &mut Context<Self>,
    ) {
        self.meters = meters.into_iter().map(|m| m.into()).collect();
        cx.notify();
    }

    pub(crate) fn meters_for_stage(
        &self,
        strip_idx: usize,
        stage_id: u32,
    ) -> impl Iterator<Item = (usize, u32, &String, MeterType, Option<f32>)> {
        self.meters.iter().filter_map(move |m| {
            if m.strip == strip_idx && m.stage == stage_id {
                Some((m.slot, m.param_id, &m.name, m.r#type, m.value))
            } else {
                None
            }
        })
    }

    pub(crate) fn meter_value(&self, slot: usize) -> Option<f32> {
        self.meters.get(slot).and_then(|m| m.value)
    }
}

pub trait MetersStateContextExt<T> {
    /// Subscribe to the given meter indices. If called with an empty iterator for `meters`
    /// it will execute for any changed meter
    fn subscribe_meter_values(
        &mut self,
        meters: impl IntoIterator<Item = usize>,
        on_changed: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription;
}

impl<'a, T: 'static> MetersStateContextExt<T> for Context<'a, T> {
    fn subscribe_meter_values(
        &mut self,
        meters: impl IntoIterator<Item = usize>,
        mut on_changed: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription {
        let meters = meters.into_iter().collect::<HashSet<_>>();

        self.subscribe(&UiState::meters(self), move |this, _, evt, cx| match evt {
            MetersStateEvent::MeterValuesUpdated { updated_meters }
                if meters.is_empty() || updated_meters.iter().any(|m| meters.contains(m)) =>
            {
                on_changed(this, cx);
            }
            _ => {}
        })
    }
}
