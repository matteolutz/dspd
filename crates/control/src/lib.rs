//! REST control API for a running dspd engine.
//!
//! | method | path            | what it does                                          |
//! |--------|-----------------|-------------------------------------------------------|
//! | GET    | `/health`       | liveness probe                                        |
//! | GET    | `/session`      | the current session as JSON (params included)         |
//! | GET    | `/stats`        | engine load, block count, overruns                    |
//! | POST   | `/params`       | `{"strip":0,"stage":0,"param_id":0,"value":-6.0}` -> audio thread |
//! | POST   | `/session/save` | write the session to the path given at startup        |
//! | GET    | `/plugins`      | native plugins this server offers, with their parameters |
//! | GET    | `/meters`       | every live meter value, with strip, stage and name    |
//! | GET    | `/ws?hz=30`     | WebSocket: meter values pushed at `hz` (1-100)        |
//!
//! # Meters
//!
//! Processors publish live values (levels, gain reduction, ...) into a shared
//! lock-free bank; see `dspd_engine::MeterBank`. `/meters` returns one snapshot:
//!
//! ```json
//! {"meters": [{"slot": 0, "strip": 1, "stage": 0, "param_id": 1, "name": "Peak", "value": 0.49}]}
//! ```
//!
//! The WebSocket first sends `{"type":"hello","hz":30,"meters":[...same metadata...]}`
//! and then `{"type":"meters","values":[...]}` every tick, with `values[i]` belonging to
//! the meter whose `slot` is `i`. Non-finite values arrive as `null`.
//!
//! Parameter changes go through the lock-free queue to the audio thread and
//! are also recorded in the session, so a save always reflects the live state.
//!
//! The API has no authentication. `dspd serve` binds to 127.0.0.1 by default;
//! put it behind something that does authenticate before exposing it.

mod ws;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use dspd_engine::{
    EditError, Editor, EngineStats, MeterBank, MeterInfo, ParamEvent, ParamSender, Registry,
    SessionSpec, StageSpec, StatsSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::ws::{WebSocketEvent, WebSocketExt};

/// Everything that belongs to one running engine. A restart hands out a new set.
pub struct EngineHandles {
    pub params: ParamSender,
    pub editor: Option<Editor>,
    pub stats: Arc<EngineStats>,
    pub meters: Arc<MeterBank>,
    pub meter_infos: Arc<[MeterInfo]>,
}

/// Everything the HTTP handlers need.
pub struct ControlState {
    params: Mutex<ParamSender>,
    session: Mutex<SessionSpec>,
    registry: Arc<Registry>,
    stats: Mutex<Arc<EngineStats>>,
    session_path: Option<PathBuf>,
    meters: Mutex<Arc<MeterBank>>,
    /// The meters that exist right now; replaced when edits add or remove some.
    meter_infos: Mutex<Arc<[MeterInfo]>>,
    /// Bumped whenever `meter_infos` is replaced, so WebSocket clients know to re-read it.
    meter_version: AtomicU64,
    editor: Mutex<Option<Editor>>,
    /// Counts engine restarts (backend changes). Stage ids start over after one.
    generation: AtomicU64,
    /// False if a restart failed and could not be undone: there is no engine to talk to.
    engine_up: AtomicBool,
    /// Requests that talk to the engine hold this for reading; a restart holds it for
    /// writing, so nothing reaches a half-replaced engine.
    gate: tokio::sync::RwLock<()>,

    event_tx: tokio::sync::broadcast::Sender<WebSocketEvent>,
}

impl ControlState {
    pub fn new(
        params: ParamSender,
        session: SessionSpec,
        registry: Arc<Registry>,
        stats: Arc<EngineStats>,
        session_path: Option<PathBuf>,
    ) -> Self {
        Self {
            params: Mutex::new(params),
            session: Mutex::new(session),
            registry,
            stats: Mutex::new(stats),
            session_path,
            meters: Mutex::new(MeterBank::new(0)),
            meter_infos: Mutex::new(Arc::from(Vec::new())),
            meter_version: AtomicU64::new(0),
            editor: Mutex::new(None),
            generation: AtomicU64::new(0),
            engine_up: AtomicBool::new(true),
            gate: tokio::sync::RwLock::new(()),
            event_tx: tokio::sync::broadcast::channel(16).0,
        }
    }

    /// Serve the engine's live meters (`Engine::meters()` and `Engine::meter_infos()`).
    pub fn with_meters(mut self, meters: Arc<MeterBank>, infos: Arc<[MeterInfo]>) -> Self {
        self.meters = Mutex::new(meters);
        self.meter_infos = Mutex::new(infos);
        self
    }

