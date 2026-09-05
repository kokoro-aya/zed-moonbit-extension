use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const MAX_STDERR_BYTES: usize = 32 * 1024;
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suite {
    Capabilities,
    ProjectRoots,
    Freshness,
    All,
}

impl Suite {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "capabilities" => Ok(Self::Capabilities),
            "project-roots" => Ok(Self::ProjectRoots),
            "freshness" => Ok(Self::Freshness),
            "all" => Ok(Self::All),
            _ => Err(format!("unknown suite: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(format!("unknown output format: {value}")),
        }
    }
}

#[derive(Debug)]
pub struct ProbeOptions {
    pub suite: Suite,
    pub moon: PathBuf,
    pub format: OutputFormat,
}

#[derive(Debug)]
enum ReaderEvent {
    Message(Value),
    Closed,
    Error(String),
}

#[derive(Debug, Default)]
struct StderrState {
    bytes: Vec<u8>,
    truncated: bool,
}

pub struct LspSession {
    child: Child,
    input: Option<BufWriter<ChildStdin>>,
    receiver: mpsc::Receiver<ReaderEvent>,
    pending: VecDeque<Value>,
    timeline: Vec<Value>,
    next_id: u64,
    stderr: Arc<Mutex<StderrState>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    moon: PathBuf,
    version: String,
    cwd: PathBuf,
    root_uri: String,
}

impl LspSession {
    pub fn start(moon: &Path, cwd: &Path) -> Result<Self, String> {
        let moon = moon
            .canonicalize()
            .map_err(|error| format!("resolve MoonBit binary {}: {error}", moon.display()))?;
        let cwd = cwd
            .canonicalize()
            .map_err(|error| format!("resolve session cwd {}: {error}", cwd.display()))?;
        let root_uri = url::Url::from_directory_path(&cwd)
            .map_err(|_| format!("convert root path to URI: {}", cwd.display()))?
            .to_string();
        let version = moon_version(&moon)?;

        let mut child = Command::new(&moon)
            .arg("lsp")
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn {} lsp: {error}", moon.display()))?;

        let input = BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| "MoonBit LSP stdin was not piped".to_string())?,
        );
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "MoonBit LSP stdout was not piped".to_string())?;
        let error = child
            .stderr
            .take()
            .ok_or_else(|| "MoonBit LSP stderr was not piped".to_string())?;

        let (sender, receiver) = mpsc::channel();
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                match read_frame(&mut reader) {
                    Ok(Some(message)) => {
                        if sender.send(ReaderEvent::Message(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = sender.send(ReaderEvent::Closed);
                        break;
                    }
                    Err(error) => {
                        let _ = sender.send(ReaderEvent::Error(error));
                        break;
                    }
                }
            }
        });

        let stderr = Arc::new(Mutex::new(StderrState::default()));
        let stderr_capture = Arc::clone(&stderr);
        let stderr_thread = thread::spawn(move || {
            let mut reader = BufReader::new(error);
            let mut chunk = [0_u8; 4096];
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                let mut state = stderr_capture.lock().expect("stderr capture poisoned");
                let remaining = MAX_STDERR_BYTES.saturating_sub(state.bytes.len());
                let retained = remaining.min(count);
                state.bytes.extend_from_slice(&chunk[..retained]);
                state.truncated |= retained < count;
            }
        });

        Ok(Self {
            child,
            input: Some(input),
            receiver,
            pending: VecDeque::new(),
            timeline: vec![json!({
                "event": "spawn",
                "command": moon,
                "args": ["lsp"],
                "cwd": cwd,
                "root_uri": root_uri,
            })],
            next_id: 1,
            stderr,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            moon,
            version,
            cwd,
            root_uri,
        })
    }

    pub fn initialize(&mut self) -> Result<Value, String> {
        let result = self.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": self.root_uri,
                "capabilities": {}
            }),
            RESPONSE_TIMEOUT,
        )?;
        self.notify("initialized", json!({}))?;
        Ok(result)
    }

    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = call_message(Some(id), method, params);
        self.send(request.clone())?;

        if let Some(position) = self
            .pending
            .iter()
            .position(|message| message.get("id").and_then(Value::as_u64) == Some(id))
        {
            return response_result(
                method,
                self.pending
                    .remove(position)
                    .expect("pending response position exists"),
            );
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("timeout waiting for {method} response id {id}"));
            }
            let message = self
                .receive_from_channel(remaining)
                .map_err(|error| format!("{error}; outbound {method} request was {request}"))?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return response_result(method, message);
            }
            self.pending.push_back(message);
        }
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(call_message(None, method, params))
    }

    pub fn finish(mut self) -> Result<Value, String> {
        let shutdown_result = self.request("shutdown", Value::Null, RESPONSE_TIMEOUT);
        let exit_result = self.notify("exit", Value::Null);
        drop(self.input.take());

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    self.child
                        .kill()
                        .map_err(|error| format!("kill unresponsive MoonBit LSP: {error}"))?;
                    let _ = self.child.wait();
                    break;
                }
                Err(error) => return Err(format!("wait for MoonBit LSP: {error}")),
            }
        }

        if let Some(handle) = self.stdout_thread.take() {
            handle
                .join()
                .map_err(|_| "MoonBit LSP stdout reader panicked".to_string())?;
        }
        if let Some(handle) = self.stderr_thread.take() {
            handle
                .join()
                .map_err(|_| "MoonBit LSP stderr reader panicked".to_string())?;
        }

        shutdown_result?;
        exit_result?;
        let stderr = self.stderr_json();
        Ok(json!({
            "cwd": self.cwd,
            "root_uri": self.root_uri,
            "timeline": self.timeline,
            "stderr": stderr,
        }))
    }

    pub fn binary_json(&self) -> Value {
        json!({
            "path": self.moon,
            "version": self.version,
            "args": ["lsp"],
        })
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        let payload = serde_json::to_vec(&message)
            .map_err(|error| format!("serialize JSON-RPC message: {error}"))?;
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| "MoonBit LSP stdin is already closed".to_string())?;
        input
            .write_all(format!("Content-Length: {}\r\n\r\n", payload.len()).as_bytes())
            .and_then(|_| input.write_all(&payload))
            .and_then(|_| input.flush())
            .map_err(|error| format!("write JSON-RPC message: {error}"))?;
        self.timeline.push(message_summary("send", &message));
        Ok(())
    }

    fn receive_from_channel(&mut self, timeout: Duration) -> Result<Value, String> {
        let event = match self.receiver.recv_timeout(timeout) {
            Ok(event) => event,
            Err(error) => {
                let stderr = self.stderr_json();
                return Err(format!(
                    "receive JSON-RPC message: {error}; captured stderr: {}",
                    stderr["text"]
                ));
            }
        };
        match event {
            ReaderEvent::Message(message) => {
                self.timeline.push(message_summary("receive", &message));
                Ok(message)
            }
            ReaderEvent::Closed => Err("MoonBit LSP stdout closed".to_string()),
            ReaderEvent::Error(error) => Err(error),
        }
    }

    fn stderr_json(&self) -> Value {
        let state = self.stderr.lock().expect("stderr capture poisoned");
        json!({
            "text": String::from_utf8_lossy(&state.bytes),
            "truncated": state.truncated,
            "retained_bytes": state.bytes.len(),
            "limit_bytes": MAX_STDERR_BYTES,
        })
    }
}

