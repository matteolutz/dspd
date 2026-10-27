use gpui_kit::{
    div, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, ParentElement, Render,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription,
};

use crate::{context::GpuiContextExtension, state::UiState};

mod actions {
    use gpui_kit::{App, KeyBinding};

    #[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
    #[action(namespace = strips, no_json)]
    pub struct ChangeStripBy {
        pub by: i8,
    }

    pub const KEY_CONTEXT: &str = "strips";

    pub fn init(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("left", ChangeStripBy { by: -1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("right", ChangeStripBy { by: 1 }, Some(KEY_CONTEXT)),
        ]);
    }
}

pub(super) fn init(cx: &mut App) {
    actions::init(cx);
}

pub struct StripsTab {
    strips: Vec<Entity<crate::components::ChannelStrip>>,

    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,

    _subs: Vec<Subscription>,
}

impl StripsTab {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let strips = Self::collect_strips(cx);

        let _subs = vec![
            cx.observe(&UiState::session(cx), |this, _, cx| {
                this.strips = Self::collect_strips(cx);
                cx.notify();
            }),
            cx.observe(&UiState::selected_strip(cx), |this, strip, cx| {
                let Some(current_strip) = *strip.read(cx) else {
                    return;
                };

                let ((top_item, _), (bottom_item, _)) = (
                    this.scroll_handle.logical_scroll_top(),
                    this.scroll_handle.logical_scroll_bottom(),
                );

                let is_in_view = current_strip >= top_item && current_strip <= bottom_item;

                if is_in_view {
                    return;
                }

                this.scroll_handle.scroll_to_item(current_strip);
            }),
        ];

        Self {
            strips,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            _subs,
        }
    }
}

impl StripsTab {
    fn collect_strips(cx: &mut App) -> Vec<Entity<crate::components::ChannelStrip>> {
        UiState::strips(cx)
            // we need to collect into a Vec<Entity<_>> first, because we
            // can't borrow cx mutably while iterating over the strips
            .collect::<Vec<_>>()
            .into_iter()
            .map(|strip| cx.new(|cx| crate::components::ChannelStrip::new(&strip, cx)))
            .collect()
    }
}

impl Render for StripsTab {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        div()
            .id("strips")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .track_focus(&self.focus_handle)
            .key_context(actions::KEY_CONTEXT)
            .on_action(|action: &actions::ChangeStripBy, _, cx| {
                UiState::selected_strip(cx).update(cx, |strip, cx| {
                    let n_strips = UiState::strips(cx).count();
                    let next_strip = strip
                        .map(|s| {
                            s.saturating_add_signed(action.by as isize)
                                .min(n_strips.saturating_sub(1))
                        })
                        .unwrap_or(0);

                    *strip = Some(next_strip);
                    cx.notify();
                })
            })
            .w_full()
            .flex_1()
            .flex()
            .children(self.strips.iter().cloned())
    }
}