    /// Allow inserting, removing and reordering plugins while running.
    /// Build the editor from the rack before it moves into the engine.
    pub fn with_editor(mut self, editor: Editor) -> Self {
        self.editor = Mutex::new(Some(editor));
        self
    }

    fn meter_infos(&self) -> Arc<[MeterInfo]> {
        Arc::clone(&lock(&self.meter_infos))
    }

    fn meters(&self) -> Arc<MeterBank> {
        Arc::clone(&lock(&self.meters))
    }

    fn publish_meter_infos(&self, infos: Arc<[MeterInfo]>) {
        *lock(&self.meter_infos) = infos;
        self.meter_version.fetch_add(1, Ordering::Release);
    }

    /// Switch to a freshly started engine.
    fn install(&self, h: EngineHandles) {
        *lock(&self.params) = h.params;
        *lock(&self.editor) = h.editor;
        *lock(&self.stats) = h.stats;
        *lock(&self.meters) = h.meters;
        self.engine_up.store(true, Ordering::Release);
        // Before the layout, so a client that sees the new layout also sees the new generation.
        self.generation.fetch_add(1, Ordering::Release);
        self.publish_meter_infos(h.meter_infos);
    }

    /// 503 while there is no engine (a failed restart that could not be undone).
    fn require_engine(&self) -> Result<(), ApiError> {
        if self.engine_up.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "the audio engine is not running; fix the settings with PUT /backend",
            ))
        }
    }
}

/// A poisoned lock only means another handler panicked; the data is still usable.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Shared = Arc<ControlState>;
type ApiError = (StatusCode, Json<Value>);

fn error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

#[derive(Debug, Deserialize)]
struct ParamRequest {
    /// Index into the session's `strips` list. Required on purpose: with many
    /// strips, a forgotten field must not silently hit strip 0.
    strip: u32,
    /// The stage's id (see the module docs): its position until the first edit.
    stage: u32,
    param_id: u32,
    value: f64,
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// The session as JSON, with each stage's id added as `id`.
async fn get_session(State(s): State<Shared>) -> Json<Value> {
    let session = lock(&s.session);
    let mut value = serde_json::to_value(&*session).unwrap_or(Value::Null);
    let editor = lock(&s.editor);

    // TODO: this is kinda hacky, maybe introduce a new type for this
    if let Some(strips) = value.get_mut("strips").and_then(Value::as_array_mut) {
        for (si, strip) in strips.iter_mut().enumerate() {
            let ids = editor.as_ref().and_then(|e| e.stage_ids(si));
            if let Some(chain) = strip.get_mut("chain").and_then(Value::as_array_mut) {
                for (ti, stage) in chain.iter_mut().enumerate() {
                    // Without an editor nothing can move, so the id is the position.
                    let id = ids
                        .as_ref()
                        .and_then(|v| v.get(ti).copied())
                        .unwrap_or(ti as u32);
                    if let Some(obj) = stage.as_object_mut() {
                        obj.insert("id".into(), json!(id));
                    }
                }
            }
        }
    }

    Json(value)
}

async fn get_stats(State(s): State<Shared>) -> Json<StatsSnapshot> {
    let sample_rate = lock(&s.session).sample_rate;
    let stats = Arc::clone(&lock(&s.stats));
    Json(stats.snapshot(sample_rate))
}

async fn set_param(
    State(s): State<Shared>,
    Json(req): Json<ParamRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !req.value.is_finite() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "value must be a finite number",
        ));
    }
    let _gate = s.gate.read().await;
    s.require_engine()?;
    let mut session = lock(&s.session);

    // Validate against a copy so a rejected change leaves the session untouched.
    let mut candidate = session.clone();
    // The spec lists stages by position; the request names one by id.
    let position = match &*lock(&s.editor) {
        Some(editor) => editor.index_of(req.strip as usize, req.stage),
        None => Some(req.stage as usize),
    };
    let applied = position.is_some_and(|p| {
        candidate.apply_param(&s.registry, req.strip as usize, p, req.param_id, req.value)
    });
    if !applied {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!(
                "no strip {} stage {} with parameter {}",
                req.strip, req.stage, req.param_id
            ),
        ));
    }

    let event = ParamEvent {
        strip: req.strip,
        stage: req.stage,
        param_id: req.param_id,
        value: req.value,
    };
    if lock(&s.params).send(event).is_err() {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "parameter queue is full; retry",
        ));
    }
    *session = candidate;

    _ = s.event_tx.send(WebSocketEvent::ParamChanged {
        strip: req.strip,
        stage_id: req.stage,
        param_id: req.param_id,
        value: req.value,
    });

    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "queued" }))))
}

