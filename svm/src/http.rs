use crate::decode::{decode, Encoding};
use crate::engine::Engine;
use crate::project::project;
use crate::rpc::{aval_meta, handle};
use crate::source::AccountSource;
use crate::upstream::Upstream;
use axum::{extract::State, routing::{get, post}, Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct App<S> {
    pub engine: Engine<S>,
    pub upstream: Upstream,
}

async fn rpc<S: AccountSource>(State(app): State<Arc<App<S>>>, Json(body): Json<Value>) -> Json<Value> {
    Json(handle(&app.engine, &app.upstream, body).await)
}

async fn project_route<S: AccountSource>(State(app): State<Arc<App<S>>>, Json(body): Json<Value>) -> Json<Value> {
    let fail = |code: i64, message: String| Json(json!({"ok": false, "error": {"code": code, "message": message}}));
    let Some(tx) = body.get("transaction").and_then(Value::as_str) else {
        return fail(-32602, "transaction (base64) is required".into());
    };
    let decoded = match decode(tx, Encoding::Base64) {
        Ok(d) => d,
        Err(e) => return fail(-32602, e.to_string()),
    };
    let fresh = body.get("fresh").and_then(Value::as_bool).unwrap_or(false);
    match app.engine.simulate(decoded, fresh).await {
        Ok(r) => Json(json!({
            "ok": r.outcome.err.is_none(),
            "err": r.outcome.err.as_ref().map(|e| serde_json::to_value(e).unwrap_or(json!(e.to_string()))),
            "unitsConsumed": r.outcome.units,
            "logs": r.outcome.logs,
            "projection": project(&r.pre, &r.outcome.post),
            "aval": aval_meta(&r),
        })),
        Err(e) => fail(e.code(), e.to_string()),
    }
}

pub fn router<S: AccountSource>(app: Arc<App<S>>) -> Router {
    Router::new()
        .route("/", post(rpc::<S>))
        .route("/v1/project", post(project_route::<S>))
        .route("/health", get(|| async { Json(json!({"ok": true})) }))
        .with_state(app)
}
