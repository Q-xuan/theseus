//! Loopback HTTP + WebSocket → `pi --mode rpc` stdio.
//!
//! Incoming JSON-RPC methods are pi commands (or `list_sessions`).
//! Outgoing pi events are mapped to Thread / Turn / Item notifications.
//! Bind 127.0.0.1 only. The shell does not run the agent loop.

use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::map::{
    enrich_result, is_pi_command, is_theseus_app_server_method, EventMap,
};
use crate::sessions::list_sessions;
use crate::sidecar::Sidecar;

const INDEX: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");

#[derive(Default)]
struct SessionHint {
    id: String,
    file: Option<String>,
    name: Option<String>,
    model: Option<String>,
}

#[derive(Clone)]
struct BridgeState {
    sidecar: Arc<Mutex<Sidecar>>,
    out: broadcast::Sender<String>,
    map: Arc<Mutex<EventMap>>,
    session: Arc<Mutex<SessionHint>>,
}

pub async fn serve(
    addr: SocketAddr,
    sidecar: Sidecar,
    rx: Receiver<String>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_listener(listener, sidecar, rx).await
}

pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    sidecar: Sidecar,
    rx: Receiver<String>,
) -> std::io::Result<()> {
    let (out, _) = broadcast::channel::<String>(512);
    let state = Arc::new(BridgeState {
        sidecar: Arc::new(Mutex::new(sidecar)),
        out: out.clone(),
        map: Arc::new(Mutex::new(EventMap::new())),
        session: Arc::new(Mutex::new(SessionHint::default())),
    });
    pump_stdout(state.clone(), rx);
    axum::serve(listener, router(state)).await
}

fn pump_stdout(state: Arc<BridgeState>, rx: Receiver<String>) {
    tokio::task::spawn_blocking(move || {
        while let Ok(line) = rx.recv() {
            for outbound in handle_pi_line(&state, &line) {
                let _ = state.out.send(outbound);
            }
        }
    });
}

fn handle_pi_line(state: &BridgeState, line: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    if v.get("type").and_then(|t| t.as_str()) == Some("response") {
        remember_state(state, &v);
        let session_id = state
            .session
            .lock()
            .ok()
            .map(|s| {
                if s.id.is_empty() {
                    "session".into()
                } else {
                    s.id.clone()
                }
            })
            .unwrap_or_else(|| "session".into());
        return vec![rpc_from_pi_response(&v, &session_id)];
    }
    if v.get("type").and_then(|t| t.as_str()) == Some("extension_ui_request") {
        if let Some(reply) = auto_ui_response(&v) {
            let sidecar = state.sidecar.clone();
            let _ = sidecar
                .lock()
                .ok()
                .and_then(|g| g.send_line(&reply.to_string()).ok());
        }
        return Vec::new();
    }
    let mapped = state
        .map
        .lock()
        .ok()
        .map(|mut m| m.ingest(&v))
        .unwrap_or_default();
    mapped.into_iter().map(|n| n.to_string()).collect()
}

fn remember_state(state: &BridgeState, response: &Value) {
    if response.get("command").and_then(|c| c.as_str()) != Some("get_state") {
        if response.get("command").and_then(|c| c.as_str()) == Some("new_session") {
            if let Ok(mut map) = state.map.lock() {
                map.reset();
            }
        }
        return;
    }
    let Some(data) = response.get("data") else {
        return;
    };
    if let Ok(mut hint) = state.session.lock() {
        if let Some(id) = data.get("sessionId").and_then(|v| v.as_str()) {
            hint.id = id.to_string();
        }
        hint.file = data
            .get("sessionFile")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        hint.name = data
            .get("sessionName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        hint.model = data
            .get("model")
            .and_then(|m| {
                m.get("id")
                    .or_else(|| m.get("name"))
                    .and_then(|v| v.as_str())
            })
            .map(|s| s.to_string());
    }
}

fn rpc_from_pi_response(v: &Value, session_id: &str) -> String {
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
    let command = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
    if !success {
        let msg = match v.get("error") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => "pi command failed".into(),
        };
        return json!({"jsonrpc":"2.0","id":id,"error":{"message":msg}}).to_string();
    }
    let data = v.get("data").cloned().unwrap_or(json!({}));
    let result = enrich_result(command, session_id, &data);
    json!({"jsonrpc":"2.0","id":id,"result":result}).to_string()
}

fn auto_ui_response(req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone();
    match req.get("method").and_then(|m| m.as_str()).unwrap_or("") {
        "confirm" => Some(json!({
            "type": "extension_ui_response",
            "id": id,
            "confirmed": true
        })),
        "select" => {
            let first = req
                .get("options")
                .and_then(|o| o.as_array())
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(Value::Null);
            Some(json!({
                "type": "extension_ui_response",
                "id": id,
                "value": first
            }))
        }
        "input" | "editor" => Some(json!({
            "type": "extension_ui_response",
            "id": id,
            "cancelled": true
        })),
        _ => None,
    }
}

fn to_pi_line(method: &str, id: &Value, params: &Value) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("type".into(), json!(method));
    if !id.is_null() {
        obj.insert("id".into(), id.clone());
    }
    if let Some(map) = params.as_object() {
        for (k, v) in map {
            if k == "type" || k == "id" {
                continue;
            }
            obj.insert(k.clone(), v.clone());
        }
    }
    if method == "prompt" && !obj.contains_key("message") {
        if let Some(text) = params.get("text").cloned() {
            obj.insert("message".into(), text);
        }
    }
    Value::Object(obj).to_string()
}

fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/health", get(health))
        .route("/model", get(get_model).put(put_model))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/workspace-pick", post(pick_workspace))
        .route("/ws", get(ws_upgrade))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn app_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

