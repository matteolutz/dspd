use crate::{AudioBlock, Processor};

/// Parameter id of the gain control (in dB).
pub const GAIN_PARAM_DB: u32 = 0;

/// Built-in gain stage. Useful as a test processor that needs no plugin files.
/// Gain changes are ramped linearly across one block to avoid zipper noise.
pub struct Gain {
    current: f32,
    target: f32,
}

fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

impl Gain {
    pub fn new(gain_db: f32) -> Self {
        let g = db_to_linear(gain_db as f64);
        Self { current: g, target: g }
    }
}

impl Processor for Gain {
    fn name(&self) -> &str {
        "gain"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let n = input.frames();
        output.set_frames(n);
        let step = if n > 0 {
            (self.target - self.current) / n as f32
        } else {
            0.0
        };
        let channels = input.num_channels().min(output.num_channels());
        for c in 0..channels {
            let src = input.channel(c);
            let dst = output.channel_mut(c);
            let mut g = self.current;
            for (d, s) in dst.iter_mut().zip(src) {
                g += step;
                *d = *s * g;
            }
        }
        self.current = self.target;
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if id == GAIN_PARAM_DB {
            self.target = db_to_linear(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minus_six_db_is_about_half() {
        let mut gain = Gain::new(-6.0206);
        let mut input = AudioBlock::new(2, 16);
        for c in 0..2 {
            input.channel_mut(c).fill(1.0);
        }
        let mut output = AudioBlock::new(2, 16);
        gain.process(&input, &mut output);
        for c in 0..2 {
            for s in output.channel(c) {
                assert!((s - 0.5).abs() < 1e-3, "got {s}");
            }
        }
    }

    #[test]
    fn param_change_ramps_over_one_block() {
        let mut gain = Gain::new(0.0);
        gain.set_param(GAIN_PARAM_DB, -120.0);
        let mut input = AudioBlock::new(1, 8);
        input.channel_mut(0).fill(1.0);
        let mut output = AudioBlock::new(1, 8);
        gain.process(&input, &mut output);
        let out = output.channel(0);
        assert!(out[0] > out[7], "should ramp down: {out:?}");
        assert!(out[7] < 1e-3);
    }
}