async fn save_session(State(s): State<Shared>) -> Result<Json<Value>, ApiError> {
    let Some(path) = &s.session_path else {
        return Err(error(
            StatusCode::CONFLICT,
            "server was started without a session file path",
        ));
    };
    let session = lock(&s.session).clone();
    session.save(path).map_err(|e| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("saving session failed: {e}"),
        )
    })?;
    Ok(Json(json!({ "status": "saved", "path": path })))
}

async fn get_plugins(State(s): State<Shared>) -> Json<Value> {
    let plugins: Vec<Value> = s
        .registry
        .iter()
        .map(|d| {
            let params: Vec<Value> = d
                .params
                .iter()
                .map(|p| {
                    json!({ "id": p.id, "key": p.key, "name": p.name, "min": p.min, "max": p.max, "default": p.default })
                })
                .collect();
            json!({ "id": d.id, "name": d.name, "description": d.description, "params": params })
        })
        .collect();
    Json(json!({ "plugins": plugins }))
}

// ---------------------------------------------------------------------------
// live editing
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct InsertRequest {
    /// Position in the chain; the end if absent.
    index: Option<usize>,
    stage: StageSpec,
}

#[derive(Deserialize)]
struct MoveRequest {
    index: usize,
}

#[derive(Deserialize)]
struct OrderRequest {
    order: Vec<u32>,
}

fn edit_status(e: &EditError) -> StatusCode {
    match e {
        EditError::NoStrip(_) | EditError::NoStage { .. } => StatusCode::NOT_FOUND,
        EditError::BadIndex { .. }
        | EditError::BadOrder(_)
        | EditError::Invalid(_)
        | EditError::TooManyStages => StatusCode::BAD_REQUEST,
        EditError::NoMeterSlots => StatusCode::INSUFFICIENT_STORAGE,
        EditError::QueueFull => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Run an edit on a blocking thread (loading a plugin can take a while) with the
/// session and editor locked, then publish the new meter list.
async fn edit<T: Send + 'static>(
    s: Shared,
    f: impl FnOnce(&mut Editor, &mut SessionSpec) -> Result<T, EditError> + Send + 'static,
) -> Result<T, ApiError> {
    let gate = s.gate.read().await;
    s.require_engine()?;
    let inner = Arc::clone(&s);
    let out = tokio::task::spawn_blocking(move || {
        let s = inner;
        // Lock order everywhere: session, then editor.
        let mut session = lock(&s.session);
        let mut guard = lock(&s.editor);
        let Some(editor) = guard.as_mut() else {
            return Err(error(
                StatusCode::NOT_IMPLEMENTED,
                "this server was started without live editing",
            ));
        };
        let out = f(editor, &mut session).map_err(|e| error(edit_status(&e), e.to_string()))?;
        s.publish_meter_infos(editor.meter_infos());
        Ok(out)
    })
    .await
    .unwrap_or_else(|_| {
        Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the edit task failed",
        ))
    });
    drop(gate);
    out
}

async fn insert_stage(
    State(s): State<Shared>,
    Path(strip): Path<usize>,
    Json(req): Json<InsertRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let done = edit(s, move |ed, session| {
        ed.insert(session, strip, req.index, req.stage)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": done.id,
            "index": done.index,
            "latency_frames": done.latency_frames,
            "rack_latency_frames": done.rack_latency_frames,
        })),
    ))
}

async fn remove_stage(
    State(s): State<Shared>,
    Path((strip, id)): Path<(usize, u32)>,
) -> Result<Json<Value>, ApiError> {
    edit(s, move |ed, session| ed.remove(session, strip, id)).await?;
    Ok(Json(json!({ "removed": id })))
}

async fn move_stage(
    State(s): State<Shared>,
    Path((strip, id)): Path<(usize, u32)>,
    Json(req): Json<MoveRequest>,
) -> Result<Json<Value>, ApiError> {
    edit(s, move |ed, session| {
        ed.move_stage(session, strip, id, req.index)
    })
    .await?;
    Ok(Json(json!({ "moved": id, "index": req.index })))
}

async fn set_order(
    State(s): State<Shared>,
    Path(strip): Path<usize>,
    Json(req): Json<OrderRequest>,
) -> Result<Json<Value>, ApiError> {
    let order = req.order.clone();
    edit(s, move |ed, session| ed.reorder(session, strip, &req.order)).await?;
    Ok(Json(json!({ "order": order })))
}

