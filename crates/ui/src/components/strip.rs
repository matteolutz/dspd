use gpui_kit::{
    component::ActiveTheme, div, prelude::FluentBuilder, Context, ParentElement, Render, Styled,
};

use crate::{components::meters::LevelMeter, state::UiState};

pub struct ChannelStrip {
    strip_idx: usize,
}

impl ChannelStrip {
    pub fn new(strip_idx: usize, cx: &mut Context<Self>) -> Self {
        Self { strip_idx }
    }
}

impl Render for ChannelStrip {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let Some(strip) = UiState::strip_at(self.strip_idx, cx) else {
            return div();
        };

        div()
            .w_20()
            .p_2()
            .h_full()
            .border_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .items_center()
            .child(strip.name.clone())
            .when_some(strip.get_level_meters(cx), |this, levels| {
                this.child(
                    div()
                        .mt_auto()
                        .h_80()
                        .child(LevelMeter::new(levels).with_labels(8)),
                )
            })
    }
}
