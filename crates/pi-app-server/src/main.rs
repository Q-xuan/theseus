use std::env;
use std::io::{self, BufRead, Write};

use pi_app_server::AppServer;
use pi_core::{default_sessions_dir, ENV_HOME, ENV_SESSIONS_DIR};
use pi_llm::{LlmSeam, OpenAiChatSeam, ENV_API_KEY, ENV_BASE_URL};
use pi_protocol::session_event_json_schema;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--print-schema") | Some("schema") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&session_event_json_schema()).expect("schema")
            );
        }
        Some("--demo") | Some("demo") => run_demo(),
        Some("--help") | Some("-h") => print_help(),
        Some(other) => {
            eprintln!("unknown argument: {other}");
            print_help();
            std::process::exit(2);
        }
        None => run_stdio(),
    }
}

fn print_help() {
    eprintln!(
        "\
pi-app-server — stdio JSON-RPC app-server

Usage:
  pi-app-server              read NDJSON JSON-RPC on stdin, write on stdout
  pi-app-server --demo       in-process thread/start + turn/start transcript
  pi-app-server --print-schema
  pi-app-server --help

Set {ENV_API_KEY} (required for turn/start) and optional {ENV_BASE_URL}.
Do not pass the key on the command line.

Session JSONL: ${ENV_SESSIONS_DIR}/<thread_id>.jsonl, or ${ENV_HOME}/sessions/, or ~/.pi-app/sessions/.
"
    );
}

fn run_stdio() {
    eprintln!(
        "pi-app-server sessions: {}",
        default_sessions_dir().display()
    );
    let mut server = AppServer::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(err) => {
                eprintln!("stdin error: {err}");
                break;
            }
        };
        let result = server.handle_line_sink(&line, &mut |msg| {
            let _ = writeln!(stdout, "{}", msg.to_line());
            let _ = stdout.flush();
        });
        if result.shutdown {
            break;
        }
    }
}

fn run_demo() {
    if OpenAiChatSeam::from_env().ready().is_err() {
        eprintln!("{ENV_API_KEY} is not set; turn/start will return an error and will not write a turn to the log.");
    }
    let mut server = AppServer::new();
    let script = [
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"clientInfo":{"name":"demo","version":"0.1.0"}}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{"threadId":"thr_1","input":[{"type":"text","text":"hello"}]}}"#,
    ];
    for line in script {
        println!(">> {line}");
        let result = server.handle_line_sink(line, &mut |msg| {
            println!("<< {}", msg.to_line());
        });
        let _ = result;
    }
}