/// Free removed plugins promptly even when no further edits come in.
async fn reap_loop(s: Shared) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tick.tick().await;
        // try_lock: an edit in progress will reap by itself when it sends.
        if let Ok(mut guard) = s.editor.try_lock() {
            if let Some(ed) = guard.as_mut() {
                ed.reap();
            }
        }
    }
}

/// One meter as `/meters` reports it: its metadata plus the current value.
#[derive(Serialize)]
struct MeterReading<'a> {
    #[serde(flatten)]
    info: &'a MeterInfo,
    value: f32,
}

async fn get_meters(State(s): State<Shared>) -> Json<Value> {
    let infos = s.meter_infos();
    let bank = s.meters();
    let readings: Vec<_> = infos
        .iter()
        .map(|info| MeterReading {
            info,
            value: bank.get(info.slot),
        })
        .collect();
    Json(json!({ "meters": readings }))
}

const DEFAULT_METER_HZ: u32 = 30;
const MAX_METER_HZ: u32 = 100;

#[derive(Deserialize)]
struct WsQuery {
    hz: Option<u32>,
}

async fn ws(
    ws: WebSocketUpgrade,
    State(s): State<Shared>,
    Query(q): Query<WsQuery>,
) -> impl IntoResponse {
    let hz = q.hz.unwrap_or(DEFAULT_METER_HZ).clamp(1, MAX_METER_HZ);
    ws.on_upgrade(move |socket| ws_stream(socket, s, hz))
}

