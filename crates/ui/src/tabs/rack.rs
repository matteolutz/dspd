use gpui_kit::{
    base::StyledExt,
    component::{
        button::{Button, ButtonVariants},
        ActiveTheme,
    },
    div, App, Context, ParentElement, Render, Styled, Subscription,
};

use crate::{components::ChannelStripColor, context::GpuiContextExtension, state::UiState};

mod actions {
    use gpui_kit::{App, KeyBinding};

    #[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
    #[action(namespace = rack, no_json)]
    pub struct ChangeStripBy {
        pub by: i8,
    }

    #[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
    #[action(namespace = rack, no_json)]
    pub struct ChangeStageBy {
        pub by: i8,
    }

    pub const KEY_CONTEXT: &str = "rack";

    pub fn init(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("left", ChangeStripBy { by: -1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("right", ChangeStripBy { by: 1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("up", ChangeStageBy { by: -1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("down", ChangeStageBy { by: 1 }, Some(KEY_CONTEXT)),
        ]);
    }
}

pub(super) fn init(cx: &mut App) {
    actions::init(cx);
}

pub struct RackTab {
    _subs: Vec<Subscription>,
}

impl RackTab {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let _subs = vec![cx.observe_and_notify(&UiState::strip_selection(cx))];

        Self { _subs }
    }
}

impl Render for RackTab {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let strip_selection = *UiState::strip_selection(cx).read(cx);

        let Some(strip) = strip_selection
            .selected_strip()
            .and_then(|s| UiState::strip_at(s, cx))
        else {
            return div();
        };

        div().w_full().flex_1().flex().flex_col().child(
            div() // title bar
                .w_full()
                .h_12()
                .bg(cx.theme().title_bar)
                .border_b_1()
                .border_color(cx.theme().border)
                .flex()
                .gap_2()
                .px_4()
                .child(
                    div() // selected channel
                        .h_full()
                        .items_center()
                        .flex()
                        .gap_1()
                        .child(
                            Button::new("prev-channel")
                                .secondary()
                                .child("<")
                                .text_lg()
                                .mr_12(),
                        )
                        .child(
                            div()
                                .rounded_full()
                                .size_2()
                                .bg(ChannelStripColor::default().into_hsla()),
                        )
                        .child(
                            div()
                                .child(strip.read(cx).name.clone())
                                .font_bold()
                                .text_lg()
                                .truncate(),
                        )
                        .child(
                            Button::new("next-channel")
                                .secondary()
                                .child(">")
                                .text_lg()
                                .ml_12(),
                        ),
                ),
        )
    }
}
