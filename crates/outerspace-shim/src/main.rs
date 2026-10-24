//! `outerspace-mcp.exe`: what MCP clients (Claude Code, Codex, OpenCode) launch. It bridges the stdio
//! transport they speak to OuterSpace's Streamable HTTP endpoint on 127.0.0.1 (docs/PLANO.md §2.5),
//! so their config never holds the token, and starts OuterSpace in the tray when it is not running.
//!
//! stdin/stdout carry newline-delimited JSON-RPC messages; diagnostics go to stderr only.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use outerspace_core::{MCP_PORT, config};
use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::Response;

/// Cold start of the app (driver checks, WebView) can take a while.
const START_TIMEOUT: Duration = Duration::from_secs(20);
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

fn main() {
    let bridge = Arc::new(Bridge::new());
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("outerspace-mcp: ignoring a line that is not JSON ({err})");
                continue;
            }
        };
        if message["method"] == "initialize" {
            // Everything after depends on the session this creates, so nothing runs alongside it.
            bridge.initialize(message);
        } else {
            // Requests may overlap (a long `launch` while the client sends a cancellation).
            let bridge = bridge.clone();
            workers.push(std::thread::spawn(move || bridge.forward(message, true)));
            workers.retain(|worker| !worker.is_finished());
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    bridge.close();
}

struct Bridge {
    agent: Agent,
    url: String,
    token: Mutex<String>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    stdout: Mutex<std::io::Stdout>,
}

impl Bridge {
    fn new() -> Bridge {
        // Tool calls like `launch` legitimately take seconds: no overall timeout.
        let agent = Agent::config_builder().timeout_global(None).http_status_as_error(false).build().new_agent();
        Bridge {
            agent,
            url: format!("http://127.0.0.1:{MCP_PORT}/mcp"),
            token: Mutex::new(config::load().map(|c| c.token).unwrap_or_default()),
            session: Mutex::new(None),
            protocol: Mutex::new(None),
            stdout: Mutex::new(std::io::stdout()),
        }
    }

    fn initialize(&self, message: Value) {
        if let Err(reason) = self.ensure_running() {
            return self.reply_error(&message, &reason);
        }
        self.forward(message, true);
    }

    /// Starts OuterSpace (hidden, in the tray) if its endpoint does not answer, and waits for it.
    fn ensure_running(&self) -> Result<(), String> {
        if !self.healthy() {
            let app = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("OuterSpace.exe")))
                .filter(|app| app.is_file())
                .ok_or("OuterSpace is not running and OuterSpace.exe was not found next to outerspace-mcp.exe")?;
            eprintln!("outerspace-mcp: starting {}", app.display());
            // Leave the client's job object if allowed, so closing the client does not close OuterSpace.
            let spawn = |flags: u32| {
                Command::new(&app)
                    .arg("--hidden")
                    .creation_flags(flags)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            };
            spawn(DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB)
                .or_else(|_| spawn(DETACHED_PROCESS))
                .map_err(|e| format!("could not start OuterSpace: {e}"))?;
            let deadline = Instant::now() + START_TIMEOUT;
            while !self.healthy() {
                if Instant::now() >= deadline {
                    return Err("OuterSpace did not start in time".into());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        // The app writes the token on its first run, so read it again now that it is up.
        if let Some(config) = config::load() {
            *lock(&self.token) = config.token;
        }
        Ok(())
    }

    fn healthy(&self) -> bool {
        let health = self.url.replace("/mcp", "/health");
        self.agent.get(&health).call().is_ok_and(|response| response.status().is_success())
    }

    fn forward(&self, message: Value, retry_auth: bool) {
        let mut request = self
            .agent
            .post(&self.url)
            .header("authorization", format!("Bearer {}", lock(&self.token)))
            .header("accept", "application/json, text/event-stream")
            .content_type("application/json");
        if let Some(session) = lock(&self.session).clone() {
            request = request.header("mcp-session-id", session);
        }
        if let Some(protocol) = lock(&self.protocol).clone() {
            request = request.header("mcp-protocol-version", protocol);
        }
        let response = match request.send(message.to_string().as_str()) {
            Ok(response) => response,
            Err(err) => {
                return self.reply_error(&message, &format!("OuterSpace is not reachable ({err}); is it running?"));
            }
        };

        let status = response.status().as_u16();
        if let Some(session) = header(&response, "mcp-session-id") {
            *lock(&self.session) = Some(session);
        }
        match status {
            202 => {} // a notification, accepted
            401 if retry_auth => {
                // The token changed (config recreated): pick up the new one and try once more.
                if let Some(config) = config::load() {
                    *lock(&self.token) = config.token;
                }
                self.forward(message, false);
            }
            200..=299 => {
                let streaming = header(&response, "content-type").is_some_and(|t| t.starts_with("text/event-stream"));
                let mut body = BufReader::new(response.into_body().into_reader());
                if streaming {
                    self.relay_events(&mut body);
                } else {
                    let mut text = String::new();
                    if body.read_to_string(&mut text).is_ok() && !text.trim().is_empty() {
                        self.emit(text.trim());
                    }
                }
            }
            _ => {
                let mut text = String::new();
                let _ = response.into_body().into_reader().read_to_string(&mut text);
                self.reply_error(&message, &format!("OuterSpace answered HTTP {status}: {}", text.trim()));
            }
        }
    }

    /// Server-sent events: every event's `data` is one JSON-RPC message for the client.
    fn relay_events(&self, body: &mut impl BufRead) {
        let mut data = String::new();
        for line in body.lines() {
            let Ok(line) = line else { break };
            if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            } else if line.is_empty() && !data.is_empty() {
                self.emit(&std::mem::take(&mut data));
            }
        }
        if !data.is_empty() {
            self.emit(&data);
        }
    }

    fn emit(&self, message: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(message)
            && let Some(protocol) = value["result"]["protocolVersion"].as_str()
        {
            *lock(&self.protocol) = Some(protocol.to_owned());
        }
        let mut stdout = lock(&self.stdout);
        let _ = writeln!(stdout, "{message}");
        let _ = stdout.flush();
    }

    /// Requests get a JSON-RPC error so the client shows the reason; notifications have nobody waiting.
    fn reply_error(&self, message: &Value, reason: &str) {
        eprintln!("outerspace-mcp: {reason}");
        if let Some(id) = message.get("id") {
            self.emit(
                &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": reason } }).to_string(),
            );
        }
    }

    /// Ends the HTTP session when the client goes away.
    fn close(&self) {
        if let Some(session) = lock(&self.session).clone() {
            let _ = self
                .agent
                .delete(&self.url)
                .header("authorization", format!("Bearer {}", lock(&self.token)))
                .header("mcp-session-id", session)
                .call();
        }
    }
}

fn header(response: &Response<ureq::Body>, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
