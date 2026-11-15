use std::{
    any::{Any, TypeId},
    rc::Rc,
};

use dspd_api::ParamUnit;
use gpui_kit::{
    base::{input::InputState, FocusableExt, InteractiveElementExt, Popup},
    canvas,
    component::{input::Input, ActiveTheme},
    div,
    prelude::FluentBuilder,
    px, rems, AbsoluteLength, Anchor, AnyEntity, App, AppContext, ClickEvent, Context,
    DispatchPhase, Entity, EventEmitter, Focusable, InteractiveElement, KeyDownEvent,
    MouseMoveEvent, ParentElement, PathBuilder, Pixels, Point, Render, StatefulInteractiveElement,
    Styled, Subscription, TextRun, Window,
};

use crate::{
    components::mixing::MixingComponentValue, context::GpuiContextExtension,
    state::ParamUnitToString, utils::map_range,
};

const MOUSE_SENSITIVITY: f32 = 0.005;

pub enum KnobEvent {
    MouseDown,
    MouseUp,
    ValueChanged(f32),
}

#[derive(Debug, Clone, Default)]
enum KnobState {
    #[default]
    Default,

    MouseDown(Point<Pixels>),

    TextInput,
}

impl KnobState {
    pub fn is_mouse_down(&self) -> bool {
        matches!(self, KnobState::MouseDown(_))
    }

    pub fn mouse_down_pos(&self) -> Option<Point<Pixels>> {
        match self {
            KnobState::MouseDown(pos) => Some(*pos),
            _ => None,
        }
    }

    pub fn update_mouse_down_pos(&mut self, new_pos: Point<Pixels>) {
        if let KnobState::MouseDown(pos) = self {
            *pos = new_pos;
        }
    }

    pub fn is_text_input(&self) -> bool {
        matches!(self, KnobState::TextInput)
    }
}

#[derive(Default)]
enum KnobValueFormatter {
    #[default]
    Default,

    // TODO: maybe add precision option for formatting the value
    UnitRange {
        unit: ParamUnit,
        min: f32,
        max: f32,
        precision: usize,
    },

    Custom(Rc<dyn Fn(f32) -> String>),
}

impl KnobValueFormatter {
    pub fn format(&self, value: f32) -> String {
        match self {
            KnobValueFormatter::Default => format!("{:.2}", value),
            &KnobValueFormatter::UnitRange {
                unit,
                min,
                max,
                precision,
            } => {
                let scaled_value = map_range(value, 0.0, 1.0, min, max);
                format!("{:.prec$}{}", scaled_value, unit.as_str(), prec = precision)
            }
            KnobValueFormatter::Custom(f) => f(value),
        }
    }

    pub fn text_input_suffix(&self) -> Option<&str> {
        match self {
            KnobValueFormatter::UnitRange { unit, .. } => Some(unit.as_str()),
            _ => None,
        }
    }

    pub fn text_input_value(&self, value: f32) -> String {
        match self {
            &KnobValueFormatter::UnitRange {
                min,
                max,
                precision,
                ..
            } => {
                let scaled_value = map_range(value, 0.0, 1.0, min, max);
                format!("{:.prec$}", scaled_value, prec = precision)
            }
            KnobValueFormatter::Default | KnobValueFormatter::Custom(_) => format!("{:.2}", value),
        }
    }

    pub fn parse_text_input_value(&self, input: &str) -> Option<f32> {
        match self {
            &KnobValueFormatter::UnitRange { min, max, .. } => input
                .parse::<f32>()
                .ok()
                .map(|val| map_range(val, min, max, 0.0, 1.0)),
            KnobValueFormatter::Default | KnobValueFormatter::Custom(_) => {
                input.parse::<f32>().ok().map(|val| val.clamp(0.0, 1.0))
            }
        }
    }
}

pub enum KnobValue {
    Remote(Entity<MixingComponentValue<f32>>),
    Derived {
        entity: AnyEntity,
        getter: Box<dyn Fn(&App) -> f32>,
        setter: Box<dyn Fn(f32, &mut App)>,
        start_updating: Box<dyn Fn(&mut App)>,
        stop_updating: Box<dyn Fn(&mut App)>,
    },
    Local(f32),
}