async fn health() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true,"service":"theseus-desktop","bridge":"pi-rpc"}"#,
    )
}

fn json_headers() -> [(header::HeaderName, &'static str); 1] {
    [(header::CONTENT_TYPE, "application/json")]
}

fn model_json(model: &str) -> ([(header::HeaderName, &'static str); 1], String) {
    (
        json_headers(),
        serde_json::json!({ "model": model }).to_string(),
    )
}

fn settings_json(state: Option<&BridgeState>) -> String {
    let model = state
        .and_then(|s| s.session.lock().ok())
        .and_then(|h| h.model.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::resolve_user_model);
    serde_json::json!({
        "baseUrl": "",
        "model": model,
        "workspace": crate::resolve_user_workspace(),
        "provider": "pi",
        "keyConfigured": false,
    })
    .to_string()
}

async fn get_model(State(state): State<Arc<BridgeState>>) -> impl IntoResponse {
    let model = state
        .session
        .lock()
        .ok()
        .and_then(|h| h.model.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::resolve_user_model);
    model_json(&model)
}

async fn put_model() -> impl IntoResponse {
    (
        StatusCode::OK,
        model_json(&crate::resolve_user_model()),
    )
}

async fn get_settings(State(state): State<Arc<BridgeState>>) -> impl IntoResponse {
    (json_headers(), settings_json(Some(&state)))
}

async fn put_settings(
    State(state): State<Arc<BridgeState>>,
    body: String,
) -> impl IntoResponse {
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                json_headers(),
                settings_json(Some(&state)),
            )
                .into_response();
        }
    };

    let mut restart = false;
    if let Some(raw) = parsed.get("workspace").and_then(|v| v.as_str()) {
        if !raw.trim().is_empty() {
            if crate::persist_user_workspace(raw).is_err() {
                return (
                    StatusCode::BAD_REQUEST,
                    json_headers(),
                    settings_json(Some(&state)),
                )
                    .into_response();
            }
            restart = true;
        }
    }

    if restart {
        let _ = restart_sidecar(&state);
    }

    (StatusCode::OK, json_headers(), settings_json(Some(&state))).into_response()
}

async fn pick_workspace(State(state): State<Arc<BridgeState>>) -> impl IntoResponse {
    match tokio::task::spawn_blocking(crate::pick_workspace_directory).await {
        Ok(Some(path)) => match crate::persist_user_workspace(&path) {
            Ok(workspace) => {
                let _ = restart_sidecar(&state);
                (
                    StatusCode::OK,
                    json_headers(),
                    serde_json::json!({
                        "workspace": workspace,
                        "baseUrl": "",
                        "model": crate::resolve_user_model(),
                        "provider": "pi",
                        "keyConfigured": false,
                    })
                    .to_string(),
                )
                    .into_response()
            }
            Err(_) => (
                StatusCode::BAD_REQUEST,
                json_headers(),
                settings_json(Some(&state)),
            )
                .into_response(),
        },
        _ => (StatusCode::OK, json_headers(), settings_json(Some(&state))).into_response(),
    }
}

fn restart_sidecar(state: &BridgeState) -> Result<(), String> {
    let (next, rx) = Sidecar::start().map_err(|e| e.to_string())?;
    if let Ok(mut map) = state.map.lock() {
        map.reset();
    }
    pump_stdout(
        Arc::new(BridgeState {
            sidecar: state.sidecar.clone(),
            out: state.out.clone(),
            map: state.map.clone(),
            session: state.session.clone(),
        }),
        rx,
    );
    let previous = {
        let mut guard = state
            .sidecar
            .lock()
            .map_err(|_| "sidecar lock".to_string())?;
        std::mem::replace(&mut *guard, next)
    };
    previous.shutdown();
    Ok(())
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<BridgeState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, state))
}

fn handle_client_rpc(state: &BridgeState, line: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let method = v
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let params = v.get("params").cloned().unwrap_or(json!({}));

    if method == "list_sessions" {
        let file = state
            .session
            .lock()
            .ok()
            .and_then(|s| s.file.clone());
        let result = list_sessions(file.as_deref());
        let reply = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
        let _ = state.out.send(reply);
        return Ok(());
    }

    if is_theseus_app_server_method(&method) {
        let reply = json!({
            "jsonrpc":"2.0",
            "id": id,
            "error": {
                "message": format!(
                    "product path is pi --mode rpc only; refused `{method}`"
                )
            }
        })
        .to_string();
        let _ = state.out.send(reply);
        return Ok(());
    }

    if !is_pi_command(&method) {
        let reply = json!({
            "jsonrpc":"2.0",
            "id": id,
            "error": { "message": format!("unknown command `{method}`") }
        })
        .to_string();
        let _ = state.out.send(reply);
        return Ok(());
    }

    if method == "abort" {
        if let Ok(mut map) = state.map.lock() {
            map.mark_abort();
        }
    }
    if method == "new_session" {
        if let Ok(mut map) = state.map.lock() {
            map.reset();
        }
    }

    let pi_line = to_pi_line(&method, &id, &params);
    let sidecar = state.sidecar.lock().map_err(|_| "sidecar lock".to_string())?;
    sidecar.send_line(&pi_line).map_err(|e| e.to_string())
}

async fn ws_session(mut socket: WebSocket, state: Arc<BridgeState>) {
    let mut rx = state.out.subscribe();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(line))) => {
                        let sidecar_state = state.clone();
                        let line = line.to_string();
                        if tokio::task::spawn_blocking(move || {
                            handle_client_rpc(&sidecar_state, &line)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .is_none()
                        {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Ok(line) => {
                        if socket.send(Message::Text(line)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}
