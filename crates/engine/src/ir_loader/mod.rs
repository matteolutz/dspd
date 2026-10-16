use std::{io, path::Path};

use fft_convolver::FFTConvolver;

use crate::{ir_loader::ir::LoadedIr, Processor};

mod ir;

#[derive(Debug)]
pub struct IRLoader {
    convolvers: Vec<FFTConvolver<f32>>,
}

impl IRLoader {
    pub fn from_wav(
        path: impl AsRef<Path>,
        engine_sample_rate: u32,
        num_channels: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let ir = LoadedIr::load_from_wav(path)?;

        if ir.sample_rate != engine_sample_rate {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "IR sample rate is {} Hz, but the engine uses {} Hz. \
                     Resample the IR before initialization.",
                    ir.sample_rate, engine_sample_rate,
                ),
            )
            .into());
        }

        if num_channels == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The processor needs at least one channel",
            )
            .into());
        }

        let mut convolvers = Vec::with_capacity(num_channels);

        for _ in 0..num_channels {
            let mut convolver = FFTConvolver::<f32>::default();

            // Precompute the IR and allocate working buffers here.
            // This may allocate, so run it off the audio thread.
            convolver.init(128, &ir.samples).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Could not initialize IR convolver: {e:?}"),
                )
            })?;

            convolvers.push(convolver);
        }

        Ok(Self { convolvers })
    }
}

impl Processor for IRLoader {
    fn name(&self) -> &str {
        "IR Loader"
    }

    fn process(&mut self, input: &crate::AudioBlock, output: &mut crate::AudioBlock) {
        let frames = input.frames();
        output.set_frames(frames);

        debug_assert_eq!(input.num_channels(), self.convolvers.len());
        debug_assert_eq!(output.num_channels(), self.convolvers.len());

        for (ch, convolver) in self.convolvers.iter_mut().enumerate() {
            let input_channel = input.channel(ch);
            let output_channel = output.channel_mut(ch);

            let convolver_res = convolver.process(input_channel, output_channel);
            if convolver_res.is_err() {
                output_channel.fill(0.0);
            }
        }
    }
}
