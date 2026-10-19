use std::ops::Deref;

use async_tungstenite::{smol::connect_async, tungstenite::Message};
use dspd_api::MeterInfo;
use futures_util::StreamExt;
use gpui_kit::{private::serde, App, AppContext, Context, Entity, Task};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeterWebSocketMessage {
    Hello { hz: u32, meters: Vec<MeterInfo> },
    Meters { values: Vec<f32> },
}

#[derive(Debug)]
pub struct MeterInfoWithValue {
    info: MeterInfo,
    pub(crate) value: Option<f32>,
}

impl From<MeterInfo> for MeterInfoWithValue {
    fn from(info: MeterInfo) -> Self {
        Self { info, value: None }
    }
}

impl Deref for MeterInfoWithValue {
    type Target = MeterInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[derive(Debug, Default)]
pub struct MetersState {
    meters: Vec<MeterInfoWithValue>,
}

impl Deref for MetersState {
    type Target = [MeterInfoWithValue];

    fn deref(&self) -> &Self::Target {
        &self.meters
    }
}

impl MetersState {
    fn update_values(&mut self, values: Vec<f32>) {
        if self.meters.len() != values.len() {
            return;
        }

        for (meter, value) in self.meters.iter_mut().zip(values) {
            meter.value = Some(value);
        }
    }

    pub(crate) fn meters_for_stage<'a>(
        &'a self,
        strip_idx: usize,
        stage_id: u32,
    ) -> impl Iterator<Item = (u32, &'a String, Option<f32>)> {
        self.meters.iter().filter_map(move |m| {
            if m.strip == strip_idx && m.stage == stage_id {
                Some((m.param_id, &m.name, m.value))
            } else {
                None
            }
        })
    }
}

impl MetersState {
    pub(super) fn start_socket(cx: &mut App) -> (Entity<Self>, Task<()>) {
        let meters_entity = cx.new(|_| Self::default());

        let task = cx.spawn({
            let meters_entity = meters_entity.clone();

            async move |cx| {
                let Ok((mut ws_stream, _)) = connect_async("ws://localhost:8080/ws?hz=30").await
                else {
                    return;
                };

                while let Some(Ok(message)) = ws_stream.next().await {
                    match message {
                        Message::Text(text) => {
                            if let Ok(msg) = serde_json::from_str::<MeterWebSocketMessage>(&text) {
                                match msg {
                                    MeterWebSocketMessage::Hello { meters, .. } => {
                                        cx.update_entity(
                                            &meters_entity,
                                            |this: &mut Self, cx: &mut Context<Self>| {
                                                this.meters =
                                                    meters.into_iter().map(|m| m.into()).collect();
                                                cx.notify();
                                            },
                                        );
                                    }
                                    MeterWebSocketMessage::Meters { values } => {
                                        cx.update_entity(
                                            &meters_entity,
                                            |this: &mut Self, cx: &mut Context<Self>| {
                                                this.update_values(values);
                                                cx.notify();
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        });

        (meters_entity, task)
    }
}