impl KnobValue {
    pub fn derived<T>(
        value: &Entity<MixingComponentValue<T>>,
        getter: impl Fn(&T, &App) -> f32 + 'static,
        setter: impl Fn(&mut T, f32, &mut App) + 'static,
    ) -> Self
    where
        T: Clone + 'static,
    {
        let entity = value.clone().into_any();

        let getter = Box::new({
            let entity = entity.clone();
            move |cx: &App| {
                let value = entity
                    .clone()
                    .downcast::<MixingComponentValue<T>>()
                    .expect("Failed to downcast entity to MixingComponentValue<T>");
                let value = value.read(cx);
                getter(value.display_value(), cx)
            }
        });

        let setter = Box::new({
            let entity = entity.clone();
            move |new_value: f32, cx: &mut App| {
                let value = entity
                    .clone()
                    .downcast::<MixingComponentValue<T>>()
                    .expect("Failed to downcast entity to MixingComponentValue<T>");
                value.update(cx, |mv, cx| {
                    mv.update_local_value(cx, |value, cx| {
                        setter(value, new_value, cx);
                    });
                });
            }
        });

        let start_updating = Box::new({
            let entity = entity.clone();
            move |cx: &mut App| {
                let value = entity
                    .clone()
                    .downcast::<MixingComponentValue<T>>()
                    .expect("Failed to downcast entity to MixingComponentValue<T>");
                value.update(cx, |mv, _| mv.start_updating());
            }
        });

        let stop_updating = Box::new({
            let entity = entity.clone();
            move |cx: &mut App| {
                let value = entity
                    .clone()
                    .downcast::<MixingComponentValue<T>>()
                    .expect("Failed to downcast entity to MixingComponentValue<T>");
                value.update(cx, |mv, _| mv.stop_updating());
            }
        });

        Self::Derived {
            entity: value.clone().into_any(),
            getter,
            setter,
            start_updating,
            stop_updating,
        }
    }
}

impl From<Entity<MixingComponentValue<f32>>> for KnobValue {
    fn from(entity: Entity<MixingComponentValue<f32>>) -> Self {
        KnobValue::Remote(entity)
    }
}

impl KnobValue {
    pub fn entity(&self) -> Option<&AnyEntity> {
        match self {
            KnobValue::Remote(entity) => Some(entity),
            KnobValue::Derived { entity, .. } => Some(entity),
            KnobValue::Local(_) => None,
        }
    }

    pub fn start_updating(&mut self, cx: &mut App) {
        match self {
            KnobValue::Remote(entity) => entity.update(cx, |value, _| value.start_updating()),
            KnobValue::Derived { start_updating, .. } => start_updating(cx),
            _ => {}
        }
    }

    pub fn stop_updating(&mut self, cx: &mut App) {
        match self {
            KnobValue::Remote(entity) => entity.update(cx, |value, _| value.stop_updating()),
            KnobValue::Derived { stop_updating, .. } => stop_updating(cx),
            _ => {}
        }
    }

    pub fn display_value(&self, cx: &App) -> f32 {
        match self {
            KnobValue::Remote(entity) => *entity.read(cx).display_value(),
            KnobValue::Derived { getter, .. } => {
                let value = getter(cx);
                value
            }
            KnobValue::Local(value) => *value,
        }
    }

    pub fn set_local_value(&mut self, value: f32, cx: &mut App) {
        match self {
            KnobValue::Remote(entity) => entity.update(cx, |mv, cx| mv.set_local_value(value, cx)),
            KnobValue::Derived { setter, .. } => {
                setter(value, cx);
            }
            KnobValue::Local(local_value) => *local_value = value,
        }
    }

    pub fn update_local_value<R>(
        &mut self,
        cx: &mut App,
        update: impl FnOnce(&mut f32, &mut App) -> R,
    ) -> R {
        match self {
            KnobValue::Remote(entity) => {
                entity.update(cx, |mv, cx| mv.update_local_value(cx, update))
            }
            KnobValue::Derived { setter, getter, .. } => {
                let mut value = getter(cx);
                let result = update(&mut value, cx);
                setter(value, cx);
                result
            }
            KnobValue::Local(local_value) => update(local_value, cx),
        }
    }
}

