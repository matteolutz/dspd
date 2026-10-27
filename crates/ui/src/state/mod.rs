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

mod client;
pub use client::*;

mod selection;
pub use selection::*;

pub mod tab;

use gpui_kit::{App, AppContext, Entity, Global, Subscription, Task, WeakEntity};

use crate::state::{registry::Registry, tab::UiTab};

pub struct UiState {
    client: Arc<DspdHttpClient>,

    meters: Entity<MetersState>,
    stats: Entity<EngineStats>,
    registry: Entity<Registry>,
    session: Entity<Session>,

    tab_state: Entity<UiTab>,
    strip_selection: Entity<StripSelection>,

    websocket: Entity<WebSocket>,

    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl UiState {
    pub fn client(cx: &App) -> &Arc<DspdHttpClient> {
        &cx.global::<Self>().client
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
}

impl Global for UiState {}

impl UiState {
    fn get_session(
        client: &DspdHttpClient,
        meters: WeakEntity<MetersState>,
        registry: WeakEntity<Registry>,
        cx: &mut App,
    ) -> Session {
        let body = client
            .get("/session")
            .send()
            .and_then(|res| res.text())
            .expect("Failed to get session");
        let session: SerializableSession =
            serde_json::from_str(&body).expect("Failed to deserialize session");

        Session::from_serializable(session, meters, registry, cx)
    }

    fn get_registry(client: &DspdHttpClient) -> Registry {
        let body = client
            .get("/plugins")
            .send()
            .and_then(|res| res.text())
            .expect("Failed to get plugin registry");

        serde_json::from_str(&body).expect("Failed to deserialize plugin registry")
    }

    pub fn init(server_url: impl reqwest::IntoUrl, cx: &mut App) {
        let server_url = server_url.into_url().expect("Failed to parse server URL");
        let client = DspdHttpClient::new(server_url.clone());

        let meters = cx.new(|_| MetersState::default());
        let registry = cx.new(|_| Self::get_registry(&client));
        let session =
            cx.new(|cx| Self::get_session(&client, meters.downgrade(), registry.downgrade(), cx));

        let websocket = cx.new(|cx| WebSocket::start(&server_url, cx));
        let websocket_evt_sub = cx.subscribe(&websocket, |_, evt, cx| match evt {
            WebSocketEvent::EngineRestarted => {
                let new_session = Self::get_session(
                    &cx.global::<Self>().client.clone(),
                    Self::meters(cx).downgrade(),
                    Self::registry(cx).downgrade(),
                    cx,
                );

                Self::session(cx).update(cx, |session, cx| {
                    *session = new_session;
                    cx.notify();
                });
            }
            WebSocketEvent::UpdateMeterInfos { meters } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_layout(meters.iter().cloned());
                    cx.notify();
                });
            }
            WebSocketEvent::UpdateMeterValues { values } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_values(values.clone());
                    cx.notify();
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
        });

        let (stats, stats_task) = EngineStats::start_fetching(&server_url, cx);

        let tab_state = cx.new(|_| UiTab::default());
        let strip_selection = cx.new(|_| StripSelection::default());

        let _tasks = vec![stats_task];
        let _subs = vec![websocket_evt_sub];

        let this = Self {
            client: Arc::new(client),
            meters,
            stats,
            registry,
            session,
            websocket,
            tab_state,
            strip_selection,
            _tasks,
            _subs,
        };

        cx.set_global(this);
    }
}
