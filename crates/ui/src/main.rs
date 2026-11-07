use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::{green_500, red_500, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use itertools::Itertools;

use crate::components::{WindowContext, WindowExt};
use crate::context::GpuiContextExtension;
use crate::state::tab::UiTab;
use crate::state::{EngineStats, Session, UiState};

mod components;
mod context;
mod hsla;
mod state;
mod tabs;
mod theme;
mod utils;

struct HelloWorld {
    stats: Entity<EngineStats>,
    is_up: Entity<bool>,

    session: Entity<Session>,

    // Tabs
    strips_tab: Entity<tabs::StripsTab>,
    rack_tab: Entity<tabs::RackTab>,

    _subs: Vec<Subscription>,
}

impl HelloWorld {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stats = UiState::stats(cx);
        let is_up = UiState::is_websocket_connected(cx);
        let session = UiState::session(cx);

        let _subs = vec![
            cx.observe_and_notify(&stats),
            cx.observe_and_notify(&is_up),
            cx.observe_and_notify(&session),
        ];

        let strips_tab = cx.new(|cx| tabs::StripsTab::new(window, cx));
        let rack_tab = cx.new(|cx| tabs::RackTab::new(window, cx));

        Self {
            stats,
            is_up,
            session,

            strips_tab,
            rack_tab,

            _subs,
        }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_tab = *UiState::tab_state(cx).read(cx);
        let selected_tab_idx: usize = selected_tab.into();

        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .w_full()
                    .h_20()
                    .flex_shrink_0()
                    .py_2()
                    .px_8()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().title_bar)
                    .child(div().flex_1())
                    .child(
                        Tabs::new("tabs")
                            .rounded(cx.theme().radius)
                            .flex_1()
                            .max_w_112()
                            .h_full()
                            .flex()
                            .bg(cx.theme().secondary)
                            .overflow_hidden()
                            .children(UiTab::names().with_position().map(
                                |(label_pos, (index, label))| {
                                    Tab::new(index)
                                        .flex_1()
                                        .selected(selected_tab_idx == index)
                                        .h_full()
                                        .flex()
                                        .items_center()
                                        .when_else(
                                            selected_tab_idx == index,
                                            |this| {
                                                this.font_weight(gpui::FontWeight::BOLD)
                                                    .bg(cx.theme().primary)
                                            },
                                            |this| {
                                                this.hover(|this| {
                                                    this.bg(cx.theme().secondary_hover)
                                                })
                                            },
                                        )
                                        .when(label_pos.is_first(), |this| {
                                            this.rounded_l(cx.theme().radius)
                                        })
                                        .when(label_pos.is_last(), |this| {
                                            this.rounded_r(cx.theme().radius)
                                        })
                                        .on_click(move |_, _, cx| {
                                            UiState::tab_state(cx).update(cx, |tab, cx| {
                                                tab.select_tab_by_idx(index, cx)
                                            });
                                        })
                                        .child(label)
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(&cx.theme().mono_font_family)
                            .text_color(cx.theme().muted_foreground)
                            .flex_1()
                            .flex()
                            .justify_end()
                            .gap_4()
                            .child(
                                div().flex().items_center().gap_1().child("DSP").child(
                                    div().text_color(green_500()).child(format!(
                                        "{:.0}%",
                                        self.stats.read(cx).avg_load_percent
                                    )),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(div().rounded_full().size_2().bg(
                                        if *self.is_up.read(cx) {
                                            green_500()
                                        } else {
                                            red_500()
                                        },
                                    ))
                                    .when_else(
                                        *self.is_up.read(cx),
                                        |this| this.text_color(green_500()).child("SYNC"),
                                        |this| this.text_color(red_500()).child("NO SYNC"),
                                    ),
                            ),
                    ),
            )
            .child(match selected_tab {
                UiTab::Strips => self.strips_tab.clone().into_any_element(),
                UiTab::Rack => self.rack_tab.clone().into_any_element(),
                _ => div()
                    .w_full()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("TODO: this tab")
                    .into_any_element(),
            })
            .child(
                div()
                    .w_full()
                    .h_10()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().title_bar)
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_8()
                    .text_sm()
                    .font_family(&cx.theme().mono_font_family)
                    .text_color(cx.theme().muted_foreground)
                    .child(div())
                    .child(format!(
                        "{} strips · {} plugins",
                        self.session.read(cx).strips().count(),
                        self.session.read(cx).total_stages(cx)
                    )),
            )
            .child(window.window_context(cx).list_picker())
    }
}

mod actions {
    use gpui_kit::{App, KeyBinding};

    use crate::state::{tab::UiTab, UiState};

    gpui_kit::actions!(dspd, [Quit, SwitchStripsRack]);

    pub fn init(cx: &mut App) {
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.bind_keys([KeyBinding::new("tab", SwitchStripsRack, None)]);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &SwitchStripsRack, cx| {
            UiState::tab_state(cx).update(cx, |tab_state, cx| match tab_state {
                UiTab::Strips => tab_state.select_tab(UiTab::Rack, cx),
                UiTab::Rack => tab_state.select_tab(UiTab::Strips, cx),
                _ => {}
            });
        });
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        theme::set_theme(cx);

        actions::init(cx);
        components::init(cx);
        tabs::init(cx);

        UiState::init("http://localhost:8080", cx);

        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(
                    Bounds::maximized(None, cx).inset(px(100.0)),
                )),
                ..Default::default()
            },
            cx,
            |window, cx| {
                WindowContext::init(window, cx);
                cx.new(|cx| HelloWorld::new(window, cx))
            },
        )
        .expect("Failed to open window");
    });
}
