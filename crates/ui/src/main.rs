use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::{green_500, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use itertools::Itertools;

use crate::context::GpuiContextExtension;
use crate::state::{EngineStats, Session, UiState};
use crate::tabs::StripsTab;

mod components;
mod context;
mod hsla;
mod state;
mod tabs;
mod theme;

struct HelloWorld {
    stats: Entity<EngineStats>,
    is_up: Entity<bool>,

    session: Entity<Session>,

    // Tabs
    selected_tab: usize,
    strips_tab: Entity<tabs::StripsTab>,

    _subs: Vec<Subscription>,
}

impl HelloWorld {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let stats = UiState::stats(cx);
        let is_up = UiState::is_websocket_connected(cx);
        let session = UiState::session(cx);

        let strips_tab = cx.new(|cx| StripsTab::new(cx));

        let _subs = vec![
            cx.observe_and_notify(&stats),
            cx.observe_and_notify(&is_up),
            cx.observe_and_notify(&session),
        ];

        Self {
            stats,
            is_up,
            session,

            selected_tab: 0,
            strips_tab,

            _subs,
        }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .children(
                                ["Strips", "Rack", "Patch", "Setup"]
                                    .into_iter()
                                    .enumerate()
                                    .with_position()
                                    .map(|(label_pos, (index, label))| {
                                        let entity = cx.entity().downgrade();
                                        Tab::new(index)
                                            .flex_1()
                                            .selected(self.selected_tab == index)
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .when_else(
                                                self.selected_tab == index,
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
                                                _ = entity.update(cx, |this, cx| {
                                                    this.selected_tab = index;
                                                    cx.notify();
                                                });
                                            })
                                            .child(label)
                                    }),
                            ),
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
                            .when(*self.is_up.read(cx), |this| {
                                this.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_color(green_500())
                                        .child(div().rounded_full().size_2().bg(green_500()))
                                        .child("SYNC"),
                                )
                            }),
                    ),
            )
            .child(match self.selected_tab {
                0 => self.strips_tab.clone().into_any_element(),
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
                        "{} strips · ?? plugins",
                        self.session.read(cx).strips().count()
                    )),
            )
    }
}

mod actions {
    use gpui_kit::{App, KeyBinding};

    gpui_kit::actions!(dspd, [Quit]);

    pub fn init(cx: &mut App) {
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        theme::set_theme(cx);

        actions::init(cx);
        tabs::init(cx);

        UiState::init("http://localhost:8080", cx);

        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(Bounds::default())),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|cx| HelloWorld::new(cx)),
        )
        .expect("Failed to open window");
    });
}
