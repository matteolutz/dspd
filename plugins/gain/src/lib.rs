use dspd_api::{AudioBlock, ParamDef, PluginDescriptor, Processor};

/// Parameter id of the gain control (in dB).
pub const GAIN_PARAM_DB: u32 = 0;

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "gain",
        name: "Gain".to_string(),
        description: "Simple gain stage".to_string(),
        params: [ParamDef {
            id: GAIN_PARAM_DB,
            key: "gain_db",
            name: "Gain (dB)".to_string(),
            min: -120.0,
            max: 24.0,
            default: 0.0,
        }]
        .to_vec(),
        create: Box::new(|_ctx| Box::new(Gain::new(0.0))),
    }
}

/// Gain stage. Gain changes are ramped linearly across one block to avoid zipper noise.
pub struct Gain {
    current: f32,
    target: f32,
    /// False until the first block: a value set before audio starts is applied
    /// immediately instead of ramping up from unity.
    started: bool,
}

fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

impl Gain {
    pub fn new(gain_db: f32) -> Self {
        let g = db_to_linear(gain_db as f64);
        Self {
            current: g,
            target: g,
            started: false,
        }
    }
}

impl Processor for Gain {
    fn name(&self) -> &str {
        "gain"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        self.started = true;
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
            if !self.started {
                self.current = self.target;
            }
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
    fn value_set_before_the_first_block_applies_immediately() {
        let mut gain = Gain::new(0.0);
        gain.set_param(GAIN_PARAM_DB, -6.0206);
        let mut input = AudioBlock::new(1, 8);
        input.channel_mut(0).fill(1.0);
        let mut output = AudioBlock::new(1, 8);
        gain.process(&input, &mut output);
        assert!(
            output.channel(0).iter().all(|s| (s - 0.5).abs() < 1e-3),
            "{:?}",
            output.channel(0)
        );
    }

    #[test]
    fn descriptor_matches_the_constants() {
        let desc = descriptor();
        assert_eq!(desc.param_by_key("gain_db").unwrap().id, GAIN_PARAM_DB);
        let p = (desc.create)(&dspd_api::PluginContext {
            sample_rate: 48_000,
            max_block: 64,
            channels: 2,
        });
        assert_eq!(p.name(), "gain");
    }

    #[test]
    fn param_change_ramps_over_one_block() {
        let mut gain = Gain::new(0.0);
        let mut input = AudioBlock::new(1, 8);
        input.channel_mut(0).fill(1.0);
        let mut output = AudioBlock::new(1, 8);
        gain.process(&input, &mut output); // audio has started
        gain.set_param(GAIN_PARAM_DB, -120.0);
        gain.process(&input, &mut output);
        let out = output.channel(0);
        assert!(out[0] > out[7], "should ramp down: {out:?}");
        assert!(out[7] < 1e-3);
    }
}
