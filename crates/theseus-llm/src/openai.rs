use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use native_tls::TlsConnector;
use serde::Deserialize;
use serde_json::json;

use crate::error::{
    default_base_url, default_model, env_api_key, redact, truncate, LlmError, DEFAULT_BASE_URL,
};
use crate::seam::{ChatOutcome, ChatRequest, LlmSeam, ToolCallRequest};

const CHAT_PATH: &str = "/v1/chat/completions";
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// OpenAI-compatible HTTP seam. The API key lives only in process memory.
pub struct OpenAiChatSeam {
    base_url: String,
    api_key: Option<String>,
    default_model: String,
}

impl std::fmt::Debug for OpenAiChatSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiChatSeam")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl OpenAiChatSeam {
    pub fn from_env() -> Self {
        let api_key = env_api_key().map(|s| s.trim().to_string());
        Self::new(default_base_url(), api_key, default_model())
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            default_model: default_model.into(),
        }
    }

    pub fn default_base_url_const() -> &'static str {
        DEFAULT_BASE_URL
    }

    pub fn chat_url(&self) -> String {
        chat_completions_url(&self.base_url)
    }

    fn key(&self) -> Result<&str, LlmError> {
        self.api_key
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(LlmError::MissingApiKey)
    }
}

impl LlmSeam for OpenAiChatSeam {
    fn ready(&self) -> Result<(), LlmError> {
        self.key().map(|_| ())
    }

    fn stream_chat(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, LlmError> {
        let key = self.key()?;
        let model = if request.model.is_empty() {
            self.default_model.as_str()
        } else {
            request.model.as_str()
        };
        let mut payload = json!({
            "model": model,
            "stream": true,
            "messages": request.messages,
        });
        if !request.tools.is_empty() {
            payload["tools"] = serde_json::to_value(&request.tools)
                .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
        }
        let body =
            serde_json::to_vec(&payload).map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
        let url = self.chat_url();
        post_sse(&url, key, &body, on_delta)
    }
}

struct Endpoint {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_endpoint(url: &str) -> Result<Endpoint, LlmError> {
    let url = url.trim();
    let (tls, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(LlmError::InvalidResponse(
            "llm base url must be http:// or https://".into(),
        ));
    };
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = if let Some((h, p)) = hostport.rsplit_once(':') {
        if hostport.starts_with('[') {
            return Err(LlmError::InvalidResponse(
                "ipv6 urls are out of scope".into(),
            ));
        }
        let port = p
            .parse::<u16>()
            .map_err(|_| LlmError::InvalidResponse("invalid url port".into()))?;
        (h.to_string(), port)
    } else {
        (hostport.to_string(), if tls { 443 } else { 80 })
    };
    if host.is_empty() {
        return Err(LlmError::InvalidResponse("llm url missing host".into()));
    }
    Ok(Endpoint {
        tls,
        host,
        port,
        path,
    })
}

fn post_sse(
    url: &str,
    key: &str,
    body: &[u8],
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatOutcome, LlmError> {
    let endpoint = parse_endpoint(url)?;
    let tcp = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| LlmError::Transport(e.to_string()))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| LlmError::Transport(e.to_string()))?;

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.host,
        key,
        body.len()
    );

    if endpoint.tls {
        let connector = TlsConnector::new().map_err(|e| LlmError::Transport(e.to_string()))?;
        let mut stream = connector
            .connect(&endpoint.host, tcp)
            .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(body))
            .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
        read_http_sse(stream, key, on_delta)
    } else {
        let mut stream = tcp;
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(body))
            .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
        read_http_sse(stream, key, on_delta)
    }
}

fn read_http_sse<S: Read>(
    stream: S,
    key: &str,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatOutcome, LlmError> {
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
    let status = parse_http_status(&status_line)?;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| LlmError::Transport(redact(&e.to_string(), key)))?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if line.to_ascii_lowercase().contains("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    if status != 200 {
        let mut rest = String::new();
        if chunked {
            let _ = BufReader::new(ChunkedReader::new(reader)).read_to_string(&mut rest);
        } else {
            let _ = reader.read_to_string(&mut rest);
        }
        return Err(LlmError::Http {
            status,
            message: redact(&truncate(&rest, 200), key),
        });
    }
    if chunked {
        assemble_sse(BufReader::new(ChunkedReader::new(reader)), on_delta, key)
    } else {
        assemble_sse(reader, on_delta, key)
    }
}

