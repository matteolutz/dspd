use std::{
    any::{Any, TypeId},
    rc::Rc,
};

use gpui_kit::{
    base::Disableable,
    canvas,
    component::{
        button::{Button, ButtonVariants},
        ActiveTheme,
    },
    div,
    prelude::FluentBuilder,
    rems, size, AbsoluteLength, Anchor, App, Bounds, Context, Div, ElementId, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseDownEvent, ParentElement,
    Pixels, Rems, Render, RenderOnce, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window,
};

use crate::components::{utils::ContextExt, window::WindowExt};

struct ListPickerContextState {
    elements: Vec<Rc<dyn AnyListPickerElement>>,
    _element_type: TypeId,
    on_pick: Option<Box<dyn Fn(&dyn AnyListPickerElement, &mut Window, &mut gpui_kit::App)>>,

    trigger_bounds: Bounds<Pixels>,
    trigger_anchor: Anchor,

    list_width: AbsoluteLength,
    list_item_height: AbsoluteLength,
}

pub struct ListPickerContext {
    state: Option<ListPickerContextState>,
    screen_bounds: Option<Bounds<Pixels>>,

    focus_handle: FocusHandle,
    list_scroll_handle: ScrollHandle,
}

impl ListPickerContext {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: None,
            screen_bounds: None,
            focus_handle: cx.focus_handle(),
            list_scroll_handle: ScrollHandle::new(),
        }
    }
}

impl Focusable for ListPickerContext {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ListPickerContext {
    pub fn open_picker<E>(
        elements: Vec<Rc<E>>,
        on_pick: Option<Rc<dyn Fn(&E, &mut Window, &mut gpui_kit::App)>>,
        trigger_bounds: Bounds<Pixels>,
        trigger_anchor: Anchor,
        list_width: AbsoluteLength,
        list_item_height: AbsoluteLength,
        window: &mut Window,
        cx: &mut App,
    ) where
        E: ListPickerElement,
    {
        window
            .window_context(cx)
            .list_picker()
            .update(cx, |this, cx| {
                this.state = Some(ListPickerContextState {
                    elements: elements
                        .into_iter()
                        .map(|e| e as Rc<dyn AnyListPickerElement>)
                        .collect(),
                    _element_type: TypeId::of::<E>(),
                    on_pick: on_pick.map(|f| {
                        Box::new(
                            move |element: &dyn AnyListPickerElement,
                                  window: &mut Window,
                                  cx: &mut gpui_kit::App| {
                                if let Some(e) = element.as_any().downcast_ref::<E>() {
                                    f(e, window, cx);
                                }
                            },
                        )
                            as Box<
                                dyn Fn(&dyn AnyListPickerElement, &mut Window, &mut gpui_kit::App),
                            >
                    }),
                    trigger_bounds,
                    trigger_anchor,
                    list_width,
                    list_item_height,
                });

                cx.notify();
                cx.focus_self(window);
            });
    }
}

impl ListPickerContext {
    fn make_picker_bounds(
        &self,
        window: &Window,
    ) -> Option<(&ListPickerContextState, Bounds<Pixels>)> {
        self.state
            .as_ref()
            .zip(self.screen_bounds)
            .map(|(state, screen_bounds)| {
                let mut initial_picker_bounds = Bounds::from_anchor_and_size(
                    gpui_kit::Anchor::TopLeft,
                    state.trigger_bounds.corner(state.trigger_anchor),
                    size(
                        state.list_width.to_pixels(window.rem_size()),
                        state.list_item_height.to_pixels(window.rem_size()) * state.elements.len(),
                    ),
                );

                // cap the picker size to the screen size
                initial_picker_bounds.size.width = initial_picker_bounds
                    .size
                    .width
                    .min(screen_bounds.size.width);
                initial_picker_bounds.size.height = initial_picker_bounds
                    .size
                    .height
                    .min(screen_bounds.size.height);

                // move the bounds inside the screen
                if initial_picker_bounds.top() < screen_bounds.top() {
                    initial_picker_bounds.origin.y +=
                        screen_bounds.top() - initial_picker_bounds.top();
                }

                if initial_picker_bounds.left() < screen_bounds.left() {
                    initial_picker_bounds.origin.x +=
                        screen_bounds.left() - initial_picker_bounds.left();
                }

                if initial_picker_bounds.bottom() > screen_bounds.bottom() {
                    initial_picker_bounds.origin.y -=
                        initial_picker_bounds.bottom() - screen_bounds.bottom();
                }

                if initial_picker_bounds.right() > screen_bounds.right() {
                    initial_picker_bounds.origin.x -=
                        screen_bounds.right() - initial_picker_bounds.right();
                }

                (state, initial_picker_bounds)
            })
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.state = None;
        cx.notify();
    }

    fn handle_pick(
        &mut self,
        element: &dyn AnyListPickerElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(on_pick) = self.state.as_ref().and_then(|s| s.on_pick.as_ref()) {
            on_pick(element, window, cx);
        }

        self.close(cx);
    }
}

impl Render for ListPickerContext {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let picker_state = self.make_picker_bounds(window);

