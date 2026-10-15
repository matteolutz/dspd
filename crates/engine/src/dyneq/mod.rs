use crate::{dyneq::band::DynamicBand, session::DynEqBandSpec, AudioBlock, Processor};

mod band;
mod state;

pub const BAND_COUNT: usize = 6;
pub const PARAMS_PER_BAND: u32 = 8;

pub struct DynamicEq {
    bands: [DynamicBand; BAND_COUNT],
    prepared_channels: usize,
}

impl DynamicEq {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self {
            bands: std::array::from_fn(|_| DynamicBand::new(sample_rate, channels)),
            prepared_channels: channels,
        }
    }

    pub fn new_with_bands<'a>(
        sample_rate: f32,
        channels: usize,
        bands: impl IntoIterator<Item = &'a DynEqBandSpec>,
    ) -> Self {
        let mut bands_iter = bands
            .into_iter()
            .map(|spec| DynamicBand::new(sample_rate, channels).with_spec(spec));

        Self {
            bands: std::array::from_fn(|_| {
                bands_iter
                    .next()
                    .unwrap_or_else(|| DynamicBand::new(sample_rate, channels))
            }),
            prepared_channels: channels,
        }
    }
}

impl Processor for DynamicEq {
    fn name(&self) -> &str {
        "Dynamic EQ"
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if !value.is_finite() {
            return;
        }

        let band_index = (id / PARAMS_PER_BAND) as usize;
        let field = id % PARAMS_PER_BAND;

        if let Some(band) = self.bands.get_mut(band_index) {
            band.set_param(field, value as f32);
        }
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let frames = input.frames();
        output.set_frames(frames);

        let copy_channels = input.num_channels().min(output.num_channels());
        let channels = copy_channels.min(self.prepared_channels);

        for frame in 0..frames {
            // Start with dry input.
            for ch in 0..copy_channels {
                output.set_sample(ch, frame, input.sample(ch, frame));
            }

            if channels == 0 {
                continue;
            }

            // Each band uses its own detector, envelope and EQ state.
            for band in &mut self.bands {
                // 1. Measure the input energy in this frequency region.
                // Average power links channels to a common dynamic gain.
                let mut power_sum = 0.0_f32;
                let detector_coeffs = band.detector_coeffs;

                for ch in 0..channels {
                    let x = input.sample(ch, frame);
                    let d = band.detector_states[ch].tick(x, detector_coeffs);

                    power_sum += d * d;
                }

                let power = power_sum / channels as f32;

                // 2. Attack/release smoothing of detector power.
                let env_coeff = if power > band.envelope_power {
                    band.attack_coeff
                } else {
                    band.release_coeff
                };

                band.envelope_power = env_coeff * band.envelope_power + (1.0 - env_coeff) * power;

                // 3. Power to dB.
                let level_db = 10.0 * band.envelope_power.max(1.0e-12).log10();

                // 4. Threshold, ratio, and signed dynamic range.
                let over_db = (level_db - band.threshold_db).max(0.0);
                let compression_amount = over_db * (1.0 - 1.0 / band.ratio);

                let movement_db = compression_amount.min(band.range_db.abs());

                // Negative range cuts; positive range boosts above threshold.
                let dynamic_gain_db = band.range_db.signum() * movement_db;

                let target_gain_db = (band.static_gain_db + dynamic_gain_db).clamp(-24.0, 24.0);

                // 5. Smooth gain changes before redesigning the EQ.
                let g = band.gain_smooth_coeff;

                band.smoothed_gain_db = g * band.smoothed_gain_db + (1.0 - g) * target_gain_db;

                band.update_eq_coeffs(band.smoothed_gain_db);

                // 6. Process audio through this band's peaking EQ.
                let eq_coeffs = band.eq_coeffs;

                for ch in 0..channels {
                    let x = output.sample(ch, frame);
                    let y = band.eq_states[ch].tick(x, eq_coeffs);
                    output.set_sample(ch, frame, y);
                }
            }
        }
    }
}
