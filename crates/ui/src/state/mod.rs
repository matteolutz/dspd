mod ws;
pub use ws::*;

mod session;
pub use session::*;

mod stats;
pub use stats::*;

mod meters;
pub use meters::*;

use gpui_kit::{App, AppContext, Entity, Global, Subscription, Task, WeakEntity};

pub struct UiState {
    server_url: reqwest::Url,

    meters: Entity<MetersState>,
    stats: Entity<EngineStats>,

    session: Entity<Session>,

    websocket: Entity<WebSocket>,

    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl UiState {
    pub fn meters(cx: &App) -> Entity<MetersState> {
        cx.global::<Self>().meters.clone()
    }

    pub fn stats(cx: &App) -> Entity<EngineStats> {
        cx.global::<Self>().stats.clone()
    }

    pub fn session(cx: &App) -> Entity<Session> {
        cx.global::<Self>().session.clone()
    }

    pub fn strips<'a>(cx: &'a App) -> impl Iterator<Item = &'a Strip> {
        cx.global::<Self>().session.read(cx).strips.iter()
    }

    pub fn strip_at<'a>(idx: usize, cx: &'a App) -> Option<&'a Strip> {
        cx.global::<Self>().session.read(cx).strips.get(idx)
    }
}

impl Global for UiState {}

impl UiState {
    fn get_session(server_url: &reqwest::Url, meters: WeakEntity<MetersState>) -> Session {
        let body = reqwest::blocking::get(server_url.join("/session").expect("Failed to join URL"))
            .and_then(|res| res.text())
            .expect("Failed to get session");
        let session: SerializableSession =
            serde_json::from_str(&body).expect("Failed to deserialize session");

        Session::from_serializable(session, meters)
    }

    pub fn init(server_url: impl reqwest::IntoUrl, cx: &mut App) {
        let server_url = server_url.into_url().expect("Failed to parse server URL");

        let meters = cx.new(|_| MetersState::default());
        let session = cx.new(|_| Self::get_session(&server_url, meters.downgrade()));

        let websocket = cx.new(|cx| WebSocket::start(&server_url, cx));
        let websocket_evt_sub = cx.subscribe(&websocket, |_, evt, cx| match evt {
            WebSocketEvent::EngineRestarted => {
                let new_session = Self::get_session(
                    &cx.global::<Self>().server_url,
                    Self::meters(cx).downgrade(),
                );

                Self::session(cx).update(cx, |session, cx| {
                    *session = new_session;
                    cx.notify();
                });
            }
            WebSocketEvent::UpdateMeterInfos { meters } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_layout(meters.into_iter().cloned());
                    cx.notify();
                });
            }
            WebSocketEvent::UpdateMeterValues { values } => {
                Self::meters(cx).update(cx, |meter_state, cx| {
                    meter_state.update_values(values.clone());
                    cx.notify();
                });
            }
            WebSocketEvent::UpdateParam {
                strip,
                stage_id,
                param_id,
                value,
            } => {}
        });

        let (stats, stats_task) = EngineStats::start_fetching(&server_url, cx);

        let _tasks = vec![stats_task];
        let _subs = vec![websocket_evt_sub];

        let this = Self {
            server_url,
            meters,
            stats,
            session,
            websocket,
            _tasks,
            _subs,
        };

        cx.set_global(this);
    }
}
