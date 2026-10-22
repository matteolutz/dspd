use std::ops::Deref;

use gpui_kit::{
    bounds, canvas, component::ActiveTheme, div, fill, point, prelude::FluentBuilder, px, size,
    AbsoluteLength, DefiniteLength, Hsla, IntoElement, ParentElement, RenderOnce, Styled,
};

use crate::components::meters::{db_to_linear, linear_to_db, render_db_value};

#[derive(Debug, Copy, Clone)]
pub enum LevelMeterValue {
    Linear(f32),
    Decibels(f32),
}

impl LevelMeterValue {
    fn to_linear(self) -> f32 {
        match self {
            Self::Linear(linear) => linear,
            Self::Decibels(db) => db_to_linear(db as f64),
        }
    }

    fn to_decibels(self) -> f64 {
        match self {
            Self::Linear(linear) => linear_to_db(linear),
            Self::Decibels(db) => db as f64,
        }
    }
}

impl From<f32> for LevelMeterValue {
    fn from(value: f32) -> Self {
        Self::Decibels(value)
    }
}

/// A color stop for a level meter, indicating the color to use until a certain dB value.
/// ColorStop's should be ordered from lowest to highest dB value
#[derive(Debug, Copy, Clone)]
pub struct LevelMeterColorStop {
    pub color: Hsla,
    pub until_db: f64,
}

const GAIN_COLOR_STOPS: [LevelMeterColorStop; 3] = [
    LevelMeterColorStop {
        until_db: -6.0,
        color: Hsla {
            h: 142.1 / 360.0,
            s: 0.762,
            l: 0.453,
            a: 1.0,
        },
    },
    LevelMeterColorStop {
        until_db: -3.0,
        color: Hsla {
            h: 45.4 / 360.0,
            s: 0.934,
            l: 0.475,
            a: 1.0,
        },
    },
    LevelMeterColorStop {
        until_db: 0.0,
        color: Hsla {
            h: 0.0,
            s: 0.842,
            l: 0.602,
            a: 1.0,
        },
    },
];

const GAIN_REDUCTION_COLOR_STOPS: [LevelMeterColorStop; 1] = [LevelMeterColorStop {
    until_db: 0.0,
    color: Hsla {
        h: 0.0,
        s: 0.842,
        l: 0.602,
        a: 1.0,
    },
}];

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

    fn meter_color_stops(&self) -> &[LevelMeterColorStop] {
        match self {
            Self::Gain => &GAIN_COLOR_STOPS,
            Self::GainReduction => &GAIN_REDUCTION_COLOR_STOPS,
        }
    }
}

pub struct LevelMeterValues(Vec<LevelMeterValue>);

impl<T: Into<LevelMeterValue>, I: IntoIterator<Item = T>> From<I> for LevelMeterValues {
    fn from(value: I) -> Self {
        Self(value.into_iter().map(|v| v.into()).collect())
    }
}

impl Deref for LevelMeterValues {
    type Target = Vec<LevelMeterValue>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(IntoElement)]
pub struct LevelMeter {
    values: LevelMeterValues,
    mode: LevelMeterMode,

    /// The amount of labels (defaults to 0, so no labels)
    n_labels: usize,
    label_text_size: Option<AbsoluteLength>,
}

impl LevelMeter {
    pub fn new(values: impl Into<LevelMeterValues>) -> Self {
        Self {
            values: values.into(),
            mode: LevelMeterMode::default(),

            n_labels: 0,
            label_text_size: None,
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

    pub fn with_label_font_size(mut self, font_size: impl Into<AbsoluteLength>) -> Self {
        self.label_text_size = Some(font_size.into());
        self
    }
}

impl RenderOnce for LevelMeter {
    fn render(
        self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> impl gpui_kit::prelude::IntoElement {
        div()
            .flex()
            .h_full()
            .gap_1()
            .when(self.n_labels > 0, |this| {
                this.child(div().h_full().relative().children((0..self.n_labels).map(
                    |label_idx| {
                        let db_value = if label_idx == self.n_labels - 1 {
                            f64::INFINITY
                        } else {
                            get_nth_label_db(label_idx)
                        };

                        let linear_value = db_to_linear(-db_value);

                        div()
                            .text_xs()
                            .when_some(self.label_text_size, |this, size| this.text_size(size))
                            .text_right()
                            .absolute()
                            .right_0()
                            .w_10()
                            .when_else(
                                linear_value == 0.0,
                                |this| this.bottom_0(), // make sure, the -inf dB label is aligned properly
                                |this| this.top(DefiniteLength::Fraction(1.0 - linear_value)),
                            )
                            .child(render_db_value(-db_value, 0))
                    },
                )))
            })
            .children(self.values.iter().map(|value| {
                let db_value = value.to_decibels();

                div()
                    .w_2()
                    .h_full()
                    .bg(cx.theme().border)
                    .flex()
                    .when_else(
                        self.mode.is_meter_upwards(),
                        |div| div.items_end(),
                        |div| div.items_start(),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |b, _, window, _| {
                                let mut covered_height = px(0.0f32);

                                for color_stop in self.mode.meter_color_stops() {
                                    let new_target_db = color_stop.until_db.min(db_value);
                                    let new_target_linear = db_to_linear(new_target_db);

                                    let rect_height =
                                        b.size.height * new_target_linear - covered_height;

                                    let rect = bounds(
                                        point(b.left(), b.bottom() - rect_height - covered_height),
                                        size(b.size.width, rect_height),
                                    );

                                    window.paint_quad(fill(rect, color_stop.color));

                                    covered_height += rect_height;

                                    if color_stop.until_db >= db_value {
                                        break;
                                    }
                                }
                            },
                        )
                        .size_full(),
                    )
            }))
    }
}

fn get_nth_label_db(n: usize) -> f64 {
    if n == 0 {
        0.0
    } else if n == 1 {
        5.0
    } else {
        10.0 * (n as f64 - 1.0)
    }
}
