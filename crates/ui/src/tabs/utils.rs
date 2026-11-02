use gpui_kit::App;

use crate::state::UiState;

pub fn change_strip_by(by: i8, cx: &mut App) {
    UiState::strip_selection(cx).update(cx, |sel, cx| {
        sel.update_strip(cx, |strip, cx| {
            let n_strips = UiState::strips(cx).count();
            let next_strip = strip
                .map(|s| {
                    s.saturating_add_signed(by as isize)
                        .min(n_strips.saturating_sub(1))
                })
                .unwrap_or(0);

            Some(next_strip)
        })
    })
}

pub fn change_stage_by(by: i8, cx: &mut App) {
    UiState::strip_selection(cx).update(cx, |sel, cx| {
        sel.update_stage(cx, |strip, stage, cx| {
            let Some(strip) = UiState::strip_at(strip, cx) else {
                return None;
            };

            let n_stages = strip.read(cx).stages().count();
            let next_stage = stage
                .map(|s| {
                    s.saturating_add_signed(by as isize)
                        .min(n_stages.saturating_sub(1))
                })
                .unwrap_or(0);

            Some(next_stage)
        })
    })
}

pub fn select_stage(stage_idx: usize, cx: &mut App) {
    UiState::strip_selection(cx).update(cx, |sel, cx| sel.select_stage(stage_idx, cx))
}
