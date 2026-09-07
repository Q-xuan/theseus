//! Loopback HTTP + WebSocket → sidecar **stdio**.
//!
//! Transitional glue so the thin client can keep speaking JSON-RPC over WS
//! (same shape as `theseus-web`). Not a remote-control surface: bind 127.0.0.1 only.
//! The shell still does not run the agent loop.

use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;

use crate::sidecar::Sidecar;

const INDEX: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");

#[derive(Clone)]
struct BridgeState {
    sidecar: Sidecar,
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
    let fanout = out.clone();
    tokio::task::spawn_blocking(move || {
        while let Ok(line) = rx.recv() {
            let _ = fanout.send(line);
        }
    });
    let state = Arc::new(BridgeState { sidecar, out });
    axum::serve(listener, router(state)).await
}

fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/health", get(health))
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
                        if tokio::task::spawn_blocking(move || sidecar.send_line(&line))
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
