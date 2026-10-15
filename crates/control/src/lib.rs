//! REST control API for a running dspd engine.
//!
//! | method | path            | what it does                                          |
//! |--------|-----------------|-------------------------------------------------------|
//! | GET    | `/health`       | liveness probe                                        |
//! | GET    | `/session`      | the current session as JSON (params included)         |
//! | GET    | `/stats`        | engine load, block count, overruns                    |
//! | POST   | `/params`       | `{"stage":0,"param_id":0,"value":-6.0}` -> audio thread |
//! | POST   | `/session/save` | write the session to the path given at startup        |
//!
//! Parameter changes go through the lock-free queue to the audio thread and
//! are also recorded in the session, so a save always reflects the live state.
//!
//! The API has no authentication. `dspd serve` binds to 127.0.0.1 by default;
//! put it behind something that does authenticate before exposing it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use dspd_engine::{EngineStats, ParamEvent, ParamSender, SessionSpec, StatsSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Everything the HTTP handlers need.
pub struct ControlState {
    params: Mutex<ParamSender>,
    session: Mutex<SessionSpec>,
    stats: Arc<EngineStats>,
    sample_rate: u32,
    session_path: Option<PathBuf>,
}

impl ControlState {
    pub fn new(
        params: ParamSender,
        session: SessionSpec,
        stats: Arc<EngineStats>,
        session_path: Option<PathBuf>,
    ) -> Self {
        Self {
            sample_rate: session.sample_rate,
            params: Mutex::new(params),
            session: Mutex::new(session),
            stats,
            session_path,
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
    stage: u32,
    param_id: u32,
    value: f64,
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn get_session(State(s): State<Shared>) -> Json<SessionSpec> {
    Json(lock(&s.session).clone())
}

async fn get_stats(State(s): State<Shared>) -> Json<StatsSnapshot> {
    Json(s.stats.snapshot(s.sample_rate))
}

async fn set_param(State(s): State<Shared>, Json(req): Json<ParamRequest>) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !req.value.is_finite() {
        return Err(error(StatusCode::BAD_REQUEST, "value must be a finite number"));
    }
    let mut session = lock(&s.session);

    // Validate against a copy so a rejected change leaves the session untouched.
    let mut candidate = session.clone();
    if !candidate.apply_param(req.stage as usize, req.param_id, req.value) {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!("no stage {} with parameter {}", req.stage, req.param_id),
        ));
    }

    let event = ParamEvent { stage: req.stage, param_id: req.param_id, value: req.value };
    if lock(&s.params).send(event).is_err() {
        return Err(error(StatusCode::SERVICE_UNAVAILABLE, "parameter queue is full; retry"));
    }
    *session = candidate;
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "queued" }))))
}

async fn save_session(State(s): State<Shared>) -> Result<Json<Value>, ApiError> {
    let Some(path) = &s.session_path else {
        return Err(error(StatusCode::CONFLICT, "server was started without a session file path"));
    };
    let session = lock(&s.session).clone();
    session
        .save(path)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, format!("saving session failed: {e}")))?;
    Ok(Json(json!({ "status": "saved", "path": path })))
}

/// Build the router. Exposed separately so tests can call it without a socket.
pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/session", get(get_session))
        .route("/stats", get(get_stats))
        .route("/params", post(set_param))
        .route("/session/save", post(save_session))
        .with_state(state)
}

/// Serve the API on an already-bound listener until the future is dropped.
pub async fn serve(listener: TcpListener, state: Shared) -> Result<()> {
    tracing::info!("control API listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request};
    use dspd_engine::{param_channel, GainSpec, ParamReceiver, StageSpec};
    use tower::ServiceExt;

    fn setup(path: Option<PathBuf>) -> (Shared, ParamReceiver) {
        let (tx, rx) = param_channel(2);
        let spec = SessionSpec {
            sample_rate: 48_000,
            channels: 2,
            max_block: 128,
            chain: vec![StageSpec::Gain(GainSpec { gain_db: 0.0 })],
        };
        (Arc::new(ControlState::new(tx, spec, Arc::new(EngineStats::default()), path)), rx)
    }

    async fn call(state: &Shared, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = router(Arc::clone(state)).oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn health_and_stats() {
        let (state, _rx) = setup(None);
        assert_eq!(call(&state, Method::GET, "/health", None).await, (StatusCode::OK, json!({"status": "ok"})));
        let (status, stats) = call(&state, Method::GET, "/stats", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["blocks"], 0);
        assert_eq!(stats["overruns"], 0);
    }

    #[tokio::test]
    async fn param_reaches_queue_and_session() {
        let (state, mut rx) = setup(None);
        let body = json!({"stage": 0, "param_id": 0, "value": -12.5});
        let (status, _) = call(&state, Method::POST, "/params", Some(body)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(rx.pop(), Some(ParamEvent { stage: 0, param_id: 0, value: -12.5 }));

        let (_, session) = call(&state, Method::GET, "/session", None).await;
        assert_eq!(session["chain"][0]["gain_db"], -12.5);
    }

    #[tokio::test]
    async fn bad_stage_is_404_and_changes_nothing() {
        let (state, mut rx) = setup(None);
        let body = json!({"stage": 9, "param_id": 0, "value": 1.0});
        let (status, err) = call(&state, Method::POST, "/params", Some(body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(err["error"].as_str().unwrap().contains("no stage 9"));
        assert_eq!(rx.pop(), None);
    }

    #[tokio::test]
    async fn full_queue_is_503_and_session_unchanged() {
        let (state, _rx) = setup(None); // capacity 2, never drained
        for _ in 0..2 {
            let b = json!({"stage": 0, "param_id": 0, "value": -1.0});
            assert_eq!(call(&state, Method::POST, "/params", Some(b)).await.0, StatusCode::ACCEPTED);
        }
        let b = json!({"stage": 0, "param_id": 0, "value": -99.0});
        assert_eq!(call(&state, Method::POST, "/params", Some(b)).await.0, StatusCode::SERVICE_UNAVAILABLE);
        let (_, session) = call(&state, Method::GET, "/session", None).await;
        assert_eq!(session["chain"][0]["gain_db"], -1.0);
    }

    #[tokio::test]
    async fn save_requires_configured_path_and_writes_live_state() {
        let (state, _rx) = setup(None);
        assert_eq!(call(&state, Method::POST, "/session/save", None).await.0, StatusCode::CONFLICT);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.json");
        let (state, _rx) = setup(Some(path.clone()));
        let b = json!({"stage": 0, "param_id": 0, "value": -3.0});
        call(&state, Method::POST, "/params", Some(b)).await;
        assert_eq!(call(&state, Method::POST, "/session/save", None).await.0, StatusCode::OK);

        let saved = SessionSpec::load(&path).unwrap();
        match &saved.chain[0] {
            StageSpec::Gain(g) => assert_eq!(g.gain_db, -3.0),
            other => panic!("unexpected {other:?}"),
        }
    }
}
