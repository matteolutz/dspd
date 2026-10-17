use dspd_api::{AudioBlock, MeterSpec, MeterWriter, ParamDef, PluginDescriptor, Processor};

/// Pass-through stage that reports the peak level of its input in dBFS.
///
/// The reading rises instantly and falls at 20 dB per second, so a client that
/// polls at 30 Hz still sees short peaks. Floor is -120 dBFS.
///
/// This is also the smallest example of a *native* processor with a meter:
/// declare it in `meter_specs`, keep the writer from `attach_meters`, call
/// `set` from `process`.
pub struct PeakMeter {
    sample_rate: f32,
    held: Vec<f32>, // linear
    writer: Option<MeterWriter>,
}

/// Descriptor the host registers: `{"type": "peak_meter"}`. It has no settable parameters.
pub static DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    id: "peak_meter",
    name: "Peak Meter",
    description: "Pass-through that publishes the peak level in dBFS",
    params: &[] as &[ParamDef],
    create: |ctx| Box::new(PeakMeter::new(ctx.sample_rate, ctx.channels)),
};

const FLOOR: f32 = 1e-6; // -120 dBFS
const FALL_DB_PER_SEC: f32 = 20.0;

impl PeakMeter {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            sample_rate: sample_rate.max(1) as f32,
            held: vec![0.0; channels],
            writer: None,
        }
    }
}

impl Processor for PeakMeter {
    fn name(&self) -> &str {
        "peak meter"
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        output.copy_from(input);

        if input.num_channels() != self.held.len() {
            return;
        }

        for (channel, held) in self.held.iter_mut().enumerate() {
            let mut peak = 0.0f32;
            for s in input.channel(channel) {
                peak = peak.max(s.abs());
            }

            let seconds = input.frames() as f32 / self.sample_rate;
            let decay = 10f32.powf(-FALL_DB_PER_SEC * seconds / 20.0);
            *held = peak.max(*held * decay);

            if let Some(w) = &self.writer {
                w.set(channel, 20.0 * held.max(FLOOR).log10());
            }
        }
    }

    fn meter_specs(&self) -> Vec<MeterSpec> {
        (0..self.held.len())
            .map(|channel| MeterSpec {
                param_id: channel as u32,
                name: format!("Peak {}", channel + 1),
            })
            .collect()
    }

    fn attach_meters(&mut self, writer: MeterWriter) {
        self.writer = Some(writer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspd_api::MeterBank;

    fn run(m: &mut PeakMeter, value: f32, frames: usize) {
        let mut input = AudioBlock::new(2, frames);
        input.channel_mut(0).fill(value);
        input.channel_mut(1).fill(value * 0.5);
        let mut output = AudioBlock::new(2, frames);
        m.process(&input, &mut output);
        assert_eq!(
            output.channel(0),
            input.channel(0),
            "must pass audio through unchanged"
        );
    }

    fn attached(sr: u32) -> (PeakMeter, std::sync::Arc<MeterBank>) {
        let bank = MeterBank::new(1);
        let mut m = PeakMeter::new(sr, 1);
        m.attach_meters(MeterWriter::new(std::sync::Arc::clone(&bank), 0, 1));
        (m, bank)
    }

    #[test]
    fn reports_peak_across_channels_in_dbfs() {
        let (mut m, bank) = attached(48_000);
        run(&mut m, 0.5, 64);
        assert!((bank.get(0) - (-6.0206)).abs() < 0.01, "{}", bank.get(0));
    }

    #[test]
    fn falls_at_twenty_db_per_second() {
        let (mut m, bank) = attached(48_000);
        run(&mut m, 1.0, 64);
        assert!(bank.get(0).abs() < 0.01);
        // one second of silence, in 64-frame blocks
        for _ in 0..750 {
            run(&mut m, 0.0, 64);
        }
        assert!((bank.get(0) - (-20.0)).abs() < 0.2, "{}", bank.get(0));
    }

    #[test]
    fn silence_reads_the_floor_and_unattached_meter_is_harmless() {
        let (mut m, bank) = attached(48_000);
        run(&mut m, 0.0, 64);
        assert_eq!(bank.get(0), -120.0);
        let mut loose = PeakMeter::new(48_000, 1);
        run(&mut loose, 0.3, 16); // no writer attached: must not panic
    }
}
