use std::{f32::consts::PI, ops::Deref};

use dspd_api::ParamUnit;
use gpui_kit::{
    base::{InteractiveElementExt, StyledExt},
    canvas,
    component::ActiveTheme,
    div, fill, point,
    prelude::FluentBuilder,
    px, size, Anchor, App, AppContext, Bounds, Context, CursorStyle, DispatchPhase, Entity, Hsla,
    InteractiveElement, MouseButton, MouseMoveEvent, ParentElement, PathBuilder, Pixels, Render,
    ScrollWheelEvent, Styled, Subscription, Window,
};
use serde_json::json;

use crate::{
    components::mixing::{Knob, KnobEvent, MixingComponentValue},
    context::GpuiContextExtension,
    state::{RequestBuilderExt, Stage, UiState},
    utils::map_range,
};

const fn band_color(hue: f32) -> Hsla {
    Hsla {
        h: hue / 360.0,
        s: 1.0,
        l: 0.25,
        a: 1.0,
    }
}

const BAND_COLORS: [Hsla; 6] = [
    band_color(0.0),
    band_color(30.0),
    band_color(60.0),
    band_color(120.0),
    band_color(240.0),
    band_color(300.0),
];

fn freq_to_x(f: f32, w: f32) -> f32 {
    (f.ln() - 20f32.ln()) / (20000f32.ln() - 20f32.ln()) * w
}
fn x_to_freq(x: f32, w: f32) -> f32 {
    (20f32.ln() + x / w * (20000f32.ln() - 20f32.ln())).exp()
}
fn db_to_y(db: f32, h: f32, range: f32) -> f32 {
    h * (0.5 - db / (2.0 * range))
} // ±range dB
fn y_to_db(y: f32, h: f32, range: f32) -> f32 {
    (0.5 - y / h) * (2.0 * range)
} // ±range dB

fn peaking_mag_db(f: f32, f0: f32, q: f32, gain_db: f32, fs: f32) -> f32 {
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * f0 / fs;
    let alpha = w0.sin() / (2.0 * q);
    let (b0, b1, b2) = (1.0 + alpha * a, -2.0 * w0.cos(), 1.0 - alpha * a);
    let (a0, a1, a2) = (1.0 + alpha / a, -2.0 * w0.cos(), 1.0 - alpha / a);
    let w = 2.0 * PI * f / fs;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num = ((b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2)).sqrt();
    let den = ((a0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2)).sqrt();
    20.0 * (num / den).log10()
}

const N: usize = 256;

fn curve_points(stage: &Entity<Stage>, fs: f32, cx: &App) -> Vec<(f32, f32)> {
    let stage = stage.read(cx);

    // read each band once, not once per curve point
    let bands: Vec<(f32, f32, f32)> = (0..6)
        .map(|band| {
            let base = band * 8;
            let f0 = stage.get_param(base).unwrap_or(1000.0) as f32;
            let q = stage.get_param(base + 1).unwrap_or(1.0) as f32;
            let gain = stage.get_param(base + 2).unwrap_or(0.0) as f32;
            (
                f0.clamp(20.0, 20_000.0),
                q.clamp(0.1, 18.0),
                gain.clamp(-24.0, 24.0),
            )
        })
        .collect();

    (0..N)
        .map(|i| {
            let t = i as f32 / (N - 1) as f32;
            let f = 20.0 * (20000.0f32 / 20.0).powf(t);

            let mut total_db = 0.0;
            for &(f0, q, gain) in &bands {
                if gain.abs() < 1e-3 {
                    continue; // flat band, contributes 0 dB
                }
                let db = peaking_mag_db(f, f0, q, gain, fs);
                if db.is_finite() {
                    total_db += db;
                }
            }
            (f, total_db)
        })
        .collect()
}

const DB_RANGE: f32 = 24.0;

#[derive(Debug, Clone)]
struct DynEqBandParams {
    freq: f32,
    q: f32,
    gain: f32,
}

