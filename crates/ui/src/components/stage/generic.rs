use gpui_kit::{
    component::ActiveTheme, div, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription,
};
use serde_json::json;

use crate::{
    components::mixing::{Knob, KnobEvent, MixingComponentValue},
    context::GpuiContextExtension,
    state::{RequestBuilderExt, Stage, UiState},
    utils::map_range,
};

struct GenericStageKnob {
    name: SharedString,
    knob: Entity<Knob>,
    _subs: Vec<Subscription>,
}

pub struct GenericStageEditor {
    stage: Entity<Stage>,
    knobs: Vec<GenericStageKnob>,
}

impl GenericStageEditor {
    pub fn new(stage: &Entity<Stage>, cx: &mut Context<Self>) -> Self {
        let knobs = Self::make_knobs(stage, cx);

        Self {
            stage: stage.clone(),
            knobs,
        }
    }

    fn make_knobs(stage: &Entity<Stage>, cx: &mut Context<Self>) -> Vec<GenericStageKnob> {
        let Some(descriptor) = stage.read(cx).descriptor(cx).cloned() else {
            return vec![];
        };

        descriptor
            .params
            .into_iter()
            .map(|param| {
                let value = cx.new(|cx| {
                    MixingComponentValue::from_entity_with(
                        stage,
                        move |s| {
                            let param_value = s.get_param(param.id).unwrap_or(param.default) as f32;
                            let knob_value = map_range(
                                param_value,
                                param.min as f32,
                                param.max as f32,
                                0.0,
                                1.0,
                            );
                            knob_value
                        },
                        cx,
                    )
                });

                let knob = cx.new(|cx| {
                    Knob::new(value, cx)
                        .on_double_click(move |val| {
                            *val = map_range(
                                param.default as f32,
                                param.min as f32,
                                param.max as f32,
                                0.0,
                                1.0,
                            );
                        })
                        .with_value_formatter(move |val| {
                            let mapped =
                                map_range(val, 0.0, 1.0, param.min as f32, param.max as f32);
                            format!("{:.1}", mapped)
                        })
                });

                let _subs = vec![cx.subscribe_with_entity(
                    &knob,
                    stage.clone(),
                    move |_, _, evt, stage, cx| match evt {
                        KnobEvent::ValueChanged(value) => {
                            let strip_idx = stage.read(cx).strip_idx();
                            let stage_id = stage.read(cx).stage_id();
                            let param_value =
                                map_range(*value, 0.0, 1.0, param.min as f32, param.max as f32);

                            match UiState::client(cx)
                                .post("/params")
                                .content_type_json()
                                .body(
                                    json!({
                                        "strip": strip_idx,
                                        "stage": stage_id,
                                        "param_id": param.id,
                                        "value": param_value
                                    })
                                    .to_string(),
                                )
                                .send()
                            {
                                Ok(_) => {}
                                Err(e) => println!("knob failed: {}", e),
                            }
                        }
                        _ => {}
                    },
                )];

                GenericStageKnob {
                    name: param.name.into(),
                    knob,
                    _subs,
                }
            })
            .collect()
    }
}

impl Render for GenericStageEditor {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let Some(_descriptor) = self.stage.read(cx).descriptor(cx) else {
            return div()
                .flex()
                .justify_center()
                .items_center()
                .border_1()
                .border_color(cx.theme().danger)
                .child(format!(
                    "PluginDescriptor not found for {:?}",
                    self.stage.read(cx).id_str()
                ))
                .into_any_element();
        };

        div()
            .h_full()
            .max_w_full()
            .min_w_40()
            .border_1()
            .border_color(cx.theme().secondary)
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .p_2()
            .child(
                div() // meters
                    .w_full()
                    .flex_1()
                    .py_2()
                    .border_b_2()
                    .border_color(cx.theme().border)
                    .child("Meters"),
            )
            .child(
                div() // knobs
                    .h_2_3()
                    .w_full()
                    .py_2()
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(
                        div()
                            .grid()
                            .grid_cols(u16::min(8, self.knobs.len() as u16))
                            .children(self.knobs.iter().map(|knob| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap_0p5()
                                    .p_0p5()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(knob.knob.clone())
                                    .child(knob.name.clone())
                            })),
                    ),
            )
            .into_any_element()
    }
}