/// Decode HTTP/1.1 chunked framing so SSE lines are not mixed with hex sizes.
struct ChunkedReader<R: BufRead> {
    inner: R,
    remaining: usize,
    done: bool,
}

impl<R: BufRead> ChunkedReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for ChunkedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut size_line = String::new();
            self.inner.read_line(&mut size_line)?;
            if size_line.is_empty() {
                self.done = true;
                return Ok(0);
            }
            let size_hex = size_line.trim().split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_hex, 16)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            if size == 0 {
                self.done = true;
                let mut trailer = String::new();
                let _ = self.inner.read_line(&mut trailer);
                return Ok(0);
            }
            self.remaining = size;
        }
        let take = buf.len().min(self.remaining);
        let n = self.inner.read(&mut buf[..take])?;
        self.remaining -= n;
        if self.remaining == 0 {
            let mut crlf = [0u8; 2];
            let _ = self.inner.read_exact(&mut crlf);
        }
        Ok(n)
    }
}

fn parse_http_status(line: &str) -> Result<u16, LlmError> {
    let code = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| LlmError::InvalidResponse("missing http status".into()))?;
    code.parse::<u16>()
        .map_err(|_| LlmError::InvalidResponse("invalid http status".into()))
}

pub(crate) fn chat_completions_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    format!("{base}{CHAT_PATH}")
}

pub(crate) fn assemble_sse<R: BufRead>(
    reader: R,
    on_delta: &mut dyn FnMut(&str),
    redact_secret: &str,
) -> Result<ChatOutcome, LlmError> {
    let mut assembled = String::new();
    let mut acc: Vec<AccCall> = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| LlmError::Transport(redact(&e.to_string(), redact_secret)))?;
        let Some(data) = sse_data(&line) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: StreamChunk = serde_json::from_str(data).map_err(|e| {
            LlmError::InvalidResponse(redact(&truncate(&e.to_string(), 120), redact_secret))
        })?;
        if let Some(text) = chunk.first_content() {
            if !text.is_empty() {
                on_delta(text);
                assembled.push_str(text);
            }
        }
        for delta in chunk.tool_call_deltas() {
            if acc.len() <= delta.index {
                acc.resize_with(delta.index + 1, AccCall::default);
            }
            let slot = &mut acc[delta.index];
            if let Some(id) = delta.id {
                if !id.is_empty() {
                    slot.id = id;
                }
            }
            if let Some(function) = delta.function {
                if let Some(name) = function.name {
                    if !name.is_empty() {
                        if slot.name.is_empty() {
                            slot.name = name;
                        } else {
                            slot.name.push_str(&name);
                        }
                    }
                }
                if let Some(arguments) = function.arguments {
                    slot.arguments.push_str(&arguments);
                }
            }
        }
    }
    let tool_calls = acc
        .into_iter()
        .enumerate()
        .filter(|(_, c)| !c.name.is_empty())
        .map(|(i, c)| ToolCallRequest {
            id: if c.id.is_empty() {
                format!("call_{i}")
            } else {
                c.id
            },
            name: c.name,
            arguments: c.arguments,
        })
        .collect();
    Ok(ChatOutcome {
        text: assembled,
        tool_calls,
    })
}

#[derive(Default)]
struct AccCall {
    id: String,
    name: String,
    arguments: String,
}

fn sse_data(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix("data:")?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

impl StreamChunk {
    fn first_content(&self) -> Option<&str> {
        self.choices.first()?.delta.content.as_deref()
    }

    fn tool_call_deltas(&self) -> Vec<ToolCallDelta> {
        self.choices
            .first()
            .map(|c| c.delta.tool_calls.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Clone, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::messages_from_derived;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use theseus_protocol::DerivedMessage;

    fn request() -> ChatRequest {
        ChatRequest::new(
            "gpt-4o-mini",
            messages_from_derived(&[DerivedMessage::User {
                content: "hi".into(),
            }]),
        )
    }

    fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header_len = pos + 4;
                        let headers = String::from_utf8_lossy(&raw[..header_len]);
                        let cl = headers
                            .lines()
                            .find_map(|l| {
                                let lower = l.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        while raw.len() < header_len + cl {
                            match stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => raw.extend_from_slice(&buf[..n]),
                                Err(_) => break,
                            }
                        }
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        raw
    }

    fn spawn_http_once(
        status_line: &'static str,
        content_type: &'static str,
        body: String,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let raw = read_http_request(&mut stream);
            let _ = tx.send(String::from_utf8_lossy(&raw).into_owned());
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        });
        (format!("http://{addr}"), rx, handle)
    }

    #[test]
    fn chat_url_joins_v1() {
        assert_eq!(
            chat_completions_url("https://ai.aruyx.com/"),
            "https://ai.aruyx.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://ai.aruyx.com"),
            "https://ai.aruyx.com/v1/chat/completions"
        );
    }

    #[test]
    fn sse_assembles_deltas() {
        let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}

data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}

