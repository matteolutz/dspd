use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

use crate::components::meters::{LevelMeter, LevelMeterMode, LevelMeterValue};

mod components;
mod theme;

struct HelloWorld {
    meter_value: Entity<f32>,
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
            .child("Hello, World!")
            .child(
                Button::new("hello")
                    .primary()
                    .label("Click me")
                    .on_click(|_, _, _| println!("Clicked!")),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .h_40()
                    .child(
                        LevelMeter::new(LevelMeterValue::Decibels(*self.meter_value.read(cx)))
                            .with_labels(7),
                    )
                    .child(
                        LevelMeter::new(LevelMeterValue::Decibels(-10.0))
                            .with_mode(LevelMeterMode::GainReduction)
                            .with_labels(7),
                    ),
            )
    }
}

fn start_socket(meter_value: Entity<f32>, cx: &mut App) {}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        theme::set_theme(cx);

        let meter_value = cx.new(|_| 0.0f32);

        start_socket(meter_value.clone(), cx);

        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(Bounds::default())),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| HelloWorld { meter_value }),
        )
        .expect("Failed to open window");
    });
}
