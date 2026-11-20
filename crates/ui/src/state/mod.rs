mod ws;
use std::sync::Arc;

pub use ws::*;

mod session;
pub use session::*;

mod stats;
pub use stats::*;

mod meters;
pub use meters::*;

mod registry;
pub use registry::*;

mod client;
pub use client::*;

mod selection;
pub use selection::*;

pub mod tab;

mod patch;
pub use patch::*;

use gpui_kit::{App, AppContext, AsyncApp, Entity, Global, Subscription, Task};

use crate::state::{registry::Registry, tab::UiTab};

pub struct UiState {
    client: Arc<DspdHttpClient>,

    meters: Entity<MetersState>,
    stats: Entity<EngineStats>,
    registry: Entity<Registry>,
    session: Entity<Session>,

    patch: Entity<UiPatchState>,

    tab_state: Entity<UiTab>,
    strip_selection: Entity<StripSelection>,

    websocket: Entity<WebSocket>,

    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl UiState {
    pub fn client(cx: &impl AppContext) -> Arc<DspdHttpClient> {
        cx.read_global(|this: &Self, _| this.client.clone())
    }

    pub fn meters(cx: &App) -> Entity<MetersState> {
        cx.global::<Self>().meters.clone()
    }

    pub fn stats(cx: &App) -> Entity<EngineStats> {
        cx.global::<Self>().stats.clone()
    }

    pub fn session(cx: &App) -> Entity<Session> {
        cx.global::<Self>().session.clone()
    }

    pub fn registry(cx: &App) -> Entity<Registry> {
        cx.global::<Self>().registry.clone()
    }

    pub fn strips<'a>(cx: &'a App) -> impl Iterator<Item = Entity<Strip>> + 'a {
        cx.global::<Self>().session.read(cx).strips.iter().cloned()
    }

    pub fn tab_state(cx: &App) -> Entity<UiTab> {
        cx.global::<Self>().tab_state.clone()
    }

    pub fn strip_selection(cx: &App) -> Entity<StripSelection> {
        cx.global::<Self>().strip_selection.clone()
    }

    pub fn strip_at(idx: usize, cx: &App) -> Option<Entity<Strip>> {
        cx.global::<Self>()
            .session
            .read(cx)
            .strips
            .get(idx)
            .cloned()
    }

    pub fn is_websocket_connected(cx: &App) -> Entity<bool> {
        cx.global::<Self>().websocket.read(cx).is_connected.clone()
    }

    pub fn patch(cx: &App) -> Entity<UiPatchState> {
        cx.global::<Self>().patch.clone()
    }
}

impl Global for UiState {}

impl UiState {
    async fn get_session(client: &DspdHttpClient, cx: &impl AppContext) -> SerializableSession {
        client.get_session(cx).await.expect("Failed to get session")
    }

    async fn get_registry(client: &DspdHttpClient, cx: &App) -> Registry {
        client
            .get_plugins(cx)
            .await
            .expect("Failed to get plugin registry")
    }

    pub async fn init(server_url: impl reqwest::IntoUrl, cx: &mut App) {
        let server_url = server_url.into_url().expect("Failed to parse server URL");
        let client = Arc::new(DspdHttpClient::new(server_url.clone()));

        let meters = cx.new(|_| MetersState::default());

        let registry = Self::get_registry(&client, cx).await;
        let registry = cx.new(|_| registry);

        let session = Self::get_session(&client, cx).await;
        let session = cx.new(|cx| {
            Session::from_serializable(session, meters.downgrade(), registry.downgrade(), cx)
        });

        let websocket = cx.new(|cx| WebSocket::start(&server_url, cx));
        let websocket_evt_sub = cx.subscribe(&websocket, |_, evt, cx| match evt {
            WebSocketEvent::EngineRestarted => {
                cx.spawn(async |cx: &mut AsyncApp| {
                    let new_session = Self::get_session(&UiState::client(cx), cx).await;

                    cx.update(|cx| {
                        let new_session = Session::from_serializable(
                            new_session,
                            Self::meters(cx).downgrade(),
                            Self::registry(cx).downgrade(),
                            cx,
                        );

                        Self::session(cx).update(cx, |session, cx| {
                            *session = new_session;
                            cx.notify();
                        });
                    });
                })
                .detach();
            }
            WebSocketEvent::UpdateMeterInfos { meters } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_layout(meters.iter().cloned(), cx)
                });
            }
            WebSocketEvent::UpdateMeterValues { values } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_values(values.clone(), cx)
                });
            }
            &WebSocketEvent::UpdateParam {
                strip,
                stage_id,
                param_id,
                value,
            } => {
                Self::session(cx).update(cx, |session, cx| {
                    session.set_param(strip, stage_id, param_id, value, cx);
                });
            }
            WebSocketEvent::StageInserted {
                strip,
                stage_id,
                position,
                stage,
            } => {
                Self::session(cx).update(cx, |session, cx| {
                    session.insert_stage(*strip, stage.clone(), *stage_id, *position, cx);
                });
            }
            &WebSocketEvent::StageRemoved { strip, stage_id } => {
                Self::session(cx).update(cx, |session, cx| {
                    session.remove_stage(strip, stage_id, cx);
                });
            }
            &WebSocketEvent::StageMoved {
                strip,
                stage_id,
                new_position,
            } => {
                Self::session(cx).update(cx, |session, cx| {
                    session.move_stage(strip, stage_id, new_position, cx)
                });
            }
            WebSocketEvent::StageOrder { strip, order } => {
                Self::session(cx).update(cx, |session, cx| {
                    session.set_stage_order(*strip, order.clone(), cx);
                });
            }
            WebSocketEvent::PatchChanged => {
                let session = UiState::session(cx);
                let client = UiState::client(cx);
                UiState::patch(cx).update(cx, move |p, cx| {
                    p.update_patch_detached(client, session, cx)
                });
            }
        });

        let patch = UiPatchState::get(&client, &session, cx)
            .await
            .expect("Failed to get patch state");

        let (stats, stats_task) = EngineStats::start_fetching(client.clone(), cx);

        let tab_state = cx.new(|_| UiTab::default());
        let strip_selection = cx.new(|_| StripSelection::default());

        let _tasks = vec![stats_task];
        let _subs = vec![websocket_evt_sub];

        let this = Self {
            client,
            meters,
            stats,
            registry,
            session,
            websocket,
            tab_state,
            strip_selection,
            patch,
            _tasks,
            _subs,
        };

        cx.set_global(this);
    }
}
