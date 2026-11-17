use dspd_engine::{AudioDevice, AudioPatching, PatchState};

use crate::jack_backend::{dspd_input_port_name, dspd_output_port_name};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{mpsc, Arc},
    time::Duration,
};

pub struct JackAudioDevice {
    client: String,
    name: String,
    n_channels: usize,
    start_channel: usize,
}

impl AudioDevice for JackAudioDevice {
    fn display_name(&self) -> Cow<'_, str> {
        format!("{}:{}", self.client, self.name).into()
    }

    fn num_channels(&self) -> usize {
        self.n_channels
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl JackAudioDevice {
    fn to_port_name(&self, channel_idx: usize) -> anyhow::Result<String> {
        if channel_idx >= self.n_channels {
            anyhow::bail!(
                "channel index {} is out of range for device {} with {} channels",
                channel_idx,
                self.name,
                self.n_channels
            );
        }

        let port_idx = self.start_channel + channel_idx;
        Ok(format!("{}:{}_{}", self.client, self.name, port_idx))
    }

    /// Check if the given port name belongs to this device, and if so,
    /// return the 0 based channel index.
    fn is_device_channel(&self, port_name: &str) -> Option<usize> {
        let prefix = format!("{}:{}_", self.client, self.name);
        if !port_name.starts_with(&prefix) {
            return None;
        }

        let channel_str = &port_name[prefix.len()..];
        let channel = channel_str.parse::<usize>().ok()?;
        let channel_idx = channel.checked_sub(self.start_channel)?;

        if channel_idx < self.n_channels {
            Some(channel_idx)
        } else {
            None
        }
    }
}

pub enum JackPatchCommand {
    ConnectInput {
        input_port: String,
        dspd_input_idx: usize,
        result_sender: mpsc::Sender<anyhow::Result<()>>,
    },

    ConnectOutput {
        output_port: String,
        dspd_output_idx: usize,
        result_sender: mpsc::Sender<anyhow::Result<()>>,
    },

    DisconnectInput {
        dspd_input_idx: usize,
        result_sender: mpsc::Sender<anyhow::Result<()>>,
    },

    DisconnectOutput {
        dspd_output_idx: usize,
        result_sender: mpsc::Sender<anyhow::Result<()>>,
    },

    CurrentPatch {
        result_sender: mpsc::Sender<anyhow::Result<PatchState>>,
    },

    CaptureDevices {
        result_sender: mpsc::Sender<anyhow::Result<Vec<Arc<dyn AudioDevice>>>>,
    },

    PlaybackDevices {
        result_sender: mpsc::Sender<anyhow::Result<Vec<Arc<dyn AudioDevice>>>>,
    },
}

pub struct JackAudioPatching {
    pub(super) commands: mpsc::Sender<JackPatchCommand>,
}

impl JackAudioPatching {
    fn request_blocking<T>(
        &self,
        command: impl FnOnce(mpsc::Sender<anyhow::Result<T>>) -> JackPatchCommand,
    ) -> anyhow::Result<T> {
        let (result_sender, result_recv) = mpsc::channel();
        self.commands
            .send(command(result_sender))
            .map_err(|_| anyhow::anyhow!("JACK backend is not running"))?;

        result_recv
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| {
                anyhow::anyhow!("JACK backend did not respond to patching request in time")
            })?
    }
}

fn jack_device(device: &dyn AudioDevice) -> anyhow::Result<&JackAudioDevice> {
    device
        .as_any()
        .downcast_ref()
        .ok_or_else(|| anyhow::anyhow!("Device isn't a JackAudioDevice"))
}

impl AudioPatching for JackAudioPatching {
    fn connect_input(
        &self,
        input_device: &dyn AudioDevice,
        input_device_channel_idx: usize,
        dspd_input_idx: usize,
    ) -> anyhow::Result<()> {
        let input_port = jack_device(input_device)?.to_port_name(input_device_channel_idx)?;

        self.request_blocking(|result_sender| JackPatchCommand::ConnectInput {
            input_port,
            dspd_input_idx,
            result_sender,
        })
    }

    fn connect_output(
        &self,
        dspd_output_idx: usize,
        output_device: &dyn AudioDevice,
        output_device_channel_idx: usize,
    ) -> anyhow::Result<()> {
        let output_port = jack_device(output_device)?.to_port_name(output_device_channel_idx)?;

        self.request_blocking(|result_sender| JackPatchCommand::ConnectOutput {
            output_port,
            dspd_output_idx,
            result_sender,
        })
    }

    fn disconnect_input(&self, dspd_input_idx: usize) -> anyhow::Result<()> {
        self.request_blocking(|result_sender| JackPatchCommand::DisconnectInput {
            dspd_input_idx,
            result_sender,
        })
    }

    fn disconnect_output(&self, dspd_output_idx: usize) -> anyhow::Result<()> {
        self.request_blocking(|result_sender| JackPatchCommand::DisconnectOutput {
            dspd_output_idx,
            result_sender,
        })
    }

    fn current_patch(&self) -> anyhow::Result<PatchState> {
        self.request_blocking(|result_sender| JackPatchCommand::CurrentPatch { result_sender })
    }

    fn capture_devices(&self) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>> {
        self.request_blocking(|result_sender| JackPatchCommand::CaptureDevices { result_sender })
    }

