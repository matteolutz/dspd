use gpui_kit::{
    base::StyledExt,
    component::{
        button::{Button, ButtonRounded, ButtonVariants},
        ActiveTheme,
    },
    div,
    prelude::FluentBuilder,
    rems, rgb, App, AppContext, Context, Entity, Hsla, InteractiveElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription,
};
use serde_json::json;

use crate::{
    components::{
        meters::LevelMeter,
        mixing::{Knob, KnobEvent, MixingComponentValue},
    },
    context::GpuiContextExtension,
    state::{tab::UiTab, RequestBuilderExt, Strip, UiState},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelStripColor {
    CanaryYellow,
    Goldenrod,
    Tangerine,
    SignalRed,
    Coral,
    HotPink,
    Orchid,
    ElectricViolet,
    #[default]
    IndigoBlue,
    Azure,
    Cyan,
    Turquoise,
    SeaGreen,
    Emerald,
    LimeGreen,
    Olive,
    SlateBlue,
    SteelGray,
    GraphiteGray,
    WhiteSmoke,
}

impl ChannelStripColor {
    pub fn into_hsla(self) -> Hsla {
        self.into()
    }
}

#[allow(clippy::from_over_into)]
impl Into<Hsla> for ChannelStripColor {
    fn into(self) -> Hsla {
        match self {
            Self::CanaryYellow => rgb(0xEAC93D).into(),
            Self::Goldenrod => rgb(0xDEA42D).into(),
            Self::Tangerine => rgb(0xE6782B).into(),
            Self::SignalRed => rgb(0xDA463B).into(),
            Self::Coral => rgb(0xE26456).into(),
            Self::HotPink => rgb(0xD54A8F).into(),
            Self::Orchid => rgb(0xB75BBA).into(),
            Self::ElectricViolet => rgb(0x8564D5).into(),
            Self::IndigoBlue => rgb(0x556FCD).into(),
            Self::Azure => rgb(0x388FDA).into(),
            Self::Cyan => rgb(0x2EB3C3).into(),
            Self::Turquoise => rgb(0x32B49F).into(),
            Self::SeaGreen => rgb(0x3EA876).into(),
            Self::Emerald => rgb(0x46B55C).into(),
            Self::LimeGreen => rgb(0x84C245).into(),
            Self::Olive => rgb(0x99A33F).into(),
            Self::SlateBlue => rgb(0x6682A3).into(),
            Self::SteelGray => rgb(0x7E8B97).into(),
            Self::GraphiteGray => rgb(0x59626C).into(),
            Self::WhiteSmoke => rgb(0xDDE2E6).into(),
        }
    }
}

pub struct ChannelStrip {
    strip: Entity<Strip>,

    knob: Entity<Knob>,

    _subs: Vec<Subscription>,
}

impl ChannelStrip {
    pub fn new(strip: &Entity<Strip>, cx: &mut Context<Self>) -> Self {
        let gain_stage = strip.read(cx).stages().nth(1).unwrap().clone();

        let knob_value = cx.new(|cx| {
            MixingComponentValue::from_entity_with(
                &gain_stage,
                |s| {
                    let db_value = s.get_param(0).unwrap_or(0.0);
                    ((db_value + 120.0) / 144.0).clamp(0.0, 1.0) as f32
                },
                cx,
            )
        });
        let knob = cx.new(|cx| Knob::new(knob_value, cx));

        let _subs = vec![
            cx.observe_and_notify(strip),
            cx.observe_and_notify(&UiState::tab_state(cx)),
            cx.observe_and_notify(&UiState::meters(cx)),
            cx.subscribe(&knob, |this, _, evt: &KnobEvent, cx| match evt {
                KnobEvent::ValueChanged(value) => {
                    let strip_idx = this.strip.read(cx).strip_idx;
                    let db_value = (value * 144.0) - 120.0;

                    match UiState::client(cx)
                        .post("/params")
                        .content_type_json()
                        .body(
                            json!({
                                "strip": strip_idx,
                                "stage": 1,
                                "param_id": 0,
                                "value": db_value
                            })
                            .to_string(),
                        )
                        .send()
                    {
                        Ok(res) => println!("res: {}", res.text().unwrap()),
                        Err(e) => println!("knob failed: {}", e),
                    }
                }
                _ => {}
            }),
        ];

        Self {
            strip: strip.clone(),
            knob,
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
            .bg(cx.theme().title_bar)
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
        let strip_idx = strip.strip_idx;

        let is_selected = UiState::strip_selection(cx)
            .read(cx)
            .is_strip_selected(strip_idx);

        div()
            .id("strip")
            .on_click(move |_, _, cx| {
                UiState::strip_selection(cx).update(cx, |sel, cx| sel.select_strip(strip_idx, cx))
            })
            .flex_shrink_0()
            .w_24()
            .h_full()
            .when_else(
                is_selected,
                |this| this.border_1().border_t_0().border_color(cx.theme().ring),
                |this| this.border_r_1().border_color(cx.theme().border),
            )
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h_2()
                    .bg(ChannelStripColor::default().into_hsla()),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .py_4()
                    .bg(cx.theme().accent)
                    .line_height(rems(1.0))
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{:0>2}", self.strip.read(cx).input + 1)),
                    )
                    .child(div().text_base().font_semibold().child(strip.name.clone())),
            )
            .child(
                Self::strip_section(cx)
                    .flex()
                    .justify_between()
                    .py_1()
                    .px_2()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "IN {}",
                        self.strip.read(cx).get_input_channel_str()
                    ))
                    .child(match self.strip.read(cx).width {
                        dspd_api::Width::Mono => "M",
                        dspd_api::Width::Stereo => "ST",
                    }),
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
                    .flex()
                    .flex_col()
                    .p_2()
                    .gap(rems(0.3))
                    .children(strip.stages_without_io_meters(cx).enumerate().map(
                        |(stage_idx, stage)| {
                            div()
                                .id(("stage", stage_idx))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    UiState::strip_selection(cx).update(cx, |sel, cx| {
                                        sel.select_strip_and_stage(strip_idx, stage_idx, cx)
                                    });
                                    UiState::tab_state(cx)
                                        .update(cx, |tab, cx| tab.select_tab(UiTab::Rack, cx))
                                })
                                .bg(cx.theme().primary)
                                .text_color(cx.theme().primary_foreground)
                                .text_xs()
                                .font_bold()
                                .w_full()
                                .h_8()
                                .p_2()
                                .rounded(cx.theme().radius * 0.5)
                                .child(
                                    stage
                                        .read(cx)
                                        .name(cx)
                                        .map(|name| name.to_string())
                                        .unwrap_or_else(|| stage.read(cx).id_str().to_string()),
                                )
                        },
                    ))
                    .child(
                        Button::new("add-stage")
                            .secondary()
                            .w_full()
                            .h_8()
                            .p_2()
                            .rounded(ButtonRounded::Small)
                            .text_xs()
                            .child("+"),
                    ),
            ) // plugin list
            .child(
                Self::strip_section(cx)
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(self.knob.clone()),
            )
            .when_some(strip.get_output_level_meters(cx), |this, levels| {
                this.child(
                    Self::strip_section(cx)
                        .border_b_0()
                        .flex()
                        .justify_center()
                        .child(
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
