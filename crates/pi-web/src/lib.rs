//! Thin Web **client** over the existing app-server JSON-RPC.
//!
//! π核：浏览器只发 `thread/start` / `thread/resume` / `thread/list` /
//! `turn/start` / `tool/approve` / `tool/reject`，只渲染 Thread / Turn /
//! Item + delta + 一张工具确认卡；不实现 loop/log；密钥不进前端；
//! 无 MCP / 会话墙 / 插件 / compaction / 策略设置页。
//!
//! `static/` 是协议连通皮，不是桌面产品终态。Codex/dsh 密度的壳在 `pi-desktop`。
//!
//! 连接取更薄的一侧：同进程转发 [`pi_app_server::AppServer::handle_line_sink`]
//!（与 stdio 同一套 RPC），经 WebSocket 推给浏览器。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use pi_app_server::AppServer;
use serde_json::Value;
use tokio::sync::mpsc;

/// Process-wide app-server. `initialize` is done once at construction.
pub struct Hub {
    server: Mutex<AppServer>,
}

impl Hub {
    pub fn new() -> Self {
        Self::from_server(AppServer::new())
    }

    /// Same JSON-RPC initialize as [`Hub::new`], with a caller-supplied server
    /// (used in tests to pin `PI_SESSIONS_DIR`).
    pub fn from_server(mut server: AppServer) -> Self {
        let _ = server.handle_line(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"clientInfo":{"name":"pi-web","version":"0.1.0"}}}"#,
        );
        Self {
            server: Mutex::new(server),
        }
    }

    /// Run one JSON-RPC line. `emit` is called for each outgoing message in
    /// order, including mid-turn stream notifications.
    pub fn handle_line_stream(&self, line: &str, mut emit: impl FnMut(Value)) {
        let mut server = self.server.lock().expect("app-server lock");
        server.handle_line_sink(line, &mut |out| {
            emit(out.to_json());
        });
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .with_state(hub)
}

pub async fn serve(addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let hub = Arc::new(Hub::new());
    axum::serve(listener, router(hub)).await
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/app.js"),
    )
}

async fn app_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/app.css"),
    )
}

async fn health() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true,"service":"pi-web"}"#,
    )
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, hub))
}