        div()
            .id("container")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, evt: &KeyDownEvent, _, cx| {
                if evt.keystroke.key == "escape" {
                    this.close(cx);
                }
            }))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .child(cx.bounds(|this, bounds, _, cx| {
                this.screen_bounds = Some(bounds);
                cx.notify();
            }))
            .when_some(picker_state, |this, (state, bounds)| {
                this.child(
                    div()
                        .id("list")
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.close(cx);
                        }))
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .top(bounds.top())
                        .left(bounds.left())
                        .overflow_hidden()
                        .overflow_y_scroll()
                        .track_scroll(&self.list_scroll_handle)
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cx.theme().primary)
                        .flex()
                        .flex_col()
                        .children(state.elements.iter().cloned().enumerate().map(
                            |(idx, entry)| {
                                Button::new(("list-entry", idx))
                                    .w_full()
                                    .h(state.list_item_height)
                                    .ghost()
                                    .label(entry.display_name())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.handle_pick(entry.as_ref(), window, cx);
                                    }))
                            },
                        )),
                )
            })
    }
}

#[derive(Default)]
struct ListPickerState {
    bounds: Option<Bounds<Pixels>>,
}

pub trait ListPickerElement: 'static {
    type Id;

    fn display_name(&self) -> SharedString;
    fn id(&self) -> Self::Id;
}

impl ListPickerElement for SharedString {
    type Id = SharedString;

    fn display_name(&self) -> SharedString {
        self.clone()
    }

    fn id(&self) -> Self::Id {
        self.clone()
    }
}

trait AnyListPickerElement {
    fn display_name(&self) -> SharedString;
    fn as_any(&self) -> &dyn Any;
}

impl<E: ListPickerElement> AnyListPickerElement for E {
    fn display_name(&self) -> SharedString {
        self.display_name()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(IntoElement)]
pub struct ListPicker<E: ListPickerElement> {
    id: ElementId,
    base: Stateful<Div>,

    disabled: bool,

    elements: Vec<Rc<E>>,
    on_pick: Option<Rc<dyn Fn(&E, &mut Window, &mut gpui_kit::App)>>,

    trigger_anchor: Anchor,
    list_width: Option<AbsoluteLength>,
    list_item_height: AbsoluteLength,
}

impl<E: ListPickerElement> Styled for ListPicker<E> {
    #[doc = " Returns a reference to the style memory of this element."]
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl<E: ListPickerElement> ParentElement for ListPicker<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui_kit::AnyElement>) {
        self.base.extend(elements);
    }
}

impl<E: ListPickerElement> InteractiveElement for ListPicker<E> {
    fn interactivity(&mut self) -> &mut gpui_kit::Interactivity {
        self.base.interactivity()
    }
}

impl<E: ListPickerElement> Disableable for ListPicker<E> {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<E: ListPickerElement> ListPicker<E> {
    pub fn new(id: impl Into<ElementId>, elements: impl IntoIterator<Item = E>) -> Self {
        let id = id.into();

        Self {
            id: id.clone(),
            base: div().id(id),
            elements: elements.into_iter().map(Rc::new).collect(),
            on_pick: None,
            trigger_anchor: Anchor::BottomLeft,
            list_width: None,
            list_item_height: rems(2.0).into(),
            disabled: false,
        }
    }

    pub fn on_pick(
        mut self,
        callback: impl Fn(&E, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> Self {
        self.on_pick = Some(Rc::new(callback));
        self
    }

    /// Sets the anchor point of the trigger bounds to use
    /// when positioning the top-left corner of the picker.
    /// Defaults to `Anchor::BottomLeft`.
    pub fn trigger_anchor(mut self, anchor: Anchor) -> Self {
        self.trigger_anchor = anchor;
        self
    }

    /// Sets the width of the list picker. If not set, the picker will use the width
    /// of the trigger bounds. If set, the picker will use the specified width.
    pub fn list_width(mut self, width: impl Into<AbsoluteLength>) -> Self {
        self.list_width = Some(width.into());
        self
    }

    /// Sets the list picker to automatically use the width of the trigger bounds.
    pub fn list_width_auto(mut self) -> Self {
        self.list_width = None;
        self
    }

    /// Sets the height of each list item in the picker. Defaults to 2 rems.
    pub fn list_item_height(mut self, height: impl Into<AbsoluteLength>) -> Self {
        self.list_item_height = height.into();
        self
    }
}

impl<E: ListPickerElement> RenderOnce for ListPicker<E> {
    fn render(
        self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> impl gpui_kit::prelude::IntoElement {
        let state = window.use_keyed_state(self.id, cx, |_, _| ListPickerState::default());

        self.base
            .relative()
            .when(!self.disabled, |this| {
                this.on_click({
                    let state = state.clone();
                    move |_, window, cx| {
                        if let Some(bounds) = state.read(cx).bounds {
                            ListPickerContext::open_picker(
                                self.elements.clone(),
                                self.on_pick.clone(),
                                bounds,
                                self.trigger_anchor,
                                self.list_width.unwrap_or(bounds.size.width.into()),
                                self.list_item_height,
                                window,
                                cx,
                            );
                        }
                    }
                })
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        state.update(cx, |state, cx| {
                            state.bounds = Some(bounds);
                            cx.notify();
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}
