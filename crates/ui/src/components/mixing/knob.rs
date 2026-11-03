use std::rc::Rc;

use gpui_kit::{
    base::{input::InputState, InteractiveElementExt, Popup},
    canvas,
    component::ActiveTheme,
    div,
    prelude::FluentBuilder,
    px, Anchor, AppContext, Context, DispatchPhase, Entity, EventEmitter, InteractiveElement,
    MouseMoveEvent, ParentElement, PathBuilder, Pixels, Point, Render, Styled, Subscription,
    Window,
};

use crate::{components::mixing::MixingComponentValue, context::GpuiContextExtension};

const MOUSE_SENSITIVITY: f32 = 0.005;

pub enum KnobEvent {
    MouseDown,
    MouseUp,
    ValueChanged(f32),
}

pub struct Knob {
    value: Entity<MixingComponentValue<f32>>,
    mouse_down_pos: Option<Point<Pixels>>,

    text_input: Entity<InputState>,
    text_input_shown: bool,

    on_double_click: Option<Rc<dyn Fn(&mut f32)>>,
    value_formatter: Option<Rc<dyn Fn(f32) -> String>>,

    _subs: Vec<Subscription>,
}

impl Knob {
    pub fn new(
        value: Entity<MixingComponentValue<f32>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_input = cx.new(|cx| InputState::new(window, cx));

        let _subs = vec![cx.observe_and_notify(&value)];

        Self {
            value,
            mouse_down_pos: None,
            on_double_click: None,
            value_formatter: None,
            text_input,
            text_input_shown: false,
            _subs,
        }
    }

    pub fn on_double_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&mut f32) + 'static,
    {
        self.on_double_click = Some(Rc::new(handler));
        self
    }

    pub fn with_value_formatter<F>(mut self, formatter: F) -> Self
    where
        F: Fn(f32) -> String + 'static,
    {
        self.value_formatter = Some(Rc::new(formatter));
        self
    }
    fn handle_left_mouse_down(
        &mut self,
        event: &gpui_kit::MouseDownEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.mouse_down_pos = Some(event.position);
        self.value.update(cx, |value, _| value.start_updating());
        cx.emit(KnobEvent::MouseDown);
    }

    fn handle_left_mouse_up(
        &mut self,
        _event: &gpui_kit::MouseUpEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.mouse_down_pos = None;
        self.value.update(cx, |value, _| value.stop_updating());
        cx.emit(KnobEvent::MouseUp);
    }
}

impl Render for Knob {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let value = self.value.read(cx).display_value();
        let rotation = (value * 270.0) - 225.0;

        div()
            .id("knob")
            .on_double_click(cx.listener(|this, _, _, cx| {
                if let Some(handler) = this.on_double_click.as_ref() {
                    let new_value = this.value.update(cx, |value, cx| {
                        value.start_updating();
                        let new_value = value.update_local_value(cx, |local, _| {
                            handler(local);
                            *local
                        });
                        value.stop_updating();

                        new_value
                    });

                    cx.emit(KnobEvent::ValueChanged(new_value));
                    cx.notify();
                }
            }))
            .flex_shrink_0()
            .size_11()
            .rounded_full()
            .bg(cx.theme().border)
            .flex()
            .justify_center()
            .items_center()
            .relative()
            .child(
                Popup::new("knob-value", div().w_full())
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .anchor(Anchor::TopCenter)
                    .offset(-2.25 * window.rem_size())
                    .when(self.mouse_down_pos.is_some(), |this| {
                        this.content(
                            div()
                                .bg(cx.theme().secondary)
                                .border_1()
                                .border_color(cx.theme().secondary_hover)
                                .text_color(cx.theme().secondary_foreground)
                                .p_1()
                                .child(
                                    self.value_formatter
                                        .as_ref()
                                        .map(|f| f(*value))
                                        .unwrap_or_else(|| value.to_string()),
                                ),
                        )
                    }),
            )
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

                                            let new_value = this.value.update(cx, |mv, cx| {
                                                mv.update_local_value(cx, |value, _| {
                                                    let new_value =
                                                        (*value - value_delta).clamp(0.0, 1.0);
                                                    *value = new_value;
                                                    new_value
                                                })
                                            });

                                            this.mouse_down_pos = Some(event.position);

                                            cx.notify();
                                            cx.emit(KnobEvent::ValueChanged(new_value));
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
                    .bg(cx.theme().background)
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
