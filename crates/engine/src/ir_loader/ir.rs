use std::{io, path::Path};

use hound::{SampleFormat, WavReader};

#[derive(Debug)]
pub struct LoadedIr {
    pub(super) samples: Vec<f32>,
    pub(super) sample_rate: u32,
}

impl LoadedIr {
    pub fn load_from_wav(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut reader = WavReader::open(path)?;
        let spec = reader.spec();

        // This implementation expects one mono impulse response.
        if spec.channels != 1 {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "Expected a mono IR WAV file").into(),
            );
        }

        // Convert all supported file formats to normalized f32.
        let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
            (SampleFormat::Float, 32) => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,

            (SampleFormat::Int, 16) => reader
                .samples::<i16>()
                .map(|s| s.map(|v| v as f32 / 32_768.0))
                .collect::<Result<Vec<_>, _>>()?,

            (SampleFormat::Int, 24) => reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / 8_388_608.0))
                .collect::<Result<Vec<_>, _>>()?,

            (SampleFormat::Int, 32) => reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / 2_147_483_648.0))
                .collect::<Result<Vec<_>, _>>()?,

            (format, bits) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unsupported IR format: {format:?}, {bits} bits"),
                )
                .into());
            }
        };

        if samples.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "The IR file is empty").into());
        }

        if samples.iter().any(|s| !s.is_finite()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The IR contains non-finite samples",
            )
            .into());
        }

        Ok(LoadedIr {
            samples,
            sample_rate: spec.sample_rate,
        })
    }
}
