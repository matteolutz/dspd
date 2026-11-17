use std::{
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    time::Duration,
};

use dspd_engine::{AudioBlock, BoxedProcessor};
use jack::contrib::controller::ControlledProcessorTrait;
use jack::PortSpec;

use crate::{AudioBackend, AudioPatching, PatchState, RunReport};

mod daemon;
pub use daemon::*;

#[derive(Default)]
pub struct JackBackendDaemonOptions {
    pub input_device: Option<String>,
    pub output_device: Option<String>,

    pub realtime: JackDaemonRealtimeConfig,

    pub no_audio_reservation: bool,
}

pub struct JackBackend {
    in_channels: usize,
    out_channels: usize,

    block_size: usize,
    max_block_size: usize,

    sample_rate: u32,

    daemon: Option<JackDaemon>,
    patching: Arc<JackPatching>,
    patch_commands: Option<Receiver<JackPatchCommand>>,
}

enum JackPatchCommand {
    ConnectInput {
        source: String,
        input: usize,
        result: Sender<anyhow::Result<()>>,
    },
    ConnectOutput {
        output: usize,
        destination: String,
        result: Sender<anyhow::Result<()>>,
    },
    DisconnectInput {
        input: usize,
        result: Sender<anyhow::Result<()>>,
    },
    DisconnectOutput {
        output: usize,
        result: Sender<anyhow::Result<()>>,
    },
    CurrentPatch {
        result: Sender<anyhow::Result<PatchState>>,
    },
    CaptureDevices {
        result: Sender<anyhow::Result<Vec<String>>>,
    },
    PlaybackDevices {
        result: Sender<anyhow::Result<Vec<String>>>,
    },
}

pub struct JackPatching {
    commands: Sender<JackPatchCommand>,
}

impl JackBackend {
    pub fn new(
        in_channels: usize,
        out_channels: usize,
        block_size: usize,
        max_block_size: usize,
        sample_rate: u32,
        daemon_options: Option<JackBackendDaemonOptions>,
    ) -> anyhow::Result<Self> {
        let mut daemon = None;
        if let Some(daemon_options) = daemon_options {
            daemon = Some(JackDaemon::start(&JackDaemonConfig {
                sample_rate,
                period_size: block_size as u32,
                periods: 3,
                backend: "alsa".into(),
                capture_device: daemon_options.input_device,
                playback_device: daemon_options.output_device,
                realtime: daemon_options.realtime,
                no_audio_reservation: daemon_options.no_audio_reservation,
            })?);
        }

        let (command_sender, command_receiver) = mpsc::channel();

        Ok(Self {
            in_channels,
            out_channels,
            block_size,
            max_block_size,
            sample_rate,
            daemon,
            patching: Arc::new(JackPatching {
                commands: command_sender,
            }),
            patch_commands: Some(command_receiver),
        })
    }
}

impl JackPatching {
    fn request<T>(
        &self,
        command: impl FnOnce(Sender<anyhow::Result<T>>) -> JackPatchCommand,
    ) -> anyhow::Result<T> {
        let (result_sender, result_receiver) = mpsc::channel();
        self.commands
            .send(command(result_sender))
            .map_err(|_| anyhow::anyhow!("JACK backend is not running"))?;
        result_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| anyhow::anyhow!("timed out waiting for the JACK backend"))?
    }

    pub fn connect_input(&self, source: &str, input: usize) -> anyhow::Result<()> {
        if source.is_empty() {
            anyhow::bail!("JACK source port name must not be empty");
        }
        self.request(|result| JackPatchCommand::ConnectInput {
            source: source.into(),
            input,
            result,
        })
    }

    pub fn connect_output(&self, output: usize, destination: &str) -> anyhow::Result<()> {
        if destination.is_empty() {
            anyhow::bail!("JACK destination port name must not be empty");
        }
        self.request(|result| JackPatchCommand::ConnectOutput {
            output,
            destination: destination.into(),
            result,
        })
    }

    pub fn disconnect_input(&self, input: usize) -> anyhow::Result<()> {
        self.request(|result| JackPatchCommand::DisconnectInput { input, result })
    }

    pub fn disconnect_output(&self, output: usize) -> anyhow::Result<()> {
        self.request(|result| JackPatchCommand::DisconnectOutput { output, result })
    }

    pub fn current_patch(&self) -> anyhow::Result<PatchState> {
        self.request(|result| JackPatchCommand::CurrentPatch { result })
    }

    pub fn capture_devices(&self) -> anyhow::Result<Vec<String>> {
        self.request(|result| JackPatchCommand::CaptureDevices { result })
    }

    pub fn playback_devices(&self) -> anyhow::Result<Vec<String>> {
        self.request(|result| JackPatchCommand::PlaybackDevices { result })
    }
}

