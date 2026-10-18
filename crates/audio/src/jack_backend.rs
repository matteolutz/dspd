use std::{
    ops::{Deref, DerefMut},
    sync::atomic::Ordering,
    time::Duration,
};

use dspd_engine::{AudioBlock, BoxedProcessor};
use jack::contrib::controller::ControlledProcessorTrait;

use crate::{AudioBackend, RunReport};

pub struct JackBackend {
    in_channels: usize,
    out_channels: usize,

    block_size: usize,
    max_block_size: usize,

    sample_rate: u32,
}

impl JackBackend {
    pub fn new(
        in_channels: usize,
        out_channels: usize,
        block_size: usize,
        max_block_size: usize,
        sample_rate: u32,
    ) -> Self {
        Self {
            in_channels,
            out_channels,
            block_size,
            max_block_size,
            sample_rate,
        }
    }
}

enum DspdProcessorCommand {
    SetParam { id: u32, value: f64 },
}

struct DspdProcessorWrapper {
    processor: BoxedProcessor,

    in_ports: Vec<jack::Port<jack::AudioIn>>,
    input: AudioBlock,

    out_ports: Vec<jack::Port<jack::AudioOut>>,
    output: AudioBlock,
}

impl DerefMut for DspdProcessorWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.processor
    }
}

impl Deref for DspdProcessorWrapper {
    type Target = BoxedProcessor;

    fn deref(&self) -> &Self::Target {
        &self.processor
    }
}

impl jack::contrib::controller::ControlledProcessorTrait for DspdProcessorWrapper {
    type Command = DspdProcessorCommand;

    type Notification = ();

    fn buffer_size(
        &mut self,
        _client: &jack::Client,
        size: jack::Frames,
        _channels: &mut jack::contrib::controller::ProcessorChannels<
            Self::Command,
            Self::Notification,
        >,
    ) -> jack::Control {
        self.input.set_frames(size as usize);
        self.output.set_frames(size as usize);

        jack::Control::Continue
    }

    fn process(
        &mut self,
        _client: &jack::Client,
        scope: &jack::ProcessScope,
        channels: &mut jack::contrib::controller::ProcessorChannels<
            Self::Command,
            Self::Notification,
        >,
    ) -> jack::Control {
        println!("scope.n_frames() = {}", scope.n_frames());

        while let Some(cmd) = channels.recv_command() {
            match cmd {
                DspdProcessorCommand::SetParam { id, value } => self.set_param(id, value),
            }
        }

        // move input data into the input buffer
        for (channel_idx, in_channel) in self.in_ports.iter().enumerate() {
            let in_data = in_channel.as_slice(scope);
            self.input.channel_mut(channel_idx).copy_from_slice(in_data);
        }

        self.processor.process(&self.input, &mut self.output);

        // move output data into the output buffer
        for (channel_idx, out_channel) in self.out_ports.iter_mut().enumerate() {
            let processor_out_data = self.output.channel(channel_idx);

            let out_data = out_channel.as_mut_slice(scope);
            out_data.copy_from_slice(processor_out_data);
        }

        jack::Control::Continue
    }
}

impl AudioBackend for JackBackend {
    fn run(
        &mut self,
        processor: Box<dyn dspd_engine::Processor + Send>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<crate::RunReport> {
        let started = std::time::Instant::now();
        let (client, _) = jack::Client::new("dspd_jack", jack::ClientOptions::default())?;

        client.set_buffer_size(self.block_size as u32)?;
        if client.sample_rate() != self.sample_rate {
            anyhow::bail!(
                "Requested sample rate {} but JACK server is running at {}",
                self.sample_rate,
                client.sample_rate()
            );
        }

        let in_ports: Vec<_> = (0..self.in_channels)
            .map(|channel| {
                client.register_port(
                    format!("dspd_input_{}", channel).as_str(),
                    jack::AudioIn::default(),
                )
            })
            .collect::<Result<_, _>>()?;

        let mut input = AudioBlock::new(in_ports.len(), self.max_block_size);
        input.set_frames(client.buffer_size() as usize);

        let out_ports: Vec<_> = (0..self.out_channels)
            .map(|channel| {
                client.register_port(
                    format!("dspd_output_{}", channel).as_str(),
                    jack::AudioOut::default(),
                )
            })
            .collect::<Result<_, _>>()?;

        let mut output = AudioBlock::new(out_ports.len(), self.max_block_size);
        output.set_frames(client.buffer_size() as usize);

        let processor_wrapper = DspdProcessorWrapper {
            processor,
            in_ports,
            input,
            out_ports,
            output,
        };

        let (processor_instance, mut handle) = processor_wrapper.instance(16, 16);

        let active_client = client.activate_async((), processor_instance)?;

        while !stop.load(Ordering::Relaxed) {
            for _notification in handle.drain_notifications() {}

            std::thread::sleep(Duration::from_millis(50));
        }

        active_client.deactivate()?;

        Ok(RunReport {
            frames_in: 0,
            frames_out: 0,
            blocks: 0,
            wall: started.elapsed(),
            stream_errors: 0,
        })
    }
}