impl DynEqBandParams {
    fn from_stage(stage: &Stage, band: usize) -> Self {
        let base = (band * 8) as u32;
        let freq = stage.get_param(base).unwrap_or(1000.0) as f32;
        let q = stage.get_param(base + 1).unwrap_or(1.0) as f32;
        let gain = stage.get_param(base + 2).unwrap_or(0.0) as f32;
        Self {
            freq: freq.clamp(20.0, 20_000.0),
            q: q.clamp(0.1, 18.0),
            gain: gain.clamp(-24.0, 24.0),
        }
    }
}

struct DynEqKnob {
    knob: Entity<Knob>,
    _subs: Vec<Subscription>,
}

impl DynEqKnob {
    pub fn new(
        stage: &Entity<Stage>,
        param_id: u32,
        params: &Entity<MixingComponentValue<DynEqBandParams>>,
        getter: impl Fn(&DynEqBandParams) -> f32 + 'static,
        setter: impl Fn(&mut DynEqBandParams, f32) + 'static,
        window: &mut Window,
        cx: &mut Context<DynEqEditor>,
    ) -> Self {
        let param = UiState::registry(cx)
            .read(cx)
            .get_plugin("dyn_eq")
            .unwrap()
            .find_param(param_id)
            .unwrap();

        let param_min = param.min as f32;
        let param_max = param.max as f32;
        let param_unit = param.unit;

        let knob = cx.new(move |cx| {
            Knob::new_derived(
                &params,
                move |params, _| map_range(getter(params), param_min, param_max, 0.0, 1.0),
                move |params, val, _| {
                    setter(params, map_range(val, 0.0, 1.0, param_min, param_max))
                },
                window,
                cx,
            )
            .with_unit_range(param_unit, param_min, param_max, 1, window, cx)
        });

        let strip_idx = stage.read(cx).strip_idx();
        let stage_id = stage.read(cx).stage_id();

        let _subs = vec![cx.subscribe(&knob, move |_, _, evt, cx| match evt {
            &KnobEvent::ValueChanged(value) => {
                let value = map_range(value, 0.0, 1.0, param_min, param_max);

                match UiState::client(cx)
                    .post("/params")
                    .content_type_json()
                    .body(
                        json!({
                            "strip": strip_idx,
                            "stage": stage_id,
                            "param_id": param_id,
                            "value": value
                        })
                        .to_string(),
                    )
                    .send()
                {
                    Ok(_) => {}
                    Err(e) => println!("sending value failed: {}", e),
                }
            }
            _ => {}
        })];

        Self { knob, _subs }
    }
}

struct DynEqBand {
    params: Entity<MixingComponentValue<DynEqBandParams>>,
    knobs: Vec<DynEqKnob>,
}

pub struct DynEqEditor {
    stage: Entity<Stage>,

    bands: [DynEqBand; 6],

    canvas_bounds: Option<Bounds<Pixels>>,

    dragged_band: Option<usize>,
    selected_band: Option<usize>,
}

impl DynEqEditor {
    pub fn new(stage: &Entity<Stage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        assert_eq!(
            stage.read(cx).id_str(),
            "dyn_eq",
            "Stage must be a DynEq stage. It has id_str() = {}",
            stage.read(cx).id_str()
        );

        let bands = Self::make_bands(stage, window, cx);

        Self {
            stage: stage.clone(),
            bands,
            canvas_bounds: None,

            dragged_band: None,
            selected_band: None,
        }
    }