    fn playback_devices(&self) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>> {
        self.request_blocking(|result_sender| JackPatchCommand::PlaybackDevices { result_sender })
    }
}

#[derive(Copy, Clone)]
pub(super) struct JackPatchCommandExecutionCtx {
    pub in_channels: usize,
    pub out_channels: usize,
}

pub(super) trait JackClientPatchExt {
    fn get_devices(&self, flags: jack::PortFlags) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>>;

    fn handle_patch_command(
        &self,
        command: JackPatchCommand,
        ctx: JackPatchCommandExecutionCtx,
    ) -> anyhow::Result<()>;
}

impl JackClientPatchExt for jack::Client {
    fn get_devices(&self, flags: jack::PortFlags) -> anyhow::Result<Vec<Arc<dyn AudioDevice>>> {
        let mut devices: Vec<JackAudioDevice> = Vec::new();
        let mut index: HashMap<(String, String), usize> = HashMap::new();

        let ports = self
            .ports(None, None, flags)
            .into_iter()
            .filter_map(|p| {
                let (client, port) = p.split_once(':')?;

                let (name, channel) = split_channel_suffix(port);
                let channel = channel?; // for now, only allow ports with channel suffixes (e.g. "in_1", "out_1")

                Some((client.to_string(), name.to_string(), channel))
            })
            .filter(|(client, _, _)| client != self.name());

        for (client, name, channel_idx) in ports {
            let key = (client.clone(), name.clone());

            match index.get(&key) {
                Some(&device_idx) => {
                    let device = &mut devices[device_idx];
                    device.n_channels += 1;
                    device.start_channel = device.start_channel.min(channel_idx);
                }
                None => {
                    index.insert(key, devices.len());
                    devices.push(JackAudioDevice {
                        client,
                        name,
                        n_channels: 1,
                        start_channel: channel_idx,
                    })
                }
            }
        }

        Ok(devices
            .into_iter()
            .map(|d| Arc::new(d) as Arc<dyn AudioDevice>)
            .collect())
    }

    fn handle_patch_command(
        &self,
        command: JackPatchCommand,
        _ctx: JackPatchCommandExecutionCtx,
    ) -> anyhow::Result<()> {
        match command {
            JackPatchCommand::CaptureDevices { result_sender } => {
                result_sender.send(self.get_devices(jack::PortFlags::IS_OUTPUT))?;
            }
            JackPatchCommand::PlaybackDevices { result_sender } => {
                result_sender.send(self.get_devices(jack::PortFlags::IS_INPUT))?;
            }
            JackPatchCommand::CurrentPatch { result_sender } => {
                result_sender.send(Err(anyhow::anyhow!("find a solution for this")))?;
            }
            JackPatchCommand::ConnectInput {
                input_port,
                dspd_input_idx,
                result_sender,
            } => {
                let dspd_port_name =
                    format!("{}:{}", self.name(), dspd_input_port_name(dspd_input_idx));
                let do_connect = || {
                    let dspd_port = self
                        .port_by_name(&dspd_port_name)
                        .ok_or(anyhow::anyhow!("dspd input port not found"))?;
                    self.disconnect(&dspd_port)?;
                    self.connect_ports_by_name(&input_port, &dspd_port_name)?;
                    Ok(())
                };

                result_sender.send(do_connect())?;
            }
            JackPatchCommand::ConnectOutput {
                output_port,
                dspd_output_idx,
                result_sender,
            } => {
                let dspd_port_name =
                    format!("{}:{}", self.name(), dspd_output_port_name(dspd_output_idx));
                let do_connect = || {
                    let dspd_port = self
                        .port_by_name(&dspd_port_name)
                        .ok_or(anyhow::anyhow!("dspd output port not found"))?;
                    self.disconnect(&dspd_port)?;
                    self.connect_ports_by_name(&dspd_port_name, &output_port)?;
                    Ok(())
                };

                result_sender.send(do_connect())?;
            }
            JackPatchCommand::DisconnectInput {
                dspd_input_idx,
                result_sender,
            } => {
                let do_disconnect = || {
                    let dspd_port = self
                        .port_by_name(&format!(
                            "{}:{}",
                            self.name(),
                            dspd_input_port_name(dspd_input_idx)
                        ))
                        .ok_or(anyhow::anyhow!("dspd input port not found"))?;
                    self.disconnect(&dspd_port)?;
                    Ok(())
                };

                result_sender.send(do_disconnect())?;
            }
            JackPatchCommand::DisconnectOutput {
                dspd_output_idx,
                result_sender,
            } => {
                let do_disconnect = || {
                    let dspd_port = self
                        .port_by_name(&format!(
                            "{}:{}",
                            self.name(),
                            dspd_output_port_name(dspd_output_idx)
                        ))
                        .ok_or(anyhow::anyhow!("dspd output port not found"))?;
                    self.disconnect(&dspd_port)?;
                    Ok(())
                };

                result_sender.send(do_disconnect())?;
            }
        }

        Ok(())
    }
}

fn split_channel_suffix(port: &str) -> (&str, Option<usize>) {
    let base = port.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &port[base.len()..];
    match digits.parse::<usize>() {
        Ok(n) => (
            base.trim_end_matches(|c| c == '_' || c == '-' || c == ' ' || c == '.'),
            Some(n),
        ),
        Err(_) => (port, None),
    }
}
