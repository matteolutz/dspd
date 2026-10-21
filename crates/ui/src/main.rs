use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::ChannelStrip;
use crate::context::GpuiContextExtension;
use crate::state::{EngineStats, UiState};

mod components;
mod context;
mod state;
mod theme;

struct HelloWorld {
    stats: Entity<EngineStats>,

    strips: Vec<Entity<components::ChannelStrip>>,
    selected_tab: usize,

    _subs: Vec<Subscription>,
}

impl HelloWorld {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let stats = UiState::stats(cx);

        // kinda dumb to collect and then map, but this is the easiest way to get a Vec of entities
        let strips = UiState::strips(cx)
            .collect::<Vec<_>>()
            .into_iter()
            .map(|strip| cx.new(|cx| ChannelStrip::new(&strip, cx)))
            .collect();

        let _subs = vec![
            cx.observe_and_notify(&UiState::meters(cx)),
            cx.observe_and_notify(&stats),
        ];

        Self {
            stats,
            strips,
            selected_tab: 0,
            _subs,
        }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_2()
            .child(
                div()
                    .flex()
                    .w_full()
                    .h_24()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1())
                    .p_1()
                    .child(
                        Tabs::new("tabs")
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_md()
                            .flex_1()
                            .h_full()
                            .flex()
                            .bg(cx.theme().accent)
                            .overflow_hidden()
                            .children(
                                ["Strips", "Rack", "Patch", "Setup"]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, label)| {
                                        let entity = cx.entity().downgrade();
                                        Tab::new(index)
                                            .flex_1()
                                            .selected(self.selected_tab == index)
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .when(self.selected_tab == index, |this| {
                                                this.font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .bg(cx.theme().primary)
                                            })
                                            .on_click(move |_, _, cx| {
                                                _ = entity.update(cx, |this, cx| {
                                                    this.selected_tab = index;
                                                    cx.notify();
                                                });
                                            })
                                            .py_2()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .size_full()
                                                    .flex()
                                                    .justify_center()
                                                    .items_center()
                                                    .child(label)
                                                    .when(index > 0, |this| {
                                                        this.border_l_2()
                                                            .border_color(cx.theme().foreground)
                                                    }),
                                            )
                                    }),
                            ),
                    )
                    .child(div().flex_1()),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .flex()
                    .children(self.strips.iter().cloned()),
            )
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        theme::set_theme(cx);

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
