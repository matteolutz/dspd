use std::f32::consts::PI;

use gpui_kit::{
    base::{InteractiveElementExt, StyledExt},
    canvas,
    component::ActiveTheme,
    div, fill, point,
    prelude::FluentBuilder,
    px, rems, size, Anchor, App, AppContext, Bounds, Context, CursorStyle, DispatchPhase, Entity,
    Font, Hsla, InteractiveElement, MouseButton, MouseMoveEvent, ParentElement, PathBuilder,
    Pixels, Render, ScrollWheelEvent, SharedString, Styled, Subscription, Window,
};
use serde_json::json;

use crate::{
    components::mixing::{Knob, KnobEvent, MixingComponentValue},
    context::GpuiContextExtension,
    state::{GetParamUnitScaling, MetersStateContextExt, RequestBuilderExt, Stage, UiState},
    window::WindowExt,
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

struct DynEqCurvePoint {
    freq: f32,
    static_gain: f32,
    range: f32,
    gr: f32,
}

const N: usize = 256;

fn curve_points(
    bands: &[DynEqBand],
    gain_reduction: &[f32],
    fs: f32,
    cx: &App,
) -> Vec<DynEqCurvePoint> {
    assert!(bands.len() == gain_reduction.len());

    // read each band once, not once per curve point
    let band_values = bands
        .iter()
        .zip(gain_reduction)
        .map(|(band, &gr)| {
            let params = band.params.read(cx).display_value();
            (params.freq, params.q, params.gain, params.range, gr)
        })
        .collect::<Vec<_>>();

    (0..N)
        .map(|i| {
            let t = i as f32 / (N - 1) as f32;
            let f = 20.0 * (20000.0f32 / 20.0).powf(t);

            let mut total_static_gain = 0.0;
            let mut total_range = 0.0;
            let mut total_gr = 0.0;

            for &(f0, q, static_gain, range, gr) in &band_values {
                if static_gain.abs() >= 1e-3 {
                    let db = peaking_mag_db(f, f0, q, static_gain, fs);
                    if db.is_finite() {
                        total_static_gain += db;
                    }
                }

                if range.abs() >= 1e-3 {
                    let db = peaking_mag_db(f, f0, q, range, fs);
                    if db.is_finite() {
                        total_range += db;
                    }
                }

                if gr.abs() >= 1e-3 {
                    let db = peaking_mag_db(f, f0, q, gr, fs);
                    if db.is_finite() {
                        total_gr += db;
                    }
                }
            }

            DynEqCurvePoint {
                freq: f,
                static_gain: total_static_gain,
                range: total_range,
                gr: total_gr,
            }
        })
        .collect()
}

const DB_RANGE: f32 = 24.0;

#[derive(Debug, Clone)]
struct DynEqBandParams {
    freq: f32,
    q: f32,
    gain: f32,

    threshold_db: f32,
    ratio: f32,
    range: f32,

    attack_ms: f32,
    release_ms: f32,
}

impl DynEqBandParams {
    fn from_stage(stage: &Stage, band: usize) -> Self {
        let base = (band * 8) as u32;
        let freq = stage.get_param(base).unwrap_or_else(|| match band {
            0 => 60.0,
            1 => 180.0,
            2 => 500.0,
            3 => 1500.0,
            4 => 4000.0,
            5 => 10000.0,
            _ => 1000.0,
        }) as f32;
        let q = stage.get_param(base + 1).unwrap_or(1.0) as f32;
        let gain = stage.get_param(base + 2).unwrap_or(0.0) as f32;
        let threshold_db = stage.get_param(base + 3).unwrap_or(0.0) as f32;
        let ratio = stage.get_param(base + 4).unwrap_or(1.0) as f32;
        let range = stage.get_param(base + 5).unwrap_or(0.0) as f32;
        let attack_ms = stage.get_param(base + 6).unwrap_or(30.0) as f32;
        let release_ms = stage.get_param(base + 7).unwrap_or(100.0) as f32;

        Self {
            freq: freq.clamp(20.0, 20_000.0),
            q: q.clamp(0.1, 18.0),
            gain: gain.clamp(-24.0, 24.0),
            threshold_db: threshold_db.clamp(-90.0, 0.0),
            ratio: ratio.clamp(1.0, 20.0),
            range: range.clamp(-24.0, 24.0),
            attack_ms: attack_ms.clamp(0.1, 500.0),
            release_ms: release_ms.clamp(0.1, 2000.0),
        }
    }
}

struct DynEqKnob {
    knob: Entity<Knob>,
    display_name: SharedString,
    _subs: Vec<Subscription>,
}

impl DynEqKnob {
    pub fn new(
        stage: &Entity<Stage>,
        param_id: u32,
        display_name: impl Into<SharedString>,
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
        let param_default = param.default as f32;
        let param_unit = param.unit;
        let band_color = BAND_COLORS[param_id as usize / 8];

        let knob = cx.new(move |cx| {
            Knob::new_derived(
                &params,
                move |params, _| {
                    param_unit
                        .scaling()
                        .to_normalized(getter(params), param_min, param_max)
                },
                move |params, val, _| {
                    let val = param_unit.scaling().to_scaled(val, param_min, param_max);
                    setter(params, val)
                },
                window,
                cx,
            )
            .with_unit_range(param_unit, param_min, param_max, 1, window, cx)
            .with_default_value(param_unit.scaling().to_normalized(
                param_default,
                param_min,
                param_max,
            ))
            .with_accent_color(band_color)
            .with_size(rems(4.0))
            .with_inset(px(6.0))
        });

        let strip_idx = stage.read(cx).strip_idx();
        let stage_id = stage.read(cx).stage_id();

        let _subs = vec![cx.subscribe(&knob, move |_, _, evt, cx| match evt {
            &KnobEvent::ValueChanged(value) => {
                let value = param_unit.scaling().to_scaled(value, param_min, param_max);
                // let value = map_range(value, 0.0, 1.0, param_min, param_max);

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

        Self {
            knob,
            display_name: display_name.into(),
            _subs,
        }
    }
}

struct DynEqBand {
    params: Entity<MixingComponentValue<DynEqBandParams>>,
    knobs: Vec<DynEqKnob>,
}

pub struct DynEqEditor {
    stage: Entity<Stage>,

    bands: [DynEqBand; 6],
    gain_reduction: [f32; 6],

    canvas_bounds: Option<Bounds<Pixels>>,

    dragged_band: Option<usize>,
    selected_band: Option<usize>,

    _subs: Vec<Subscription>,
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

        let _subs = vec![cx.subscribe_meter_values(
            stage.read(cx).get_meter_slots(cx).collect::<Vec<_>>(),
            |this, cx| {
                let values = this
                    .stage
                    .read(cx)
                    .get_meters(cx)
                    .map(|(_, _, _, _, value)| value.unwrap_or(0.0))
                    .collect::<Vec<_>>();

                if let Ok(values) = values.try_into() {
                    this.gain_reduction = values;
                    cx.notify();
                }
            },
        )];

        Self {
            stage: stage.clone(),

            bands,
            gain_reduction: [0.0; 6],

            canvas_bounds: None,

            dragged_band: None,
            selected_band: None,

            _subs,
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
                "Freq (Hz)",
                &params,
                |params| params.freq,
                |params, freq| params.freq = freq,
                window,
                cx,
            );
            let q_knob = DynEqKnob::new(
                stage,
                (band * 8 + 1) as u32,
                "Q",
                &params,
                |params| params.q,
                |params, q| params.q = q,
                window,
                cx,
            );
            let gain_knob = DynEqKnob::new(
                stage,
                (band * 8 + 2) as u32,
                "Gain",
                &params,
                |params| params.gain,
                |params, gain| params.gain = gain,
                window,
                cx,
            );
            let threshold_knob = DynEqKnob::new(
                stage,
                (band * 8 + 3) as u32,
                "Threshold",
                &params,
                |params| params.threshold_db,
                |params, threshold| params.threshold_db = threshold,
                window,
                cx,
            );
            let ratio_knob = DynEqKnob::new(
                stage,
                (band * 8 + 4) as u32,
                "Ratio",
                &params,
                |params| params.ratio,
                |params, ratio| params.ratio = ratio,
                window,
                cx,
            );
            let range_knob = DynEqKnob::new(
                stage,
                (band * 8 + 5) as u32,
                "Range",
                &params,
                |params| params.range,
                |params, range| params.range = range,
                window,
                cx,
            );
            let attack_knob = DynEqKnob::new(
                stage,
                (band * 8 + 6) as u32,
                "Attack",
                &params,
                |params| params.attack_ms,
                |params, attack| params.attack_ms = attack,
                window,
                cx,
            );
            let release_knob = DynEqKnob::new(
                stage,
                (band * 8 + 6) as u32,
                "Release",
                &params,
                |params| params.release_ms,
                |params, release| params.release_ms = release,
                window,
                cx,
            );

            DynEqBand {
                params,
                knobs: vec![
                    freq_knob,
                    q_knob,
                    gain_knob,
                    threshold_knob,
                    ratio_knob,
                    range_knob,
                    attack_knob,
                    release_knob,
                ],
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
        // TODO: cache this
        let points = curve_points(&self.bands, &self.gain_reduction, 48_000.0, cx);

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
                                    let bounds = Bounds::from_anchor_and_size(
                                        Anchor::TopLeft,
                                        point(x, bounds.top()),
                                        size(px(1.0), px(h)),
                                    );

                                    window.paint_quad(fill(bounds, cx.theme().secondary));

                                    let freq_text = if f < 1000.0 {
                                        format!("{:.0}", f)
                                    } else {
                                        format!("{:.0}k", f / 1000.0)
                                    };

                                    window.paint_simple_text_line(
                                        freq_text,
                                        Font::default(),
                                        px(10.0),
                                        cx.theme().secondary_foreground,
                                        bounds.bottom_right() + point(px(4.0), px(-16.0)),
                                        gpui_kit::TextAlign::Left,
                                        cx,
                                    );
                                }

                                for db in [-18., -12., -6., 0., 6., 12., 18.] {
                                    let y = bounds.top() + px(db_to_y(db, h, DB_RANGE));
                                    let thickness = if db == 0.0 { 3.0 } else { 1.0 };

                                    let bounds = Bounds::from_anchor_and_size(
                                        Anchor::TopLeft,
                                        point(bounds.left(), y),
                                        size(px(w), px(thickness)),
                                    );

                                    window.paint_quad(fill(bounds, cx.theme().secondary));

                                    window.paint_simple_text_line(
                                        format!(
                                            "{}{:.0}",
                                            if db < 0.0 { "-" } else { "+" },
                                            db.abs()
                                        ),
                                        Font::default(),
                                        px(10.0),
                                        cx.theme().secondary_foreground,
                                        bounds.left_center() + point(px(4.0), px(-10.0)),
                                        gpui_kit::TextAlign::Left,
                                        cx,
                                    );
                                }

                                // total curve
                                let mut static_path_outline = PathBuilder::stroke(px(2.0));
                                let mut static_path = PathBuilder::fill();
                                let mut range_path = PathBuilder::stroke(px(2.0));
                                let mut gr_path = PathBuilder::fill();

                                for (i, eq_point) in points.iter().enumerate() {
                                    let static_point = point(
                                        bounds.left() + px(freq_to_x(eq_point.freq, w)),
                                        bounds.top()
                                            + px(db_to_y(eq_point.static_gain, h, DB_RANGE)),
                                    );

                                    let range_point = point(
                                        bounds.left() + px(freq_to_x(eq_point.freq, w)),
                                        bounds.top() + px(db_to_y(eq_point.range, h, DB_RANGE)),
                                    );

                                    let gr_point = point(
                                        bounds.left() + px(freq_to_x(eq_point.freq, w)),
                                        bounds.top() + px(db_to_y(eq_point.gr, h, DB_RANGE)),
                                    );

                                    if i == 0 {
                                        static_path_outline.move_to(static_point);
                                        static_path.move_to(static_point);
                                        range_path.move_to(range_point);
                                        gr_path.move_to(gr_point);
                                    } else {
                                        static_path_outline.line_to(static_point);
                                        static_path.line_to(static_point);
                                        range_path.line_to(range_point);
                                        gr_path.line_to(gr_point);
                                    }
                                }

                                static_path.line_to(point(
                                    bounds.left() + px(freq_to_x(20.0, w)),
                                    bounds.top() + px(db_to_y(0.0, h, DB_RANGE)),
                                ));
                                gr_path.line_to(point(
                                    bounds.left() + px(freq_to_x(20.0, w)),
                                    bounds.top() + px(db_to_y(0.0, h, DB_RANGE)),
                                ));

                                if let Ok(path) = static_path_outline.build() {
                                    window.paint_path(path, cx.theme().primary);
                                }

                                if let Ok(path) = static_path.build() {
                                    window.paint_path(path, cx.theme().primary.alpha(0.5));
                                }

                                if let Ok(path) = range_path.build() {
                                    window.paint_path(path, cx.theme().yellow_light);
                                }

                                if let Ok(path) = gr_path.build() {
                                    window.paint_path(path, cx.theme().yellow_light.alpha(0.5));
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
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .h_64()
                    .bg(cx.theme().background)
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .when_some(self.selected_band, |this, selected_band| {
                        this.children(self.bands[selected_band].knobs.iter().map(|k| {
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .w_20()
                                .child(k.display_name.clone())
                                .child(k.knob.clone())
                        }))
                    }),
            )
    }
}