/// Push the meter bank to one client at a fixed rate until it disconnects.
/// Reads are relaxed atomic loads, so any number of clients cost the audio thread nothing.
/// When edits add or remove meters, a `layout` frame carries the new list.
async fn ws_stream(mut socket: WebSocket, s: Shared, hz: u32) {
    let mut generation = s.generation.load(Ordering::Acquire);
    let mut version = s.meter_version.load(Ordering::Acquire);
    let mut infos = s.meter_infos();

    let mut event_rx = s.event_tx.subscribe();

    if socket
        .send_dspd_event(WebSocketEvent::MeterLayout {
            meters: infos.to_vec(),
            hz: Some(hz),
        })
        .await
        .is_err()
    {
        return;
    }

    let mut tick = tokio::time::interval(Duration::from_millis(1000 / u64::from(hz)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut values: Vec<f32> = Vec::new();

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let current_generation = s.generation.load(Ordering::Acquire);
                if current_generation != generation {
                    generation = current_generation;
                    _ = socket.send_dspd_event(WebSocketEvent::Restarted { generation }).await;
                }

                let current = s.meter_version.load(Ordering::Acquire);
                if current != version {
                    version = current;
                    infos = s.meter_infos();
                    _ = socket
                        .send_dspd_event(WebSocketEvent::MeterLayout {
                            meters: infos.to_vec(),
                            hz: None,
                        });
                }

                // values[i] belongs to the meter in slot i; slots past the last used one are not sent.
                let bank = s.meters();
                let used = infos.iter().map(|i| i.slot + 1).max().unwrap_or(0).min(bank.len());
                values.clear();
                values.extend((0..used).map(|i| bank.get(i)));

                _ = socket.send_dspd_event(WebSocketEvent::Meters { values: values.clone() }).await;
            }
            event = event_rx.recv() => match event {
                Ok(evt) => {
                    _ = socket.send_dspd_event(evt);
                }
                Err(_) => {
                    // Handle channel closed / lagged error if needed
                }
            },
            incoming = socket.recv() => match incoming {
                // Closed, errored, or a close frame: stop. Anything else the client sends is ignored.
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Build the router. Exposed separately so tests can call it without a socket.
pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/session", get(get_session))
        .route("/stats", get(get_stats))
        .route("/params", post(set_param))
        .route("/session/save", post(save_session))
        .route("/plugins", get(get_plugins))
        .route("/meters", get(get_meters))
        .route("/strips/{strip}/stages", post(insert_stage))
        .route("/strips/{strip}/stages/{id}", delete(remove_stage))
        .route("/strips/{strip}/stages/{id}/move", post(move_stage))
        .route("/strips/{strip}/order", put(set_order))
        .route("/ws", get(ws))
        .with_state(state)
}

/// Serve the API on an already-bound listener until the future is dropped.
pub async fn serve(listener: TcpListener, state: Shared) -> Result<()> {
    tracing::info!("control API listening on http://{}", listener.local_addr()?);
    tokio::spawn(reap_loop(Arc::clone(&state)));
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request};
    use dspd_engine::{param_channel, NativeSpec, ParamReceiver, StageSpec, StripSpec, Width};
    use tower::ServiceExt;

    fn setup(path: Option<PathBuf>) -> (Shared, ParamReceiver) {
        let (tx, rx) = param_channel(2);
        let strip = |input: usize, width: Width| StripSpec {
            name: None,
            width,
            input,
            output: None,
            chain: vec![StageSpec::Native(
                NativeSpec::new("gain").with("gain_db", 0.0),
            )],
        };
        // strip 0: mono on channel 0; strip 1: stereo on channels 1-2
        let spec = SessionSpec {
            sample_rate: 48_000,
            max_block: 128,
            inputs: None,
            outputs: None,
            strips: vec![strip(0, Width::Mono), strip(1, Width::Stereo)],
        };
        let registry = Registry::from_descriptors(&[&dspd_plugin_gain::DESCRIPTOR]).unwrap();

        (
            Arc::new(ControlState::new(
                tx,
                spec,
                Arc::new(registry),
                Arc::new(EngineStats::default()),
                path,
            )),
            rx,
        )
    }

    async fn call(
        state: &Shared,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = router(Arc::clone(state))
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn health_and_stats() {
        let (state, _rx) = setup(None);
        assert_eq!(
            call(&state, Method::GET, "/health", None).await,
            (StatusCode::OK, json!({"status": "ok"}))
        );
        let (status, stats) = call(&state, Method::GET, "/stats", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["blocks"], 0);
        assert_eq!(stats["overruns"], 0);
    }

    #[tokio::test]
    async fn param_reaches_queue_and_session() {
        let (state, mut rx) = setup(None);
        let body = json!({"strip": 1, "stage": 0, "param_id": 0, "value": -12.5});
        let (status, _) = call(&state, Method::POST, "/params", Some(body)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            rx.pop(),
            Some(ParamEvent {
                strip: 1,
                stage: 0,
                param_id: 0,
                value: -12.5
            })
        );

        // Only the addressed strip changes in the session.
        let (_, session) = call(&state, Method::GET, "/session", None).await;
        assert_eq!(session["strips"][1]["chain"][0]["gain_db"], -12.5);
        assert_eq!(session["strips"][0]["chain"][0]["gain_db"], 0.0);
    }

    #[tokio::test]
    async fn bad_address_is_404_and_changes_nothing() {
        let (state, mut rx) = setup(None);
        for body in [
            json!({"strip": 0, "stage": 9, "param_id": 0, "value": 1.0}), // no such stage
            json!({"strip": 7, "stage": 0, "param_id": 0, "value": 1.0}), // no such strip
        ] {
            let (status, err) = call(&state, Method::POST, "/params", Some(body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert!(err["error"].as_str().unwrap().contains("no strip"));
        }
        assert_eq!(rx.pop(), None);
    }

    #[tokio::test]
    async fn missing_strip_field_is_rejected() {
        let (state, mut rx) = setup(None);
        let body = json!({"stage": 0, "param_id": 0, "value": 1.0});
        let (status, _) = call(&state, Method::POST, "/params", Some(body)).await;
        assert!(status.is_client_error(), "{status}");
        assert_eq!(rx.pop(), None);
    }

    #[tokio::test]
    async fn full_queue_is_503_and_session_unchanged() {
        let (state, _rx) = setup(None); // capacity 2, never drained
        for _ in 0..2 {
            let b = json!({"strip": 0, "stage": 0, "param_id": 0, "value": -1.0});
            assert_eq!(
                call(&state, Method::POST, "/params", Some(b)).await.0,
                StatusCode::ACCEPTED
            );
        }
        let b = json!({"strip": 0, "stage": 0, "param_id": 0, "value": -99.0});
        assert_eq!(
            call(&state, Method::POST, "/params", Some(b)).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (_, session) = call(&state, Method::GET, "/session", None).await;
        assert_eq!(session["strips"][0]["chain"][0]["gain_db"], -1.0);
    }

    #[tokio::test]
    async fn save_requires_configured_path_and_writes_live_state() {
        let (state, _rx) = setup(None);
        assert_eq!(
            call(&state, Method::POST, "/session/save", None).await.0,
            StatusCode::CONFLICT
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.json");
        let (state, _rx) = setup(Some(path.clone()));
        let b = json!({"strip": 0, "stage": 0, "param_id": 0, "value": -3.0});
        call(&state, Method::POST, "/params", Some(b)).await;
        assert_eq!(
            call(&state, Method::POST, "/session/save", None).await.0,
            StatusCode::OK
        );

        let saved = SessionSpec::load(&path).unwrap();
        match &saved.strips[0].chain[0] {
            StageSpec::Native(n) => assert_eq!(n.settings["gain_db"], -3.0),
            other => panic!("unexpected {other:?}"),
        }
    }
}