pub struct Knob {
    value: KnobValue,
    state: KnobState,

    text_input: Entity<InputState>,

    on_double_click: Option<Rc<dyn Fn(&mut f32)>>,
    value_formatter: KnobValueFormatter,

    default_value: Option<f32>,

    _subs: Vec<Subscription>,
}

impl Knob {
    pub fn new(
        value: Entity<MixingComponentValue<f32>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value_formatter = KnobValueFormatter::default();

        let text_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value_formatter.text_input_value(*value.read(cx).display_value()))
        });

        let mut _subs = vec![cx.observe_in(&value, window, |this, _, window, cx| {
            this.update_text_input(window, cx);
            cx.notify();
        })];

        Self {
            value: KnobValue::Remote(value),
            state: KnobState::default(),
            on_double_click: None,
            value_formatter,
            default_value: None,
            text_input,
            _subs,
        }
    }

    pub fn new_derived<T>(
        value: &Entity<MixingComponentValue<T>>,
        getter: impl Fn(&T, &App) -> f32 + 'static,
        setter: impl Fn(&mut T, f32, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        T: Clone + 'static,
    {
        let knob_value = KnobValue::derived(&value, getter, setter);
        let value_formatter = KnobValueFormatter::default();

        let text_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value_formatter.text_input_value(knob_value.display_value(cx)))
        });

        let mut _subs = vec![cx.observe_in(&value, window, |this, _, window, cx| {
            this.update_text_input(window, cx);
            cx.notify();
        })];

        Self {
            value: knob_value,
            state: KnobState::default(),
            on_double_click: None,
            value_formatter,
            default_value: None,
            text_input,
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

    pub fn with_unit_range(
        mut self,
        unit: ParamUnit,
        min: f32,
        max: f32,
        precision: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        self.value_formatter = KnobValueFormatter::UnitRange {
            unit,
            min,
            max,
            precision,
        };
        self.update_text_input(window, cx);
        self
    }

    pub fn with_custom_formatter<F>(
        mut self,
        formatter: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        F: Fn(f32) -> String + 'static,
    {
        self.value_formatter = KnobValueFormatter::Custom(Rc::new(formatter));
        self.update_text_input(window, cx);
        self
    }

    pub fn with_default_value(mut self, default_value: f32) -> Self {
        self.default_value = Some(default_value);
        self
    }

    pub fn set_local_value(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.value.set_local_value(value, cx);
        self.update_text_input(window, cx);
    }

    fn update_text_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.text_input.update(cx, |input, cx| {
            input.set_value(
                self.value_formatter
                    .text_input_value(self.value.display_value(cx)),
                window,
                cx,
            );
        });
    }

    fn handle_left_mouse_down(
        &mut self,
        event: &gpui_kit::MouseDownEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.state = KnobState::MouseDown(event.position);
        self.value.start_updating(cx);
        cx.emit(KnobEvent::MouseDown);
    }

    fn handle_left_mouse_up(
        &mut self,
        _event: &gpui_kit::MouseUpEvent,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) {
        self.stop_editing(cx);
        cx.emit(KnobEvent::MouseUp);
    }

    /// Helper function to stop editing the knob, resetting its state and stopping any value updates.
    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.state = KnobState::Default;
        self.value.stop_updating(cx);
    }

    fn formatted_display_value(&self, cx: &App) -> String {
        let value = self.value.display_value(cx);
        self.value_formatter.format(value)
    }

    fn get_text_width(
        text: &str,
        font_size: impl Into<AbsoluteLength>,
        window: &mut Window,
    ) -> Pixels {
        let font_size = font_size.into().to_pixels(window.rem_size());
        let runs = &[TextRun {
            len: text.len(),
            ..Default::default()
        }];

        window
            .text_system()
            .layout_line(text, font_size, runs, None)
            .width
    }
}

impl Render for Knob {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let value = self.value.display_value(cx);

        let is_default = self
            .default_value
            .is_some_and(|default_value| value == default_value);

        let rotation = (value * 270.0) - 225.0;

