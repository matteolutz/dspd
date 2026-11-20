use dspd_api::PatchState;
use gpui_kit::{AppContext, Task};

use crate::state::{
    registry::Registry, AudioDevice, EngineStats, PatchDirection, SerializableSession,
};

pub struct DspdHttpClient {
    server_url: reqwest::Url,
    client: reqwest::blocking::Client,
}

impl DspdHttpClient {
    pub fn new(server_url: impl Into<reqwest::Url>) -> Self {
        let server_url = server_url.into();
        Self {
            server_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn request(
        &self,
        method: reqwest::Method,
        path: impl AsRef<str>,
    ) -> reqwest::blocking::RequestBuilder {
        let url = self
            .server_url
            .join(path.as_ref())
            .expect("Failed join URL");
        self.client.request(method, url)
    }

    pub fn get(&self, path: impl AsRef<str>) -> reqwest::blocking::RequestBuilder {
        self.request(reqwest::Method::GET, path)
    }

    pub fn post(&self, path: impl AsRef<str>) -> reqwest::blocking::RequestBuilder {
        self.request(reqwest::Method::POST, path)
    }

    pub fn delete(&self, path: impl AsRef<str>) -> reqwest::blocking::RequestBuilder {
        self.request(reqwest::Method::DELETE, path)
    }
}

#[derive(serde::Serialize)]
pub struct DspdParamsReq {
    pub strip: usize,
    pub stage: u32,
    pub param_id: u32,
    pub value: f32,
}

impl DspdHttpClient {
    // ---------- GET requests ----------
    pub fn get_session(&self, cx: &impl AppContext) -> Task<reqwest::Result<SerializableSession>> {
        self.get("/session").send_async(cx, |res| res.json())
    }

    pub fn get_plugins(&self, cx: &impl AppContext) -> Task<reqwest::Result<Registry>> {
        self.get("/plugins").send_async(cx, |res| res.json())
    }

    pub fn get_stats(&self, cx: &impl AppContext) -> Task<reqwest::Result<EngineStats>> {
        self.get("/stats").send_async(cx, |res| res.json())
    }

    pub fn get_capture_devices(
        &self,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<Vec<AudioDevice>>> {
        self.get("/patch/capture").send_async(cx, |res| res.json())
    }

    pub fn get_playback_devices(
        &self,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<Vec<AudioDevice>>> {
        self.get("/patch/playback").send_async(cx, |res| res.json())
    }

    pub fn get_current_patch(&self, cx: &impl AppContext) -> Task<reqwest::Result<PatchState>> {
        self.get("/patch/current").send_async(cx, |res| res.json())
    }

    pub fn patch_capture_device(
        &self,
        device_id: &str,
        device_channel_idx: usize,
        dpsd_channel_idx: usize,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        self.post("/patch/capture")
            .json(&serde_json::json!({
                "device_id": device_id,
                "device_channel_idx": device_channel_idx,
                "channel_idx": dpsd_channel_idx,
            }))
            .send_async(cx, |res| res.json())
    }

    pub fn patch_playback_device(
        &self,
        device_id: &str,
        device_channel_idx: usize,
        dpsd_channel_idx: usize,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        self.post("/patch/playback")
            .json(&serde_json::json!({
                "device_id": device_id,
                "device_channel_idx": device_channel_idx,
                "channel_idx": dpsd_channel_idx,
            }))
            .send_async(cx, |res| res.json())
    }

    pub fn patch_device(
        &self,
        direction: PatchDirection,
        device_id: &str,
        device_channel_idx: usize,
        dpsd_channel_idx: usize,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        match direction {
            PatchDirection::Capture => {
                self.patch_capture_device(device_id, device_channel_idx, dpsd_channel_idx, cx)
            }
            PatchDirection::Playback => {
                self.patch_playback_device(device_id, device_channel_idx, dpsd_channel_idx, cx)
            }
        }
    }

    // ---------- POST requests ----------
    pub fn post_params(
        &self,
        req: DspdParamsReq,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        self.post("/params").json(&req).send_async(cx, |_| Ok(()))
    }

    pub fn insert_stage(
        &self,
        strip_idx: usize,
        stage_idx: usize,
        stage_type: &str,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        self.post(format!("/strips/{}/stages", strip_idx))
            .json(&serde_json::json!({
                "index": stage_idx,
                "stage": {
                    "type": stage_type
                }
            }))
            .send_async(cx, |_| Ok(()))
    }

    pub fn remove_stage(
        &self,
        strip_idx: usize,
        stage_id: u32,
        cx: &impl AppContext,
    ) -> Task<reqwest::Result<()>> {
        self.delete(format!("/strips/{}/stages/{}", strip_idx, stage_id))
            .send_async(cx, |_| Ok(()))
    }
}

pub trait RequestBuilderExt {
    fn content_type_json(self) -> Self;

    fn send_async<R>(
        self,
        cx: &impl AppContext,
        blocking_transform: impl FnOnce(reqwest::blocking::Response) -> reqwest::Result<R>
            + Send
            + 'static,
    ) -> Task<reqwest::Result<R>>
    where
        R: Send + 'static;
}

impl RequestBuilderExt for reqwest::blocking::RequestBuilder {
    fn content_type_json(self) -> Self {
        self.header("Content-Type", "application/json")
    }

    fn send_async<R>(
        self,
        cx: &impl AppContext,
        transform: impl FnOnce(reqwest::blocking::Response) -> reqwest::Result<R> + Send + 'static,
    ) -> Task<reqwest::Result<R>>
    where
        R: Send + 'static,
    {
        cx.background_spawn(async move {
            self.send()
                .and_then(|res| res.error_for_status())
                .and_then(transform)
        })
    }
}
