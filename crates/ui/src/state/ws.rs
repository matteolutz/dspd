use async_tungstenite::{smol::connect_async, tungstenite::Message};
use dspd_api::MeterInfo;
use futures_util::StreamExt;
use gpui_kit::{private::serde, Context, EventEmitter, Task};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSocketMessage {
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
        strip: u32,
        stage_id: u32,
        param_id: u32,
        value: f64,
    },
}

pub struct WebSocket {
    last_generation: Option<u64>,
    _task: Task<()>,
}

pub enum WebSocketEvent {
    EngineRestarted,

    UpdateMeterInfos {
        meters: Vec<MeterInfo>,
    },
    UpdateMeterValues {
        values: Vec<f32>,
    },

    UpdateParam {
        strip: u32,
        stage_id: u32,
        param_id: u32,
        value: f64,
    },
}

impl EventEmitter<WebSocketEvent> for WebSocket {}

impl WebSocket {
    pub(super) fn start(server_url: &reqwest::Url, cx: &mut Context<Self>) -> Self {
        let server_host = server_url
            .host_str()
            .expect("Failed to get host from server URL");
        let server_port = server_url
            .port()
            .map(|p| format!(":{}", p))
            .unwrap_or_default();
        let ws_protocol = server_url.scheme().replace("http", "ws"); // makes "ws" or "wss"

        let ws_address = format!("{}://{}{}/ws?hz=30", ws_protocol, server_host, server_port);

        println!("websocket connecting to: {}", ws_address);

        let _task = cx.spawn(async move |this, cx| {
            let Ok((mut ws_stream, _)) = connect_async(ws_address).await else {
                return;
            };

            while let Some(Ok(message)) = ws_stream.next().await {
                match message {
                    Message::Text(text) => {
                        if let Ok(msg) = serde_json::from_str::<WebSocketMessage>(&text) {
                            match msg {
                                WebSocketMessage::MeterLayout { meters, .. } => {
                                    _ = this.update(cx, |_, cx| {
                                        cx.emit(WebSocketEvent::UpdateMeterInfos { meters })
                                    });
                                }
                                WebSocketMessage::Meters { values } => {
                                    _ = this.update(cx, |_, cx| {
                                        cx.emit(WebSocketEvent::UpdateMeterValues { values })
                                    });
                                }
                                WebSocketMessage::Restarted { generation } => {
                                    let last_generation = this
                                        .read_with(cx, |this, _| this.last_generation)
                                        .ok()
                                        .flatten();

                                    if last_generation.is_some_and(|lg| lg >= generation) {
                                        continue;
                                    }

                                    _ = this.update(cx, |this, cx| {
                                        this.last_generation = Some(generation);
                                        cx.emit(WebSocketEvent::EngineRestarted);
                                    });
                                }
                                WebSocketMessage::ParamChanged {
                                    strip,
                                    stage_id,
                                    param_id,
                                    value,
                                } => {
                                    _ = this.update(cx, |_, cx| {
                                        cx.emit(WebSocketEvent::UpdateParam {
                                            strip,
                                            stage_id,
                                            param_id,
                                            value,
                                        })
                                    });
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        Self {
            last_generation: None,
            _task,
        }
    }
}
