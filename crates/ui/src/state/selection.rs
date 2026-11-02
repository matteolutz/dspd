use gpui_kit::{App, Context, Entity, EventEmitter};
use itertools::Itertools;

use crate::state::{Stage, Strip, UiState};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StripSelectionEvent {
    StripChanged(Option<usize>),
    StageChanged(Option<usize>),
}

#[derive(Debug, Default, Copy, Clone)]
pub struct StripSelection {
    selected_strip: Option<usize>,
    selected_stage: Option<usize>,
}

impl EventEmitter<StripSelectionEvent> for StripSelection {}

impl StripSelection {
    pub fn selected_strip(&self) -> Option<usize> {
        self.selected_strip
    }

    pub fn selected_strip_entity(&self, cx: &App) -> Option<Entity<Strip>> {
        self.selected_strip
            .and_then(|idx| UiState::strip_at(idx, cx))
    }

    pub fn selected_stage(&self) -> Option<usize> {
        self.selected_stage
    }

    pub fn selected_stage_entity(&self, cx: &App) -> Option<Entity<Stage>> {
        self.selected_strip
            .and_then(|idx| UiState::strip_at(idx, cx))
            .zip(self.selected_stage)
            .and_then(|(strip, stage)| {
                strip
                    .read(cx)
                    .stages_without_io_meters(cx)
                    .nth(stage)
                    .cloned()
            })
    }

    pub fn is_strip_selected(&self, strip_idx: usize) -> bool {
        self.selected_strip == Some(strip_idx)
    }

    pub fn select_strip(&mut self, strip_idx: usize, cx: &mut Context<Self>) {
        self.update_strip(cx, |_, _| Some(strip_idx));
    }

    pub fn update_strip(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(Option<usize>, &mut App) -> Option<usize>,
    ) {
        let new_strip = update(self.selected_strip, cx);
        if self.selected_strip == new_strip {
            return;
        }

        let previous_selected_stage = self.selected_stage_entity(cx).map(|s| s.read(cx).id_str());

        self.selected_strip = new_strip;

        if let Some((new_selected_strip, previous_selected_stage)) =
            self.selected_strip_entity(cx).zip(previous_selected_stage)
        {
            self.selected_stage = new_selected_strip
                .read(cx)
                .stages_without_io_meters(cx)
                .find_position(|s| s.read(cx).id_str() == previous_selected_stage)
                .map(|(idx, _)| idx);
        } else {
            self.selected_stage = None;
        }

        cx.notify();
        cx.emit(StripSelectionEvent::StripChanged(new_strip));
        cx.emit(StripSelectionEvent::StageChanged(None));
    }

    pub fn select_stage(&mut self, stage_idx: usize, cx: &mut Context<Self>) {
        self.update_stage(cx, |_, _, _| Some(stage_idx));
    }

    pub fn update_stage(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(usize, Option<usize>, &mut App) -> Option<usize>,
    ) {
        let Some(selected_strip) = self.selected_strip else {
            return;
        };

        let new_stage = update(selected_strip, self.selected_stage, cx);
        if self.selected_stage == new_stage {
            return;
        }

        self.selected_stage = new_stage;

        cx.notify();
        cx.emit(StripSelectionEvent::StageChanged(None));
    }

    pub fn select_strip_and_stage(
        &mut self,
        strip_idx: usize,
        stage_idx: usize,
        cx: &mut Context<Self>,
    ) {
        if self.selected_strip != Some(strip_idx) {
            self.selected_strip = Some(strip_idx);
            self.selected_stage = Some(stage_idx);

            cx.notify();
            cx.emit(StripSelectionEvent::StripChanged(Some(strip_idx)));
            cx.emit(StripSelectionEvent::StageChanged(Some(stage_idx)));
        }

        if self.selected_stage != Some(stage_idx) {
            self.selected_stage = Some(stage_idx);

            cx.notify();
            cx.emit(StripSelectionEvent::StageChanged(Some(stage_idx)));
        }
    }
}
