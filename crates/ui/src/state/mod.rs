mod meters;
pub use meters::*;

mod session;
pub use session::*;

mod stats;
pub use stats::*;

use gpui_kit::{App, AppContext, Entity, Global, Task, WeakEntity};

pub struct UIState {
    meters: Entity<MetersState>,
    stats: Entity<EngineStats>,

    session: Entity<Session>,

    _tasks: Vec<Task<()>>,
}

impl UIState {
    pub fn meters(&self) -> Entity<MetersState> {
        self.meters.clone()
    }

    pub fn stats(&self) -> Entity<EngineStats> {
        self.stats.clone()
    }

    pub fn session(&self) -> Entity<Session> {
        self.session.clone()
    }

    pub fn strips<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a Strip> {
        self.session.read(cx).strips.iter()
    }
}

impl Global for UIState {}

impl UIState {
    fn get_session(meters: WeakEntity<MetersState>) -> Session {
        let body = reqwest::blocking::get("http://localhost:8080/session")
            .and_then(|res| res.text())
            .expect("Failed to get session");
        let session: SerializableSession =
            serde_json::from_str(&body).expect("Failed to deserialize session");

        Session::from_serializable(session, meters)
    }

    pub fn init(cx: &mut App) {
        let (meters, meters_task) = MetersState::start_socket(cx);
        let (stats, stats_task) = EngineStats::start_fetching(cx);

        let session = cx.new(|_| Self::get_session(meters.downgrade()));

        let _tasks = vec![meters_task, stats_task];

        let this = Self {
            meters,
            stats,
            session,
            _tasks,
        };

        cx.set_global(this);
    }
}
