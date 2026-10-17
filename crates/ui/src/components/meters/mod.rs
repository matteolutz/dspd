mod level;
pub use level::*;

pub(self) fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

pub(self) fn linear_to_db(linear: f32) -> f64 {
    20.0 * (linear as f64).log10()
}

pub(self) fn render_db_value(db: f64) -> String {
    if db.is_infinite() {
        if db.is_sign_positive() {
            "+∞".to_string()
        } else {
            "-∞".to_string()
        }
    } else {
        format!("{}{:.1}", if db.is_sign_positive() { "+" } else { "" }, db)
    }
}
