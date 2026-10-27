use gpui_kit::{App, Context, EventEmitter};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StripSelectionEvent {
    StripChanged(Option<usize>),
    StageChanged(Option<usize>),
}

#[derive(Default, Copy, Clone)]
pub struct StripSelection {
    selected_strip: Option<usize>,
    selected_stage: Option<usize>,
}

impl EventEmitter<StripSelectionEvent> for StripSelection {}

impl StripSelection {
    pub fn selected_strip(&self) -> Option<usize> {
        self.selected_strip
    }

    pub fn selected_stage(&self) -> Option<usize> {
        self.selected_stage
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

        self.selected_strip = new_strip;
        self.selected_stage = None;

        cx.notify();
        cx.emit(StripSelectionEvent::StripChanged(new_strip));
        cx.emit(StripSelectionEvent::StageChanged(None));
    }

    pub fn select_stage(&mut self, stage_idx: usize, cx: &mut Context<Self>) {
        if self.selected_stage == Some(stage_idx) {
            return;
        }

        self.selected_stage = Some(stage_idx);

        cx.notify();
        cx.emit(StripSelectionEvent::StageChanged(Some(stage_idx)));
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
