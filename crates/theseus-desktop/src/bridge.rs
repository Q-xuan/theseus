//! Loopback HTTP + WebSocket → sidecar **stdio**.
//!
//! Transitional glue so the thin client can keep speaking JSON-RPC over WS
//! (same shape as `theseus-web`). Not a remote-control surface: bind 127.0.0.1 only.
//! The shell still does not run the agent loop.

use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::broadcast;

use crate::sidecar::Sidecar;

const INDEX: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");

#[derive(Clone)]
struct BridgeState {
    sidecar: Arc<Mutex<Sidecar>>,
    out: broadcast::Sender<String>,
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
    pump_stdout(out.clone(), rx);
    let state = Arc::new(BridgeState {
        sidecar: Arc::new(Mutex::new(sidecar)),
        out,
    });
    axum::serve(listener, router(state)).await
}

fn pump_stdout(fanout: broadcast::Sender<String>, rx: Receiver<String>) {
    tokio::task::spawn_blocking(move || {
        while let Ok(line) = rx.recv() {
            let _ = fanout.send(line);
        }
    });
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
        r#"{"ok":true,"service":"theseus-desktop","bridge":"stdio-sidecar"}"#,
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

fn settings_json() -> String {
    serde_json::json!({
        "baseUrl": crate::resolve_user_base_url(),
        "model": crate::resolve_user_model(),
        "workspace": crate::resolve_user_workspace(),
        "keyConfigured": crate::key_configured(),
    })
    .to_string()
}

async fn get_model() -> impl IntoResponse {
    model_json(&crate::resolve_user_model())
}

async fn put_model(body: String) -> impl IntoResponse {
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                model_json(&crate::resolve_user_model()),
            )
                .into_response();
        }
    };
    let raw = parsed.get("model").and_then(|v| v.as_str()).unwrap_or("");
    match crate::persist_user_model(raw) {
        Ok(model) => (StatusCode::OK, model_json(&model)).into_response(),
        Err(_) => (
            StatusCode::BAD_REQUEST,
            model_json(&crate::resolve_user_model()),
        )
            .into_response(),
    }
}

async fn get_settings() -> impl IntoResponse {
    (json_headers(), settings_json())
}

async fn put_settings(
    State(state): State<Arc<BridgeState>>,
    body: String,
) -> impl IntoResponse {
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response();
        }
    };

    if let Some(raw) = parsed.get("baseUrl").and_then(|v| v.as_str()) {
        if !raw.trim().is_empty() && crate::persist_user_base_url(raw).is_err() {
            return (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response();
        }
    }
    if let Some(raw) = parsed.get("model").and_then(|v| v.as_str()) {
        if !raw.trim().is_empty() && crate::persist_user_model(raw).is_err() {
            return (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response();
        }
    }
    if let Some(raw) = parsed.get("workspace").and_then(|v| v.as_str()) {
        if !raw.trim().is_empty() && crate::persist_user_workspace(raw).is_err() {
            return (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response();
        }
    }

    let mut restart = parsed
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if let Some(raw) = parsed.get("key").and_then(|v| v.as_str()) {
        if !raw.trim().is_empty() {
            if crate::persist_user_key(raw).is_err() {
                return (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response();
            }
            restart = true;
        }
    }

    if restart {
        let _ = restart_sidecar(&state);
    }

    (StatusCode::OK, json_headers(), settings_json()).into_response()
}

async fn pick_workspace() -> impl IntoResponse {
    match tokio::task::spawn_blocking(crate::pick_workspace_directory).await {
        Ok(Some(path)) => match crate::persist_user_workspace(&path) {
            Ok(workspace) => (
                StatusCode::OK,
                json_headers(),
                serde_json::json!({
                    "workspace": workspace,
                    "baseUrl": crate::resolve_user_base_url(),
                    "model": crate::resolve_user_model(),
                    "keyConfigured": crate::key_configured(),
                })
                .to_string(),
            )
                .into_response(),
            Err(_) => (StatusCode::BAD_REQUEST, json_headers(), settings_json()).into_response(),
        },
        _ => (StatusCode::OK, json_headers(), settings_json()).into_response(),
    }
}

fn restart_sidecar(state: &BridgeState) -> Result<(), String> {
    crate::hydrate_process_key();
    let (next, rx) = Sidecar::start().map_err(|e| e.to_string())?;
    pump_stdout(state.out.clone(), rx);
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

async fn ws_session(mut socket: WebSocket, state: Arc<BridgeState>) {
    let mut rx = state.out.subscribe();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(line))) => {
                        let sidecar = state.sidecar.clone();
                        let line = line.to_string();
                        if tokio::task::spawn_blocking(move || {
                            let guard = sidecar.lock().ok()?;
                            guard.send_line(&line).ok()
                        })
                        .await
                        .ok()
                        .flatten()
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
