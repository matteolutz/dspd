use gpui_kit::{
    canvas,
    component::{black, ActiveTheme},
    div, px, DispatchPhase, ElementId, EventEmitter, InteractiveElement, MouseMoveEvent,
    ParentElement, PathBuilder, Pixels, Point, Render, Styled,
};

use crate::{components::mixing::MixingComponentValue, context::GpuiContextExtension};

const MOUSE_SENSITIVITY: f32 = 0.005;

pub enum KnobEvent {
    MouseDown,
    MouseUp,
}

pub struct Knob {
    id: ElementId,

    value: MixingComponentValue<f32>,

    mouse_down_pos: Option<Point<Pixels>>,
}

impl Knob {
    pub fn new(id: impl Into<ElementId>, value: impl Into<MixingComponentValue<f32>>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            mouse_down_pos: None,
        }
    }

    fn handle_left_mouse_down(
        &mut self,
        event: &gpui_kit::MouseDownEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.mouse_down_pos = Some(event.position);
        cx.emit(KnobEvent::MouseDown);
    }

    fn handle_left_mouse_up(
        &mut self,
        _event: &gpui_kit::MouseUpEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.mouse_down_pos = None;
        cx.emit(KnobEvent::MouseUp);
    }
}

impl Render for Knob {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let value = *self.value.read(cx);
        let rotation = (value * 270.0) - 225.0;

        div()
            .id(self.id.clone())
            .size_11()
            .rounded_full()
            .bg(black())
            .flex()
            .justify_center()
            .items_center()
            .child(
                div()
                    .child(
                        canvas(
                            |_, _, _| {},
                            cx.draw_canvas(|this, _, _, window, cx| {
                                if this.mouse_down_pos.is_none() {
                                    return;
                                };

                                let this = cx.entity();
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if phase != DispatchPhase::Bubble {
                                            return;
                                        }

                                        this.update(cx, |this, cx| {
                                            let Some(mouse_down_pos) = this.mouse_down_pos else {
                                                return;
                                            };

                                            let pos_delta = event.position - mouse_down_pos;
                                            let delta_y = pos_delta.y;

                                            let is_fine = event.modifiers.alt;
                                            let value_delta = delta_y.as_f32()
                                                * MOUSE_SENSITIVITY
                                                * if is_fine { 0.1 } else { 1.0 };

                                            this.value.update(cx, |value, cx| {
                                                *value = (*value - value_delta).clamp(0.0, 1.0);
                                                cx.notify();
                                            });

                                            this.mouse_down_pos = Some(event.position);
                                            cx.notify();
                                        });
                                    },
                                );
                            }),
                        )
                        .absolute()
                        .size_full(),
                    )
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(Self::handle_left_mouse_down),
                    )
                    .on_mouse_up(
                        gpui_kit::MouseButton::Left,
                        cx.listener(Self::handle_left_mouse_up),
                    )
                    .on_mouse_up_out(
                        gpui_kit::MouseButton::Left,
                        cx.listener(Self::handle_left_mouse_up),
                    )
                    .size_10()
                    .rounded_full()
                    .bg(cx.theme().border)
                    .border_3()
                    .border_color(cx.theme().accent)
                    .flex()
                    .justify_center()
                    .text_xs()
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                let center = bounds.center();
                                let radius = bounds.size.width.min(bounds.size.height) / 2.0;

                                let angle = rotation.to_radians();

                                let indicator_from = Point {
                                    x: center.x + radius * angle.cos(),
                                    y: center.y + radius * angle.sin(),
                                };
                                let indicator_to = (center - indicator_from) * 0.7 + indicator_from;

                                let mut path_builder = PathBuilder::stroke(px(3.0));
                                path_builder.move_to(indicator_from);
                                path_builder.line_to(indicator_to);
                                let path = path_builder.build().unwrap();
                                window.paint_path(path, cx.theme().foreground);
                            },
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
    }
}

impl EventEmitter<KnobEvent> for Knob {}
