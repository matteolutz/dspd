use std::f32::consts::PI;

use gpui_kit::{
    canvas, component::ActiveTheme, div, fill, point, px, App, Context, Entity, ParentElement,
    PathBuilder, Render, Styled,
};

use crate::state::Stage;

fn freq_to_x(f: f32, w: f32) -> f32 {
    (f.ln() - 20f32.ln()) / (20000f32.ln() - 20f32.ln()) * w
}
fn x_to_freq(x: f32, w: f32) -> f32 {
    (20f32.ln() + x / w * (20000f32.ln() - 20f32.ln())).exp()
}
fn db_to_y(db: f32, h: f32, range: f32) -> f32 {
    h * (0.5 - db / (2.0 * range))
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

pub struct DynEqEditor {
    stage: Entity<Stage>,
}

impl DynEqEditor {
    pub fn new(stage: &Entity<Stage>, cx: &mut Context<Self>) -> Self {
        assert_eq!(
            stage.read(cx).id_str(),
            "dyn_eq",
            "Stage must be a DynEq stage. It has id_str() = {}",
            stage.read(cx).id_str()
        );

        Self {
            stage: stage.clone(),
        }
    }
}

impl Render for DynEqEditor {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let points = curve_points(&self.stage, 48_000.0, cx);

        div()
            .size_full()
            .border_1()
            .border_color(cx.theme().secondary)
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .p_2()
            .child(
                canvas(
                    move |bounds, _window, _cx| bounds,                 // prepaint: stash bounds for hit-testing
                    move |bounds, _bounds, window, cx| {
                        let w: f32 = bounds.size.width.into();   // Pixels -> f32
                        let h: f32 = bounds.size.height.into();

                        // background + grid
                        window.paint_quad(fill(bounds, cx.theme().border));
                        for f in [30., 50., 100., 200., 500., 1e3, 2e3, 5e3, 1e4] { /* thin vertical line */ }
                        for db in [-18., -12., -6., 0., 6., 12., 18.] { /* thin horizontal line */ }

                        // total curve
                        let mut pb = PathBuilder::stroke(px(2.0));
                        for (i, &(f, db)) in points.iter().enumerate() {
                            let p = point(bounds.left() + px(freq_to_x(f, w)), bounds.top() + px(db_to_y(db, h, 24.0)));
                            if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
                        }
                        if let Ok(path) = pb.build() { window.paint_path(path, cx.theme().primary); }

                        // optional: fill under the curve to the 0 dB line using PathBuilder::fill()
                        // nodes

                        // for band in 0..6 { window.paint_quad(circle_quad(node_pos(band), r, color[band], selected)); }
                    },
                )
                .size_full()
            )
    }
}
