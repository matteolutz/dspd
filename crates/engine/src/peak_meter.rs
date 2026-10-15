use crate::{AudioBlock, MeterSpec, MeterWriter, Processor};

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
    held: f32, // linear
    writer: Option<MeterWriter>,
}

/// Parameter id used for the meter this stage declares.
pub const PEAK_METER_ID: u32 = 0;

const FLOOR: f32 = 1e-6; // -120 dBFS
const FALL_DB_PER_SEC: f32 = 20.0;

impl PeakMeter {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1) as f32,
            held: 0.0,
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

        let mut peak = 0.0f32;
        for c in 0..input.num_channels() {
            for s in input.channel(c) {
                peak = peak.max(s.abs());
            }
        }
        let seconds = input.frames() as f32 / self.sample_rate;
        let decay = 10f32.powf(-FALL_DB_PER_SEC * seconds / 20.0);
        self.held = peak.max(self.held * decay);

        if let Some(w) = &self.writer {
            w.set(0, 20.0 * self.held.max(FLOOR).log10());
        }
    }

    fn meter_specs(&self) -> Vec<MeterSpec> {
        vec![MeterSpec {
            param_id: PEAK_METER_ID,
            name: "Peak".into(),
        }]
    }

    fn attach_meters(&mut self, writer: MeterWriter) {
        self.writer = Some(writer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeterBank;

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
        let mut m = PeakMeter::new(sr);
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
        let mut loose = PeakMeter::new(48_000);
        run(&mut loose, 0.3, 16); // no writer attached: must not panic
    }
}