data: [DONE]
";
        let mut seen = Vec::new();
        let out = assemble_sse(
            std::io::Cursor::new(body),
            &mut |d| seen.push(d.to_string()),
            "",
        )
        .unwrap();
        assert_eq!(out.text, "Hello");
        assert!(out.tool_calls.is_empty());
        assert_eq!(seen, vec!["Hel", "lo"]);
    }

    #[test]
    fn sse_assembles_incremental_tool_calls() {
        let body = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"read\",\"arguments\":\"\"}}]}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]}}]}

data: [DONE]
";
        let out = assemble_sse(std::io::Cursor::new(body), &mut |_| {}, "").unwrap();
        assert!(out.text.is_empty());
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].id, "c1");
        assert_eq!(out.tool_calls[0].name, "read");
        assert_eq!(out.tool_calls[0].arguments, r#"{"path":"a.txt"}"#);
    }

    #[test]
    fn localhost_stub_streams_without_real_network() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
        let (base, rx, handle) = spawn_http_once("200 OK", "text/event-stream", sse.to_string());
        let seam = OpenAiChatSeam::new(base, Some("dummy-test-token".into()), "gpt-4o-mini");
        let mut seen = Vec::new();
        let out = seam
            .stream_chat(&request(), &mut |d| seen.push(d.to_string()))
            .unwrap();
        assert_eq!(out.text, "ok");
        assert!(out.tool_calls.is_empty());
        assert_eq!(seen, vec!["ok"]);
        let incoming = rx.recv().unwrap();
        assert!(
            incoming.contains("Authorization: Bearer dummy-test-token"),
            "seam must send the bearer header"
        );
        assert!(incoming.contains("/v1/chat/completions"));
        handle.join().unwrap();
    }

    #[test]
    fn localhost_http_error_is_redacted() {
        let (base, _rx, handle) = spawn_http_once(
            "401 Unauthorized",
            "text/plain",
            "unauthorized dummy-test-token leaked?".into(),
        );
        let seam = OpenAiChatSeam::new(base, Some("dummy-test-token".into()), "gpt-4o-mini");
        let err = seam.stream_chat(&request(), &mut |_| {}).unwrap_err();
        let shown = err.to_string();
        assert!(!shown.contains("dummy-test-token"), "{shown}");
        assert!(matches!(err, LlmError::Http { status: 401, .. }));
        let _ = handle.join();
    }

    #[test]
    fn missing_key_does_not_call_http() {
        let seam = OpenAiChatSeam::new("http://127.0.0.1:1", None, "gpt-4o-mini");
        assert_eq!(seam.ready(), Err(LlmError::MissingApiKey));
        assert_eq!(
            seam.stream_chat(&request(), &mut |_| {}).unwrap_err(),
            LlmError::MissingApiKey
        );
    }

    #[test]
    #[ignore = "hits the live THESEUS_LLM_BASE_URL; export THESEUS_LLM_API_KEY to run"]
    fn live_chat_completions() {
        let seam = OpenAiChatSeam::from_env();
        seam.ready().expect("THESEUS_LLM_API_KEY");
        let out = seam
            .stream_chat(&request(), &mut |_| {})
            .expect("live stream");
        assert!(!out.text.is_empty(), "model returned empty text");
    }

    #[test]
    fn localhost_stub_sends_tools_when_present() {
        use crate::messages::OpenAiTool;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
        let (base, rx, handle) = spawn_http_once("200 OK", "text/event-stream", sse.to_string());
        let seam = OpenAiChatSeam::new(base, Some("dummy-test-token".into()), "gpt-4o-mini");
        let mut req = request();
        req.tools = vec![OpenAiTool::function(
            "read",
            "Read a file",
            serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
        )];
        let _ = seam.stream_chat(&req, &mut |_| {}).unwrap();
        let incoming = rx.recv().unwrap();
        assert!(incoming.contains("\"tools\""), "{incoming}");
        assert!(incoming.contains("\"read\""), "{incoming}");
        handle.join().unwrap();
    }
}