async fn ws_session(mut socket: WebSocket, hub: Arc<Hub>) {
    while let Some(Ok(msg)) = socket.recv().await {
        let Message::Text(line) = msg else {
            continue;
        };
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let hub = Arc::clone(&hub);
        let task = tokio::task::spawn_blocking(move || {
            hub.handle_line_stream(&line, |value| {
                let _ = tx.send(value);
            });
        });
        while let Some(value) = rx.recv().await {
            let payload = value.to_string();
            if socket.send(Message::Text(payload)).await.is_err() {
                break;
            }
        }
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_thread_start_emits_thread_without_key() {
        let dir = std::env::temp_dir().join(format!(
            "pi-web-sess-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = Hub::from_server(AppServer::new().with_sessions_dir(dir.clone()));
        let mut out = Vec::new();
        hub.handle_line_stream(
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{}}"#,
            |v| out.push(v),
        );
        let thread = out.iter().find_map(|v| {
            v.get("result")
                .and_then(|r| r.get("thread"))
                .and_then(|t| t.get("id"))
                .and_then(|id| id.as_str())
                .map(str::to_string)
        });
        let thread_id = thread.unwrap();
        assert!(thread_id.starts_with("thr_"));
        let path = out.iter().find_map(|v| {
            v.get("result")
                .and_then(|r| r.get("thread"))
                .and_then(|t| t.get("path"))
                .and_then(|p| p.as_str())
                .map(str::to_string)
        });
        let path = path.expect("Thread.path");
        assert!(path.ends_with(&format!("{thread_id}.jsonl")), "{path}");
        assert!(std::path::Path::new(&path).is_file());
        assert!(
            !out.iter().any(|v| v.to_string().contains("sk-")),
            "bridge must not echo secrets"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hub_list_and_resume_after_new_hub() {
        use pi_llm::ScriptedSeam;

        let dir = std::env::temp_dir().join(format!(
            "pi-web-resume-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = Hub::from_server(
            AppServer::with_llm(ScriptedSeam::ok(["hello"])).with_sessions_dir(dir.clone()),
        );
        let mut started = Vec::new();
        hub.handle_line_stream(
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{}}"#,
            |v| started.push(v),
        );
        let thread_id = started
            .iter()
            .find_map(|v| v["result"]["thread"]["id"].as_str().map(str::to_string))
            .unwrap();
        hub.handle_line_stream(
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{{"threadId":"{thread_id}","input":[{{"type":"text","text":"hi"}}]}}}}"#
            ),
            |_| {},
        );

        let hub2 = Hub::from_server(
            AppServer::with_llm(ScriptedSeam::ok(["again"])).with_sessions_dir(dir.clone()),
        );
        let mut listed = Vec::new();
        hub2.handle_line_stream(
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/list","params":{}}"#,
            |v| listed.push(v),
        );
        let ids: Vec<String> = listed
            .iter()
            .find_map(|v| {
                v.get("result")
                    .and_then(|r| r.get("threads"))
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t["id"].as_str().map(str::to_string))
                            .collect()
                    })
            })
            .unwrap();
        assert_eq!(ids, vec![thread_id.clone()]);

        let mut resumed = Vec::new();
        hub2.handle_line_stream(
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"thread/resume","params":{{"threadId":"{thread_id}"}}}}"#
            ),
            |v| resumed.push(v),
        );
        let preview = resumed.iter().find_map(|v| {
            v["result"]["thread"]["preview"]
                .as_str()
                .map(str::to_string)
        });
        assert_eq!(preview.as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn static_shell_stays_a_client() {
        let html = include_str!("../static/index.html");
        let js = include_str!("../static/app.js");
        for hay in [html, js] {
            assert!(!hay.contains("PI_LLM_API_KEY"));
            assert!(!hay.contains("apiKey"));
            assert!(!hay.contains("api_key"));
            assert!(!hay.contains("password"));
            assert!(!hay.contains("derive_messages"));
            assert!(!hay.contains("MCP"));
            assert!(!hay.contains("compaction"));
            assert!(!hay.contains("PI_SESSIONS_DIR"));
            assert!(!hay.contains("PI_HOME"));
            assert!(!hay.contains(".pi-app"));
        }
        assert!(html.contains("pi-web"));
        assert!(js.contains("item/agentMessage/delta"));
        assert!(js.contains("thread/start"));
        assert!(js.contains("thread/resume"));
        assert!(js.contains("thread/list"));
        assert!(js.contains("turn/start"));
        assert!(js.contains("tool/approve"));
        assert!(js.contains("tool/reject"));
        assert!(js.contains("item/tool/approval/request"));
        assert!(html.contains("id=\"approval\""));
        assert!(!html.contains("type=\"search\""));
        assert!(!html.to_lowercase().contains("settings"));
        assert!(!js.contains("PI_TOOL_APPROVAL"));
        assert!(!js.contains("pinned"));
        assert!(
            !js.contains("session/event"),
            "UI must project Item/delta, not raw session log"
        );
    }

    #[tokio::test]
    async fn health_and_index_smoke() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router(Arc::new(Hub::new())))
                .await
                .unwrap();
        });

        let body = tokio::task::spawn_blocking(move || {
            let mut last = String::new();
            for _ in 0..40 {
                if let Ok(mut stream) = TcpStream::connect(addr) {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let req = format!(
                        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                    );
                    if stream.write_all(req.as_bytes()).is_ok() {
                        last.clear();
                        let _ = stream.read_to_string(&mut last);
                        if last.contains("pi-web") {
                            return last;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            last
        })
        .await
        .unwrap();
        assert!(body.contains(r#""ok":true"#), "{body}");
        assert!(!body.contains("sk-"));
    }
}