fn response_result(method: &str, message: Value) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        return Err(format!("{method} returned JSON-RPC error: {error}"));
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

fn call_message(id: Option<u64>, method: &str, params: Value) -> Value {
    let mut message = json!({
        "jsonrpc": "2.0",
        "method": method,
    });
    let fields = message
        .as_object_mut()
        .expect("JSON-RPC call literal is an object");
    if let Some(id) = id {
        fields.insert("id".to_string(), json!(id));
    }
    if !params.is_null() {
        fields.insert("params".to_string(), params);
    }
    message
}

impl Drop for LspSession {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn run_capabilities(options: &ProbeOptions) -> Result<Value, String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects/geometry-library")
        .canonicalize()
        .map_err(|error| format!("resolve capabilities fixture: {error}"))?;
    let mut session = LspSession::start(&options.moon, &fixture)?;
    let binary = session.binary_json();
    let initialize = session.initialize()?;
    let session_evidence = session.finish()?;

    Ok(json!({
        "schema": "moonbit-lab/lsp-probe/v1",
        "suite": "capabilities",
        "binary": binary,
        "session": session_evidence,
        "result": {
            "capabilities": initialize.get("capabilities").cloned().unwrap_or(Value::Null),
            "server_info": initialize.get("serverInfo").cloned().unwrap_or(Value::Null),
        }
    }))
}

fn moon_version(moon: &Path) -> Result<String, String> {
    let output = Command::new(moon)
        .arg("--version")
        .output()
        .map_err(|error| format!("run {} --version: {error}", moon.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version failed with {}: {}",
            moon.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut content_length = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        let count = reader
            .read_line(&mut line)
            .map_err(|error| format!("read JSON-RPC header: {error}"))?;
        if count == 0 {
            return if saw_header {
                Err("MoonBit LSP closed during JSON-RPC headers".to_string())
            } else {
                Ok(None)
            };
        }
        saw_header = true;
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|error| format!("invalid Content-Length: {error}"))?,
                );
            }
        }
    }

    let length = content_length.ok_or_else(|| "JSON-RPC frame lacks Content-Length".to_string())?;
    if length > MAX_MESSAGE_BYTES {
        return Err(format!(
            "JSON-RPC frame is {length} bytes; limit is {MAX_MESSAGE_BYTES}"
        ));
    }
    let mut payload = vec![0_u8; length];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("read JSON-RPC payload: {error}"))?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| format!("parse JSON-RPC payload: {error}"))
}

fn message_summary(direction: &str, message: &Value) -> Value {
    json!({
        "direction": direction,
        "id": message.get("id").cloned().unwrap_or(Value::Null),
        "method": message.get("method").cloned().unwrap_or(Value::Null),
        "has_result": message.get("result").is_some(),
        "has_error": message.get("error").is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_content_length_framing_with_extra_headers() {
        let payload = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let input = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            payload.len(),
            String::from_utf8_lossy(payload)
        );
        let mut reader = Cursor::new(input.into_bytes());

        let message = read_frame(&mut reader).expect("frame").expect("message");

        assert_eq!(message["id"], 1);
        assert_eq!(message["result"], Value::Null);
    }

    #[test]
    fn rejects_unbounded_frames_before_allocation() {
        let mut reader =
            Cursor::new(format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1).into_bytes());

        let error = read_frame(&mut reader).expect_err("oversized frame");

        assert!(error.contains("limit"));
    }

    #[test]
    fn omits_null_params_from_parameterless_calls() {
        let shutdown = call_message(Some(2), "shutdown", Value::Null);
        let exit = call_message(None, "exit", Value::Null);

        assert!(!shutdown
            .as_object()
            .expect("request")
            .contains_key("params"));
        assert!(!exit
            .as_object()
            .expect("notification")
            .contains_key("params"));
    }
}