    fn make_bands(
        stage: &Entity<Stage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> [DynEqBand; 6] {
        std::array::from_fn(|band| {
            let params = cx.new(move |cx| {
                MixingComponentValue::from_entity_with(
                    stage,
                    move |stage| DynEqBandParams::from_stage(stage, band),
                    cx,
                )
            });

            let freq_knob = DynEqKnob::new(
                stage,
                (band * 8) as u32,
                &params,
                |params| params.freq,
                |params, freq| params.freq = freq,
                window,
                cx,
            );
            let q_knob = DynEqKnob::new(
                stage,
                (band * 8 + 1) as u32,
                &params,
                |params| params.q,
                |params, q| params.q = q,
                window,
                cx,
            );
            let gain_knob = DynEqKnob::new(
                stage,
                (band * 8 + 2) as u32,
                &params,
                |params| params.gain,
                |params, gain| params.gain = gain,
                window,
                cx,
            );

            DynEqBand {
                params,
                knobs: vec![freq_knob, q_knob, gain_knob],
            }
        })
    }

    fn start_dragging_band(&mut self, band: usize, cx: &mut Context<Self>) {
        self.dragged_band = Some(band);
        self.selected_band = Some(band);
        self.bands[band]
            .params
            .update(cx, |b, _| b.start_updating());
        cx.notify();
    }

    fn update_band_q(&mut self, band: usize, q_delta: f32, cx: &mut Context<Self>) {
        self.selected_band = Some(band);

        let strip_idx = self.stage.read(cx).strip_idx();
        let stage_id = self.stage.read(cx).stage_id();

        let new_q = self.bands[band].params.update(cx, |b, cx| {
            b.update_local_value(cx, |b, _| {
                let new_q = b.q + q_delta * 0.01; // scale the delta for sensitivity
                b.q = new_q;
                new_q
            })
        });

        match UiState::client(cx)
            .post("/params")
            .content_type_json()
            .body(
                json!({
                    "strip": strip_idx,
                    "stage": stage_id,
                    "param_id": band * 8 + 1,
                    "value": new_q
                })
                .to_string(),
            )
            .send()
        {
            Ok(_) => {}
            Err(e) => println!("sending value failed: {}", e),
        }

        cx.notify();
    }

    fn update_dragged_band(&mut self, freq: f32, gain: f32, cx: &mut Context<Self>) {
        let Some(band) = self.dragged_band else {
            return;
        };

        let strip_idx = self.stage.read(cx).strip_idx();
        let stage_id = self.stage.read(cx).stage_id();

        self.bands[band].params.update(cx, |b, cx| {
            b.update_local_value(cx, |b, _| {
                b.freq = freq.clamp(20.0, 20_000.0);
                b.gain = gain.clamp(-24.0, 24.0);
            })
        });

        match UiState::client(cx)
            .post("/params")
            .content_type_json()
            .body(
                json!({
                    "strip": strip_idx,
                    "stage": stage_id,
                    "param_id": band * 8,
                    "value": freq
                })
                .to_string(),
            )
            .send()
        {
            Ok(_) => {}
            Err(e) => println!("sending value failed: {}", e),
        }

        match UiState::client(cx)
            .post("/params")
            .content_type_json()
            .body(
                json!({
                    "strip": strip_idx,
                    "stage": stage_id,
                    "param_id": band * 8 + 2,
                    "value": gain
                })
                .to_string(),
            )
            .send()
        {
            Ok(_) => {}
            Err(e) => println!("sending value failed: {}", e),
        }

        cx.notify();
    }

    fn stop_dragging_band(&mut self, cx: &mut Context<Self>) {
        let Some(band) = self.dragged_band else {
            return;
        };

        self.dragged_band = None;
        self.bands[band].params.update(cx, |b, _| b.stop_updating());
        cx.notify();
    }
}

impl Render for DynEqEditor {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let points = curve_points(&self.stage, 48_000.0, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .border_1()
                    .border_color(cx.theme().secondary)
                    .bg(cx.theme().background)
                    .flex()
                    .flex_col()
                    .p_2()
                    .relative()
                    .child(
                        canvas(
                            cx.processor(move |this, bounds, _, _cx| {
                                this.canvas_bounds = Some(bounds);
                            }), // prepaint: stash bounds for hit-testing
                            cx.draw_canvas(move |_, bounds, _, window, cx| {
                                let w: f32 = bounds.size.width.into(); // Pixels -> f32
                                let h: f32 = bounds.size.height.into();

                                let this = cx.entity();
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if phase != DispatchPhase::Bubble {
                                            return;
                                        }

                                        this.update(cx, |this, cx| {
                                            let offset_pos = event.position - bounds.origin;
                                            let freq = x_to_freq(offset_pos.x.as_f32(), w);
                                            let gain = y_to_db(offset_pos.y.as_f32(), h, DB_RANGE);

                                            this.update_dragged_band(freq, gain, cx);
                                        });
                                    },
                                );

                                // background + grid
                                window.paint_quad(fill(bounds, cx.theme().border));
                                for f in [30., 50., 100., 200., 500., 1e3, 2e3, 5e3, 1e4] {
                                    let x = bounds.left() + px(freq_to_x(f, w));

                                    window.paint_quad(fill(
                                        Bounds::from_anchor_and_size(
                                            Anchor::TopLeft,
                                            point(x, bounds.top()),
                                            size(px(1.0), px(h)),
                                        ),
                                        cx.theme().secondary,
                                    ));
                                }

                                for db in [-18., -12., -6., 0., 6., 12., 18.] {
                                    let y = bounds.top() + px(db_to_y(db, h, DB_RANGE));
                                    let thickness = if db == 0.0 { 3.0 } else { 1.0 };

                                    window.paint_quad(fill(
                                        Bounds::from_anchor_and_size(
                                            Anchor::TopLeft,
                                            point(bounds.left(), y),
                                            size(px(w), px(thickness)),
                                        ),
                                        cx.theme().secondary,
                                    ));
                                }

                                // total curve
                                let mut pb = PathBuilder::stroke(px(2.0));
                                for (i, &(f, db)) in points.iter().enumerate() {
                                    let p = point(
                                        bounds.left() + px(freq_to_x(f, w)),
                                        bounds.top() + px(db_to_y(db, h, DB_RANGE)),
                                    );
                                    if i == 0 {
                                        pb.move_to(p)
                                    } else {
                                        pb.line_to(p)
                                    }
                                }

                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, cx.theme().primary);
                                }
                            }),
                        )
                        .absolute()
                        .size_full(),
                    )
                    .when_some(self.canvas_bounds, |this, canvas_bounds| {
                        this.child(
                            div()
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.stop_dragging_band(cx);
                                    }),
                                )
                                .on_mouse_up_out(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.stop_dragging_band(cx);
                                    }),
                                )
                                .absolute()
                                .size_full()
                                .children(self.bands.iter().enumerate().map(|(idx, band)| {
                                    let band = band.params.read(cx).display_value();

                                    let pos = point(
                                        px(freq_to_x(band.freq, canvas_bounds.size.width.as_f32())),
                                        px(db_to_y(
                                            band.gain,
                                            canvas_bounds.size.height.as_f32(),
                                            DB_RANGE,
                                        )),
                                    );
                                    let bounds = Bounds::centered_at(pos, size(px(40.0), px(40.0)));

                                    let is_selected = self.selected_band.is_some_and(|b| b == idx);
                                    let color = BAND_COLORS[idx];

                                    div()
                                        .id(("dyn-eq-band", idx))
                                        .cursor(CursorStyle::OpenHand)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                this.start_dragging_band(idx, cx);
                                            }),
                                        )
                                        .on_double_click(cx.listener(move |_, _, _, _| {
                                            // TODO: reset the band
                                        }))
                                        .on_scroll_wheel(cx.listener(
                                            move |this, evt: &ScrollWheelEvent, _, cx| {
                                                let by = evt.delta.pixel_delta(px(0.)).y.as_f32();
                                                this.update_band_q(idx, by, cx);
                                            },
                                        ))
                                        .absolute()
                                        .top(bounds.top())
                                        .left(bounds.left())
                                        .w(bounds.size.width)
                                        .h(bounds.size.height)
                                        .rounded_full()
                                        .border_2()
                                        .border_color(if is_selected {
                                            cx.theme().yellow_light
                                        } else {
                                            color
                                        })
                                        .bg(color.alpha(if is_selected { 1.0 } else { 0.25 }))
                                        .flex()
                                        .justify_center()
                                        .items_center()
                                        .font_bold()
                                        .child(format!("{}", idx + 1))
                                })),
                        )
                    }),
            )
            .child(div().w_full().flex_shrink_0().h_64().debug_red().when_some(
                self.selected_band,
                |this, selected_band| {
                    this.children(
                        self.bands[selected_band]
                            .knobs
                            .iter()
                            .map(|k| k.knob.clone()),
                    )
                },
            ))
    }
}
