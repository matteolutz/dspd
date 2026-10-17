use gpui_kit::{
    component::{green_500, red_500, ActiveTheme},
    div,
    prelude::FluentBuilder,
    DefiniteLength, Hsla, IntoElement, ParentElement, RenderOnce, Styled,
};

use crate::components::meters::{db_to_linear, linear_to_db, render_db_value};

#[derive(Debug, Copy, Clone)]
pub enum LevelMeterValue {
    Linear(f32),
    Decibels(f32),
}

impl LevelMeterValue {
    fn to_linear(&self) -> f32 {
        match *self {
            Self::Linear(linear) => linear,
            Self::Decibels(db) => db_to_linear(db as f64),
        }
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub enum LevelMeterMode {
    #[default]
    Gain,
    GainReduction,
}

impl LevelMeterMode {
    fn is_meter_upwards(&self) -> bool {
        match self {
            Self::Gain => true,
            Self::GainReduction => false,
        }
    }

    fn meter_color(&self) -> Hsla {
        match self {
            Self::Gain => green_500(),
            Self::GainReduction => red_500(),
        }
    }
}

#[derive(IntoElement)]
pub struct LevelMeter {
    value: LevelMeterValue,
    mode: LevelMeterMode,

    /// The amount of labels (defaults to 0, so no labels)
    n_labels: usize,
}

impl LevelMeter {
    pub fn new(value: impl Into<LevelMeterValue>) -> Self {
        Self {
            value: value.into(),
            mode: LevelMeterMode::default(),

            n_labels: 0,
        }
    }

    pub fn with_mode(mut self, mode: LevelMeterMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_labels(mut self, n_labels: usize) -> Self {
        self.n_labels = n_labels;
        self
    }
}

impl RenderOnce for LevelMeter {
    fn render(
        self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> impl gpui_kit::prelude::IntoElement {
        let linear_value = self.value.to_linear();

        div()
            .flex()
            .h_full()
            .gap_1()
            .when(self.n_labels > 0, |this| {
                this.child(div().h_full().flex().flex_col().justify_between().children(
                    (0..self.n_labels).map(|label_idx| {
                        let linear_value = label_idx as f32 / (self.n_labels as f32 - 1.0);
                        let db_value = linear_to_db(1.0 - linear_value);

                        div()
                            .text_xs()
                            .text_right()
                            .child(render_db_value(db_value))
                    }),
                ))
            })
            .child(
                div()
                    .w_2()
                    .h_full()
                    .bg(cx.theme().accent)
                    .flex()
                    .when_else(
                        self.mode.is_meter_upwards(),
                        |div| div.items_end(),
                        |div| div.items_start(),
                    )
                    .child(
                        div()
                            .bg(self.mode.meter_color())
                            .w_full()
                            .h(DefiniteLength::Fraction(linear_value)),
                    ),
            )
    }
}