impl AudioPatching for JackPatching {
    fn connect_input(&self, source: &str, input: usize) -> anyhow::Result<()> {
        self.connect_input(source, input)
    }

    fn connect_output(&self, output: usize, destination: &str) -> anyhow::Result<()> {
        self.connect_output(output, destination)
    }

    fn disconnect_input(&self, input: usize) -> anyhow::Result<()> {
        self.disconnect_input(input)
    }

    fn disconnect_output(&self, output: usize) -> anyhow::Result<()> {
        self.disconnect_output(output)
    }

    fn current_patch(&self) -> anyhow::Result<PatchState> {
        self.current_patch()
    }

    fn capture_devices(&self) -> anyhow::Result<Vec<String>> {
        self.capture_devices()
    }

    fn playback_devices(&self) -> anyhow::Result<Vec<String>> {
        self.playback_devices()
    }
}

#[allow(unused)]
enum DspdProcessorCommand {
    SetParam { id: u32, value: f64 },
}

struct DspdProcessorWrapper {
    processor: BoxedProcessor,

    in_ports: Vec<jack::Port<jack::AudioIn>>,
    input: AudioBlock,

    out_ports: Vec<jack::Port<jack::AudioOut>>,
    output: AudioBlock,

    frames_count: Arc<AtomicU64>,
    block_count: Arc<AtomicU64>,
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
        while let Some(cmd) = channels.recv_command() {
            match cmd {
                DspdProcessorCommand::SetParam { id, value } => self.set_param(id, value),
            }
        }

        // adjust the input and output buffers to the current block size
        // `scope.n_frames()` should always be equal to the block size set in `buffer_size()`
        self.input.set_frames(scope.n_frames() as usize);
        self.output.set_frames(scope.n_frames() as usize);

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

        self.frames_count
            .fetch_add(scope.n_frames() as u64, Ordering::Relaxed);
        self.block_count.fetch_add(1, Ordering::Relaxed);

        jack::Control::Continue
    }
}

fn dsp_port_name(client_name: &str, direction: &str, index: usize) -> String {
    format!("{client_name}:{direction}_{index}")
}

fn disconnect_all(client: &jack::Client, port_name: &str) -> anyhow::Result<()> {
    let Some(port) = client.port_by_name(port_name) else {
        anyhow::bail!("JACK port {port_name} does not exist");
    };
    for connected in port.get_connections() {
        client
            .disconnect_ports_by_name(&connected, port_name)
            .map_err(|error| anyhow::anyhow!("disconnecting JACK ports: {error}"))?;
    }
    Ok(())
}

fn connect_input(
    client: &jack::Client,
    client_name: &str,
    source: &str,
    input: usize,
    input_count: usize,
) -> anyhow::Result<()> {
    if input >= input_count {
        anyhow::bail!("JACK input index {input} is out of range");
    }
    let destination = dsp_port_name(client_name, "input", input);
    disconnect_all(client, &destination)?;
    client
        .connect_ports_by_name(source, &destination)
        .map_err(|error| anyhow::anyhow!("connecting JACK port {source} -> {destination}: {error}"))
}

fn connect_output(
    client: &jack::Client,
    client_name: &str,
    output: usize,
    destination: &str,
    output_count: usize,
) -> anyhow::Result<()> {
    if output >= output_count {
        anyhow::bail!("JACK output index {output} is out of range");
    }
    let source = dsp_port_name(client_name, "output", output);
    disconnect_all(client, &source)?;
    client
        .connect_ports_by_name(&source, destination)
        .map_err(|error| anyhow::anyhow!("connecting JACK port {source} -> {destination}: {error}"))
}

fn external_ports(client: &jack::Client, flags: jack::PortFlags) -> Vec<String> {
    let owned_prefix = format!("{}:", client.name());
    client
        .ports(None, Some(jack::AudioIn::default().jack_port_type()), flags)
        .into_iter()
        .filter(|name| !name.starts_with(&owned_prefix))
        .collect()
}

fn current_patch(
    client: &jack::Client,
    client_name: &str,
    input_count: usize,
    output_count: usize,
) -> PatchState {
    let owned_prefix = format!("{client_name}:");
    let connections = |direction: &str, count: usize| {
        (0..count)
            .map(|index| {
                let name = dsp_port_name(client_name, direction, index);
                client.port_by_name(&name).and_then(|port| {
                    port.get_connections()
                        .into_iter()
                        .find(|name| !name.starts_with(&owned_prefix))
                })
            })
            .collect()
    };

    PatchState {
        inputs: connections("input", input_count),
        outputs: connections("output", output_count),
    }
}

