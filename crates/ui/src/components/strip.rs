use gpui_kit::{
    base::StyledExt,
    component::{
        button::{Button, ButtonRounded, ButtonVariants},
        ActiveTheme,
    },
    div,
    prelude::FluentBuilder,
    rems, App, Context, Entity, ParentElement, Render, Styled, Subscription,
};

use crate::{components::meters::LevelMeter, context::GpuiContextExtension, state::Strip};

pub struct ChannelStrip {
    strip: Entity<Strip>,
    _subs: Vec<Subscription>,
}

impl ChannelStrip {
    pub fn new(strip: &Entity<Strip>, cx: &mut Context<Self>) -> Self {
        let _subs = vec![cx.observe_and_notify(strip)];

        Self {
            strip: strip.clone(),
            _subs,
        }
    }
}

impl ChannelStrip {
    fn strip_section(cx: &App) -> gpui_kit::Div {
        div()
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().accent)
            .py_2()
            .px_2()
    }
}

impl Render for ChannelStrip {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let strip = self.strip.read(cx);

        div()
            .w_24()
            .h_full()
            .border_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .items_center()
            .child(
                Self::strip_section(cx)
                    .text_center()
                    .child(strip.name.clone()),
            )
            .child(
                Self::strip_section(cx)
                    .flex()
                    .flex_col()
                    .child("Hello")
                    .child("World"),
            )
            .when_some(strip.get_input_level_meters(cx), |this, levels| {
                this.child(
                    Self::strip_section(cx).flex().justify_center().child(
                        div().h_40().child(
                            LevelMeter::new(levels)
                                .with_labels(8)
                                .with_label_font_size(rems(0.5)),
                        ),
                    ),
                )
            })
            .child(
                Self::strip_section(cx)
                    .flex_1()
                    .bg(cx.theme().background)
                    .border_2()
                    .border_color(cx.theme().accent)
                    .flex()
                    .flex_col()
                    .py_2()
                    .px_1()
                    .gap_1()
                    .children(strip.stages_without_io_meters(cx).map(|stage| {
                        div()
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .text_xs()
                            .font_bold()
                            .w_full()
                            .h_10()
                            .p_2()
                            .rounded(cx.theme().radius * 0.5)
                            .child(
                                stage
                                    .read(cx)
                                    .name(cx)
                                    .map(|name| name.to_string())
                                    .unwrap_or_else(|| stage.read(cx).id_str().to_string()),
                            )
                    }))
                    .child(
                        Button::new("add-stage")
                            .secondary()
                            .w_full()
                            .h_10()
                            .p_2()
                            .rounded(ButtonRounded::Small)
                            .child("+"),
                    ),
            ) // plugin list
            .when_some(strip.get_output_level_meters(cx), |this, levels| {
                this.child(
                    Self::strip_section(cx).flex().justify_center().child(
                        div().h_40().child(
                            LevelMeter::new(levels)
                                .with_labels(8)
                                .with_label_font_size(rems(0.5)),
                        ),
                    ),
                )
            })
    }
}
