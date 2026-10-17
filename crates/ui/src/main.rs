use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::meters::LevelMeter;
use crate::components::mixing::Knob;
use crate::context::GpuiContextExtension;
use crate::state::{EngineStats, UIState};

mod components;
mod context;
mod state;
mod theme;

struct HelloWorld {
    stats: Entity<EngineStats>,

    knob: Entity<Knob>,

    selected_tab: usize,

    _subs: Vec<Subscription>,
}

impl HelloWorld {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let stats = cx.global::<UIState>().stats();

        let some_value = cx.new(|_| 0.0f32);
        let knob = cx.new(|_| Knob::new("knob", some_value));

        let _subs = vec![
            cx.observe_and_notify(&cx.global::<UIState>().meters()),
            cx.observe_and_notify(&stats),
        ];

        Self {
            stats,
            knob,
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
                    .children(cx.global::<UIState>().strips(cx).map(|strip| {
                        div()
                            .w_20()
                            .p_2()
                            .h_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(strip.name.clone())
                            .child(self.knob.clone())
                            .when_some(strip.get_level_meters(cx), |this, levels| {
                                this.child(
                                    div()
                                        .mt_auto()
                                        .h_80()
                                        .child(LevelMeter::new(levels).with_labels(8)),
                                )
                            })
                    })),
            )
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        theme::set_theme(cx);

        UIState::init(cx);

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
