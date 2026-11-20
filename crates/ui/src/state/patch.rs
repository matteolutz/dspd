use std::sync::Arc;

use dspd_api::{PatchEntry, PatchState};
use gpui_kit::Context;
use gpui_kit::{App, AppContext, Entity, EventEmitter, SharedString};

use crate::state::{DspdHttpClient, Session, UiState};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PatchDirection {
    Capture,
    Playback,
}

impl PatchDirection {
    pub fn name(&self) -> &'static str {
        match self {
            PatchDirection::Capture => "Capture",
            PatchDirection::Playback => "Playback",
        }
    }

    pub fn other(&self) -> Self {
        match self {
            PatchDirection::Capture => PatchDirection::Playback,
            PatchDirection::Playback => PatchDirection::Capture,
        }
    }

    pub fn get_num_channels(&self, cx: &App) -> usize {
        let session = UiState::session(cx).read(cx);
        match self {
            Self::Capture => session.num_inputs(),
            Self::Playback => session.num_outputs(),
        }
    }

    pub fn devices<'a>(&'a self, cx: &'a App) -> &'a [AudioDevice] {
        UiState::patch(cx).read(cx).devices_for(*self)
    }

    pub fn device_by_id<'a>(
        &'a self,
        device_id: &SharedString,
        cx: &'a App,
    ) -> Option<&'a AudioDevice> {
        self.devices(cx).iter().find(|d| &d.id == device_id)
    }

    pub fn get_patch_for<'a>(&'a self, channel_idx: usize, cx: &'a App) -> Option<&'a PatchEntry> {
        UiState::patch(cx)
            .read(cx)
            .get_patch_for(*self, channel_idx)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct AudioDevice {
    pub id: SharedString,
    pub name: SharedString,
    pub num_channels: usize,
}

pub enum UiPatchEvent {}

#[derive(Debug)]
pub struct UiPatchState {
    capture_devices: Vec<AudioDevice>,
    playback_devices: Vec<AudioDevice>,

    patch: PatchState,
}

impl EventEmitter<UiPatchEvent> for UiPatchState {}

impl UiPatchState {
    fn default_capture_devices() -> Vec<AudioDevice> {
        #[cfg(debug_assertions)]
        {
            vec![AudioDevice {
                id: "debug-capture".into(),
                name: "Debug Capture Device".into(),
                num_channels: 32,
            }]
        }
        #[cfg(not(debug_assertions))]
        {
            vec![]
        }
    }

    fn default_playback_devices() -> Vec<AudioDevice> {
        #[cfg(debug_assertions)]
        {
            vec![AudioDevice {
                id: "debug-playback".into(),
                name: "Debug Playback Device".into(),
                num_channels: 32,
            }]
        }
        #[cfg(not(debug_assertions))]
        {
            vec![]
        }
    }

    pub(super) fn update_patch_detached(
        &self,
        client: Arc<DspdHttpClient>,
        session: Entity<Session>,
        cx: &Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let patch = client.get_current_patch(cx).await.unwrap_or_else(|_| {
                let (n_inputs, n_outputs) = cx.read_entity(&session, |s, _| (s.inputs, s.outputs));

                PatchState {
                    inputs: vec![None; n_inputs],
                    outputs: vec![None; n_outputs],
                }
            });

            _ = this.update(cx, |this, cx| {
                this.patch = patch;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) async fn get(
        client: &DspdHttpClient,
        session: &Entity<Session>,
        cx: &mut App,
    ) -> reqwest::Result<Entity<Self>> {
        let capture_devices = client
            .get_capture_devices(cx)
            .await
            .unwrap_or_else(|_| Self::default_capture_devices());
        let playback_devices = client
            .get_playback_devices(cx)
            .await
            .unwrap_or_else(|_| Self::default_playback_devices());

        let patch = client.get_current_patch(cx).await.unwrap_or_else(|_| {
            let n_inputs = session.read(cx).inputs;
            let n_outputs = session.read(cx).outputs;

            PatchState {
                inputs: vec![None; n_inputs],
                outputs: vec![None; n_outputs],
            }
        });

        let state = Self {
            capture_devices,
            playback_devices,
            patch,
        };

        Ok(cx.new(|_| state))
    }

    pub fn devices_for(&self, direction: PatchDirection) -> &[AudioDevice] {
        match direction {
            PatchDirection::Capture => &self.capture_devices,
            PatchDirection::Playback => &self.playback_devices,
        }
    }

    pub fn get_patch_for(
        &self,
        direction: PatchDirection,
        channel_idx: usize,
    ) -> Option<&PatchEntry> {
        let patch = match direction {
            PatchDirection::Capture => &self.patch.inputs,
            PatchDirection::Playback => &self.patch.outputs,
        };

        patch.get(channel_idx).and_then(|p| p.as_ref())
    }

    #[cfg(debug_assertions)]
    pub fn debug_set_local_patch_for(
        &mut self,
        direction: PatchDirection,
        channel_idx: usize,
        entry: Option<PatchEntry>,
        cx: &mut Context<Self>,
    ) {
        let patch = match direction {
            PatchDirection::Capture => &mut self.patch.inputs,
            PatchDirection::Playback => &mut self.patch.outputs,
        };

        if channel_idx < patch.len() {
            patch[channel_idx] = entry;
        }

        cx.notify();
    }
}