fn handle_patch_command(
    client: &jack::Client,
    client_name: &str,
    input_count: usize,
    output_count: usize,
    command: JackPatchCommand,
) {
    match command {
        JackPatchCommand::ConnectInput {
            source,
            input,
            result,
        } => {
            let _ = result.send(connect_input(
                client,
                client_name,
                &source,
                input,
                input_count,
            ));
        }
        JackPatchCommand::ConnectOutput {
            output,
            destination,
            result,
        } => {
            let _ = result.send(connect_output(
                client,
                client_name,
                output,
                &destination,
                output_count,
            ));
        }
        JackPatchCommand::DisconnectInput { input, result } => {
            let outcome = if input < input_count {
                disconnect_all(client, &dsp_port_name(client_name, "input", input))
            } else {
                Err(anyhow::anyhow!("JACK input index {input} is out of range"))
            };
            let _ = result.send(outcome);
        }
        JackPatchCommand::DisconnectOutput { output, result } => {
            let outcome = if output < output_count {
                disconnect_all(client, &dsp_port_name(client_name, "output", output))
            } else {
                Err(anyhow::anyhow!(
                    "JACK output index {output} is out of range"
                ))
            };
            let _ = result.send(outcome);
        }
        JackPatchCommand::CurrentPatch { result } => {
            let _ = result.send(Ok(current_patch(
                client,
                client_name,
                input_count,
                output_count,
            )));
        }
        JackPatchCommand::CaptureDevices { result } => {
            let _ = result.send(Ok(external_ports(client, jack::PortFlags::IS_OUTPUT)));
        }
        JackPatchCommand::PlaybackDevices { result } => {
            let _ = result.send(Ok(external_ports(client, jack::PortFlags::IS_INPUT)));
        }
    }
}

impl AudioBackend for JackBackend {
    fn patching(&self) -> Option<Arc<dyn AudioPatching>> {
        Some(Arc::clone(&self.patching) as Arc<dyn AudioPatching>)
    }

    fn run(
        &mut self,
        processor: Box<dyn dspd_engine::Processor + Send>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<crate::RunReport> {
        let started = std::time::Instant::now();
        let patch_commands = self
            .patch_commands
            .take()
            .ok_or_else(|| anyhow::anyhow!("JACK backend cannot be run more than once"))?;
        let (client, _) = jack::Client::new("dspd", jack::ClientOptions::default())?;
        let client_name = client.name().to_owned();

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
                    format!("input_{}", channel).as_str(),
                    jack::AudioIn::default(),
                )
            })
            .collect::<Result<_, _>>()?;

        let mut input = AudioBlock::new(in_ports.len(), self.max_block_size);
        input.set_frames(client.buffer_size() as usize);

        let out_ports: Vec<_> = (0..self.out_channels)
            .map(|channel| {
                client.register_port(
                    format!("output_{}", channel).as_str(),
                    jack::AudioOut::default(),
                )
            })
            .collect::<Result<_, _>>()?;

        let mut output = AudioBlock::new(out_ports.len(), self.max_block_size);
        output.set_frames(client.buffer_size() as usize);

        let frames_count = Arc::new(AtomicU64::new(0));
        let block_count = Arc::new(AtomicU64::new(0));

        let processor_wrapper = DspdProcessorWrapper {
            processor,
            in_ports,
            input,
            out_ports,
            output,

            frames_count: frames_count.clone(),
            block_count: block_count.clone(),
        };

        let (processor_instance, mut handle) = processor_wrapper.instance(16, 16);

        let active_client = client.activate_async((), processor_instance)?;

        while !stop.load(Ordering::Relaxed) {
            while let Ok(command) = patch_commands.try_recv() {
                handle_patch_command(
                    active_client.as_client(),
                    &client_name,
                    self.in_channels,
                    self.out_channels,
                    command,
                );
            }
            for _notification in handle.drain_notifications() {}

            std::thread::sleep(Duration::from_millis(50));
        }

        active_client.deactivate()?;

        let n_frames = frames_count.load(Ordering::Relaxed);
        let n_blocks = block_count.load(Ordering::Relaxed);

        Ok(RunReport {
            frames_in: n_frames,
            frames_out: n_frames,
            blocks: n_blocks,
            wall: started.elapsed(),
            stream_errors: 0,
        })
    }
}
