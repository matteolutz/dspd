use std::f32::consts::TAU;

use crate::{
    dyneq::state::{BiquadState, Coeffs},
    session::DynEqBandSpec,
};

#[derive(Debug)]
pub struct DynamicBand {
    sample_rate: f32,

    // User parameters
    frequency_hz: f32,
    q: f32,
    pub static_gain_db: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub range_db: f32,
    attack_ms: f32,
    release_ms: f32,

    // Filter design, updated when frequency or Q changes
    cos_w0: f32,
    alpha: f32,
    pub(super) detector_coeffs: Coeffs,
    pub(super) eq_coeffs: Coeffs,

    // Per-channel filter state
    pub detector_states: Vec<BiquadState>,
    pub eq_states: Vec<BiquadState>,

    // One linked power envelope per band
    pub envelope_power: f32,
    pub attack_coeff: f32,
    pub release_coeff: f32,

    // Extra smoothing for the total EQ gain
    pub smoothed_gain_db: f32,
    pub gain_smooth_coeff: f32,
}

impl DynamicBand {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let mut band = Self {
            sample_rate,

            frequency_hz: 1000.0,
            q: 1.0,
            static_gain_db: 0.0,
            threshold_db: 0.0,
            ratio: 1.0,
            range_db: 0.0,
            attack_ms: 0.0,
            release_ms: 0.0,

            cos_w0: 1.0,
            alpha: 0.0,
            detector_coeffs: Coeffs::IDENTITY,
            eq_coeffs: Coeffs::IDENTITY,

            detector_states: vec![BiquadState::default(); channels],
            eq_states: vec![BiquadState::default(); channels],

            envelope_power: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,

            smoothed_gain_db: 0.0,
            gain_smooth_coeff: (-1.0 / (0.003 * sample_rate)).exp(),
        };

        band.redesign();
        band.update_timing();
        band
    }

    pub fn with_spec(mut self, spec: &DynEqBandSpec) -> Self {
        self.frequency_hz = spec.freq;
        self.q = spec.q;
        self.static_gain_db = spec.static_gain_db;
        self.threshold_db = spec.threshold_db;
        self.ratio = spec.ratio;
        self.range_db = spec.range_db;
        self.attack_ms = spec.attack_ms;
        self.release_ms = spec.release_ms;
        self
    }

    fn redesign(&mut self) {
        self.frequency_hz = self.frequency_hz.clamp(20.0, self.sample_rate * 0.45);

        self.q = self.q.clamp(0.1, 18.0);

        let w0 = TAU * self.frequency_hz / self.sample_rate as f32;
        let (sin_w0, cos_w0) = w0.sin_cos();

        self.cos_w0 = cos_w0;
        self.alpha = sin_w0 / (2.0 * self.q);

        // Band-pass detector with approximately 0 dB peak gain.
        let a0 = 1.0 + self.alpha;

        self.detector_coeffs = Coeffs {
            b0: self.alpha / a0,
            b1: 0.0,
            b2: -self.alpha / a0,
            a1: -2.0 * cos_w0 / a0,
            a2: (1.0 - self.alpha) / a0,
        };

        self.update_eq_coeffs(self.smoothed_gain_db);
    }

    pub fn update_eq_coeffs(&mut self, gain_db: f32) {
        // RBJ peaking-EQ coefficients.
        let a = 10.0_f32.powf(gain_db / 40.0);

        let a0 = 1.0 + self.alpha / a;

        self.eq_coeffs = Coeffs {
            b0: (1.0 + self.alpha * a) / a0,
            b1: (-2.0 * self.cos_w0) / a0,
            b2: (1.0 - self.alpha * a) / a0,
            a1: (-2.0 * self.cos_w0) / a0,
            a2: (1.0 - self.alpha / a) / a0,
        };
    }

    fn update_timing(&mut self) {
        let attack_seconds = self.attack_ms * 0.001;
        let release_seconds = self.release_ms * 0.001;

        self.attack_coeff = (-1.0 / (attack_seconds * self.sample_rate)).exp();

        self.release_coeff = (-1.0 / (release_seconds * self.sample_rate)).exp();
    }

    pub fn set_param(&mut self, field: u32, value: f32) {
        let mut redesign = false;

        match field {
            0 => {
                self.frequency_hz = value.clamp(20.0, self.sample_rate * 0.45);
                redesign = true;
            }
            1 => {
                self.q = value.clamp(0.1, 18.0);
                redesign = true;
            }
            2 => self.static_gain_db = value.clamp(-24.0, 24.0),
            3 => self.threshold_db = value.clamp(-90.0, 0.0),
            4 => self.ratio = value.clamp(1.0, 20.0),
            5 => self.range_db = value.clamp(-24.0, 24.0),
            6 => {
                self.attack_ms = value.clamp(0.1, 500.0);
                self.update_timing();
            }
            7 => {
                self.release_ms = value.clamp(1.0, 2000.0);
                self.update_timing();
            }
            _ => return,
        }

        if redesign {
            self.redesign();
        }
    }
}