        div()
            .id("knob")
            .on_key_down(cx.listener(|this, evt: &KeyDownEvent, _, cx| {
                if evt.keystroke.key == "escape" {
                    this.stop_editing(cx);
                    cx.stop_propagation();
                    return;
                }

                if this.state.is_text_input() && evt.keystroke.key == "enter" {
                    if let Some(input_value) = this
                        .value_formatter
                        .parse_text_input_value(this.text_input.read(cx).value().as_str())
                    {
                        this.value.set_local_value(input_value, cx);

                        cx.notify();
                        cx.emit(KnobEvent::ValueChanged(input_value));

                        this.stop_editing(cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_aux_click(cx.listener(|this, evt: &ClickEvent, window, cx| {
                if evt.is_right_click() {
                    this.stop_editing(cx);
                    this.state = KnobState::TextInput;
                    cx.notify();

                    cx.defer_in(window, |this, window, cx| {
                        this.text_input.update(cx, |input, cx| {
                            input.select_all(window, cx);
                        });
                        this.text_input.focus_handle(cx).focus(window, cx);
                    });
                }
            }))
            .on_double_click(cx.listener(|this, _, _, cx| {
                if let Some(handler) = this.on_double_click.as_ref() {
                    let new_value = this.value.update_local_value(cx, |local, cx| {
                        handler(local);
                        *local
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
                    .map(|this| match self.state {
                        KnobState::MouseDown(_) => this.content(
                            div()
                                .bg(cx.theme().secondary)
                                .border_1()
                                .border_color(cx.theme().secondary_hover)
                                .text_color(cx.theme().secondary_foreground)
                                .p_1()
                                .h_6()
                                .flex()
                                .items_center()
                                .child(self.formatted_display_value(cx)),
                        ),
                        KnobState::TextInput => {
                            let text_input_font_size = rems(0.875);
                            let text_input_width = (Self::get_text_width(
                                self.text_input.read(cx).value().as_str(),
                                text_input_font_size,
                                window,
                            ) + px(8.0))
                            .clamp(px(24.0), px(64.0));

                            this.content(
                                div()
                                    .bg(cx.theme().secondary)
                                    .border_1()
                                    .border_color(cx.theme().secondary_hover)
                                    .text_color(cx.theme().secondary_foreground)
                                    .h_6()
                                    .p_0p5()
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .child(
                                        Input::new(&self.text_input)
                                            .text_size(text_input_font_size)
                                            .w(text_input_width)
                                            .appearance(false)
                                            .focus_ring(false)
                                            .p_0()
                                            .text_center(),
                                    )
                                    .when_some(
                                        self.value_formatter.text_input_suffix(),
                                        |this, suffix| this.child(suffix.to_string()),
                                    ),
                            )
                        }
                        KnobState::Default => this,
                    }),
            )
            .child(
                div()
                    .child(
                        canvas(
                            |_, _, _| {},
                            cx.draw_canvas(|this, _, _, window, cx| {
                                if !this.state.is_mouse_down() {
                                    return;
                                };

                                let this = cx.entity();
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if phase != DispatchPhase::Bubble {
                                            return;
                                        }

                                        this.update(cx, |this, cx| {
                                            let Some(mouse_down_pos) = this.state.mouse_down_pos()
                                            else {
                                                return;
                                            };

                                            let pos_delta = event.position - mouse_down_pos;
                                            let delta_y = pos_delta.y;

                                            let is_fine = event.modifiers.alt;
                                            let value_delta = delta_y.as_f32()
                                                * MOUSE_SENSITIVITY
                                                * if is_fine { 0.1 } else { 1.0 };

                                            let new_value =
                                                this.value.update_local_value(cx, |value, _| {
                                                    let new_value =
                                                        (*value - value_delta).clamp(0.0, 1.0);
                                                    *value = new_value;
                                                    new_value
                                                });

                                            this.state.update_mouse_down_pos(event.position);

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

                                let indicator_color = if is_default {
                                    cx.theme().primary
                                } else {
                                    cx.theme().foreground
                                };

                                let mut path_builder = PathBuilder::stroke(px(3.0));
                                path_builder.move_to(indicator_from);
                                path_builder.line_to(indicator_to);
                                let path = path_builder.build().unwrap();
                                window.paint_path(path, indicator_color);
                            },
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
    }
}

impl EventEmitter<KnobEvent> for Knob {}
