use axum::extract::ws::{Message, WebSocket};
use dspd_engine::{MeterInfo, StageSpec};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSocketEvent {
    MeterLayout {
        meters: Vec<MeterInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hz: Option<u32>,
    },

    Meters {
        values: Vec<f32>,
    },

    Restarted {
        generation: u64,
    },

    ParamChanged {
        strip: usize,
        stage_id: u32,
        param_id: u32,
        value: f64,
    },

    StageInserted {
        strip: usize,
        stage_id: u32,
        position: usize,
        stage: StageSpec,
    },
    StageRemoved {
        strip: usize,
        stage_id: u32,
    },
    StageMoved {
        strip: usize,
        stage_id: u32,
        new_position: usize,
    },
    StageOrder {
        strip: usize,
        order: Vec<u32>,
    },
}

pub trait WebSocketExt {
    type Error;
    async fn send_dspd_event(&mut self, event: WebSocketEvent) -> Result<(), Self::Error>;
}
impl WebSocketExt for WebSocket {
    type Error = axum::Error;

    async fn send_dspd_event(&mut self, event: WebSocketEvent) -> Result<(), Self::Error> {
        let string = serde_json::to_string(&event).map_err(axum::Error::new)?;
        self.send(Message::text(string)).await
    }
}
