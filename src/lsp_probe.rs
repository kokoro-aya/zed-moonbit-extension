use std::collections::{BTreeSet, VecDeque};
use std::fs;
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
    pub output: Option<PathBuf>,
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
            .position(|message| is_response_for(message, id))
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
            if is_response_for(&message, id) {
                return response_result(method, message);
            }
            if self.handle_server_request(&message)? {
                continue;
            }
            self.pending.push_back(message);
        }
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(call_message(None, method, params))
    }

    pub fn open_document(
        &mut self,
        path: &Path,
        text: &str,
        version: i64,
    ) -> Result<String, String> {
        let uri = file_uri(path)?;
        self.timeline.push(json!({
            "event": "document",
            "method": "textDocument/didOpen",
            "uri": uri,
            "version": version,
        }));
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "moonbit",
                    "version": version,
                    "text": text,
                }
            }),
        )?;
        Ok(uri)
    }

    pub fn close_document(&mut self, uri: &str) -> Result<(), String> {
        self.timeline.push(json!({
            "event": "document",
            "method": "textDocument/didClose",
            "uri": uri,
        }));
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    pub fn change_document(
        &mut self,
        uri: &str,
        version: i64,
        content_changes: Value,
    ) -> Result<(), String> {
        self.timeline.push(json!({
            "event": "document",
            "method": "textDocument/didChange",
            "uri": uri,
            "version": version,
        }));
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": content_changes,
            }),
        )
    }

    pub fn save_document(&mut self, uri: &str, text: &str) -> Result<(), String> {
        self.timeline.push(json!({
            "event": "document",
            "method": "textDocument/didSave",
            "uri": uri,
        }));
        self.notify(
            "textDocument/didSave",
            json!({
                "textDocument": { "uri": uri },
                "text": text,
            }),
        )
    }

    pub fn wait_for_diagnostic_state(
        &mut self,
        uri: &str,
        want_nonempty: bool,
        timeout: Duration,
    ) -> Result<Vec<Value>, String> {
        let deadline = Instant::now() + timeout;
        let mut observed = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "timeout waiting for {} diagnostics on {uri}; observed {}",
                    if want_nonempty { "nonempty" } else { "empty" },
                    Value::Array(observed)
                ));
            }
            let message = if let Some(message) = self.pending.pop_front() {
                message
            } else {
                self.receive_from_channel(remaining)?
            };
            if self.handle_server_request(&message)? {
                continue;
            }
            if message["method"] != "textDocument/publishDiagnostics"
                || message.pointer("/params/uri").and_then(Value::as_str) != Some(uri)
            {
                continue;
            }
            let event = normalized_diagnostic_event(&message, observed.len());
            let is_nonempty = event["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty());
            observed.push(event);
            if is_nonempty == want_nonempty {
                return Ok(observed);
            }
        }
    }

    pub fn drain_messages(&mut self, wait: Duration) -> Result<Vec<Value>, String> {
        let mut messages: Vec<Value> = self.pending.drain(..).collect();
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.receiver.recv_timeout(remaining) {
                Ok(ReaderEvent::Message(message)) => {
                    self.timeline.push(message_summary("receive", &message));
                    if !self.handle_server_request(&message)? {
                        messages.push(message);
                    }
                }
                Ok(ReaderEvent::Closed) => {
                    return Err("MoonBit LSP stdout closed while draining messages".to_string())
                }
                Ok(ReaderEvent::Error(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("MoonBit LSP reader disconnected".to_string())
                }
            }
        }
        Ok(messages)
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

    fn handle_server_request(&mut self, message: &Value) -> Result<bool, String> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let Some(id) = message.get("id").cloned() else {
            return Ok(false);
        };

        let result = match method {
            "workspace/configuration" => {
                let count = message
                    .pointer("/params/items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Value::Array(vec![Value::Null; count])
            }
            "client/registerCapability" | "window/workDoneProgress/create" => Value::Null,
            _ => Value::Null,
        };
        self.timeline.push(json!({
            "event": "server_request",
            "method": method,
            "handling": if matches!(method, "workspace/configuration" | "client/registerCapability" | "window/workDoneProgress/create") {
                "modeled"
            } else {
                "null-fallback"
            },
        }));
        self.send(response_message(id, result))?;
        Ok(true)
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

fn is_response_for(message: &Value, id: u64) -> bool {
    message.get("method").is_none() && message.get("id").and_then(Value::as_u64) == Some(id)
}

fn response_message(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
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
    let capabilities = initialize
        .get("capabilities")
        .cloned()
        .unwrap_or(Value::Null);
    require_capability(&capabilities, "hoverProvider")?;
    require_capability(&capabilities, "completionProvider")?;
    require_capability(&capabilities, "documentFormattingProvider")?;

    let geometry_path = fixture.join("geometry.mbt");
    let main_path = fixture.join("cmd/main/main.mbt");
    let geometry_text = read_source(&geometry_path)?;
    let main_text = read_source(&main_path)?;
    let geometry_uri = session.open_document(&geometry_path, &geometry_text, 1)?;
    let main_uri = session.open_document(&main_path, &main_text, 1)?;

    let hover_position = position_of(&main_text, "scale", 0)?;
    let hover = session.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": main_uri },
            "position": hover_position,
        }),
        RESPONSE_TIMEOUT,
    )?;
    if hover.is_null() {
        return Err("advertised hoverProvider returned null for the scale call".to_string());
    }

    let completion_position = position_of(&main_text, "@geometry.scale", "@geometry.".len())?;
    let completion = session.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": main_uri },
            "position": completion_position,
        }),
        RESPONSE_TIMEOUT,
    )?;
    if !completion.is_array() && !completion.is_object() {
        return Err(format!(
            "advertised completionProvider returned an unexpected result: {completion}"
        ));
    }

    let formatting = session.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": geometry_uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
        RESPONSE_TIMEOUT,
    )?;
    if !formatting.is_array() {
        return Err(format!(
            "advertised documentFormattingProvider returned an unexpected result: {formatting}"
        ));
    }

    session.close_document(&main_uri)?;
    session.close_document(&geometry_uri)?;
    let session_evidence = session.finish()?;

    Ok(json!({
        "schema": "moonbit-lab/lsp-probe/v1",
        "suite": "capabilities",
        "binary": binary,
        "session": session_evidence,
        "result": {
            "capabilities": capabilities,
            "server_info": initialize.get("serverInfo").cloned().unwrap_or(Value::Null),
            "requests": {
                "hover": hover,
                "completion": completion,
                "formatting": formatting,
            }
        }
    }))
}

pub fn run_project_roots(options: &ProbeOptions) -> Result<Value, String> {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects")
        .canonicalize()
        .map_err(|error| format!("resolve project fixtures: {error}"))?;
    let library = projects.join("geometry-library");
    let application = projects.join("geometry-application");

    let direct_library = run_root_scenario(
        options,
        "direct-library",
        &library,
        &[ModuleCase::library(&library)],
    )?;
    let direct_application = run_root_scenario(
        options,
        "direct-application",
        &application,
        &[ModuleCase::application(&application)],
    )?;
    let parent = run_root_scenario(
        options,
        "parent-workspace",
        &projects,
        &[
            ModuleCase::library(&library),
            ModuleCase::application(&application),
        ],
    )?;

    let comparisons = [
        compare_module_navigation(&direct_library, &parent, "geometry-library")?,
        compare_module_navigation(&direct_application, &parent, "geometry-application")?,
    ];

    Ok(json!({
        "schema": "moonbit-lab/lsp-probe/v1",
        "suite": "project-roots",
        "binary": direct_library["binary"],
        "scenarios": [direct_library, direct_application, parent],
        "comparisons": comparisons,
    }))
}

pub fn run_freshness(options: &ProbeOptions) -> Result<Value, String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects/geometry-library")
        .canonicalize()
        .map_err(|error| format!("resolve freshness fixture: {error}"))?;
    let path = fixture.join("geometry.mbt");
    let valid_text = read_source(&path)?;
    let mut session = LspSession::start(&options.moon, &fixture)?;
    let binary = session.binary_json();
    let initialize = session.initialize()?;
    if initialize.pointer("/capabilities/textDocumentSync") != Some(&json!(2)) {
        return Err(format!(
            "freshness suite requires incremental text sync, got {}",
            initialize
                .pointer("/capabilities/textDocumentSync")
                .unwrap_or(&Value::Null)
        ));
    }

    let uri = session.open_document(&path, &valid_text, 1)?;
    session.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
        RESPONSE_TIMEOUT,
    )?;
    let baseline_messages = session.drain_messages(Duration::from_millis(500))?;
    let baseline_diagnostics = diagnostic_events(&baseline_messages);
    if baseline_diagnostics.iter().any(|event| {
        event["uri"] == uri
            && event["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }) {
        return Err(format!(
            "valid baseline unexpectedly published diagnostics: {}",
            Value::Array(baseline_diagnostics)
        ));
    }

    let invalid_range = range_of(&valid_text, "2")?;
    session.change_document(
        &uri,
        2,
        json!([{ "range": invalid_range, "rangeLength": 1, "text": "\"broken\"" }]),
    )?;
    let invalid_diagnostics = session.wait_for_diagnostic_state(&uri, true, RESPONSE_TIMEOUT)?;

    let invalid_text = valid_text.replacen('2', "\"broken\"", 1);
    let repair_range = range_of(&invalid_text, "\"broken\"")?;
    session.change_document(
        &uri,
        3,
        json!([{ "range": repair_range, "rangeLength": 8, "text": "2" }]),
    )?;
    let repair_diagnostics = session.wait_for_diagnostic_state(&uri, false, RESPONSE_TIMEOUT)?;

    session.save_document(&uri, &valid_text)?;
    session.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
        RESPONSE_TIMEOUT,
    )?;
    let after_save_messages = session.drain_messages(Duration::from_millis(750))?;
    let after_save_diagnostics = diagnostic_events(&after_save_messages);
    if after_save_diagnostics.iter().any(|event| {
        event["uri"] == uri
            && event["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }) {
        return Err(format!(
            "didSave regressed repaired diagnostics: {}",
            Value::Array(after_save_diagnostics)
        ));
    }

    session.close_document(&uri)?;
    let session_evidence = session.finish()?;

    Ok(json!({
        "schema": "moonbit-lab/lsp-probe/v1",
        "suite": "freshness",
        "binary": binary,
        "session": session_evidence,
        "result": {
            "document_uri": uri,
            "sync_kind": 2,
            "stages": [
                {
                    "label": "valid-open",
                    "version": 1,
                    "diagnostics": baseline_diagnostics,
                    "nonempty": false,
                },
                {
                    "label": "unsaved-invalid",
                    "version": 2,
                    "diagnostics": invalid_diagnostics,
                    "nonempty": true,
                },
                {
                    "label": "unsaved-repair",
                    "version": 3,
                    "diagnostics": repair_diagnostics,
                    "nonempty": false,
                },
                {
                    "label": "saved-repair",
                    "version": 3,
                    "diagnostics": after_save_diagnostics,
                    "nonempty": false,
                }
            ]
        }
    }))
}

pub fn run_all(options: &ProbeOptions) -> Result<Value, String> {
    let capabilities = run_capabilities(options)?;
    let project_roots = run_project_roots(options)?;
    let freshness = run_freshness(options)?;
    Ok(json!({
        "schema": "moonbit-lab/lsp-probe/v1",
        "suite": "all",
        "binary": capabilities["binary"],
        "results": [capabilities, project_roots, freshness],
    }))
}

struct ModuleCase {
    label: &'static str,
    root: PathBuf,
    main: PathBuf,
    sources: Vec<PathBuf>,
}

impl ModuleCase {
    fn library(root: &Path) -> Self {
        Self {
            label: "geometry-library",
            root: root.to_path_buf(),
            main: root.join("cmd/main/main.mbt"),
            sources: vec![root.join("geometry.mbt"), root.join("common/origin.mbt")],
        }
    }

    fn application(root: &Path) -> Self {
        Self {
            label: "geometry-application",
            root: root.to_path_buf(),
            main: root.join("cmd/main/main.mbt"),
            sources: vec![root.join("report.mbt"), root.join("common/origin.mbt")],
        }
    }
}

fn run_root_scenario(
    options: &ProbeOptions,
    label: &str,
    root: &Path,
    modules: &[ModuleCase],
) -> Result<Value, String> {
    let mut session = LspSession::start(&options.moon, root)?;
    let binary = session.binary_json();
    let initialize = session.initialize()?;
    let mut opened_uris = Vec::new();
    let mut main_documents = Vec::new();

    for module in modules {
        let mut paths = module.sources.clone();
        paths.push(module.main.clone());
        for path in paths {
            let text = read_source(&path)?;
            let uri = session.open_document(&path, &text, 1)?;
            if path == module.main {
                main_documents.push((module, uri.clone(), text));
            }
            opened_uris.push(uri);
        }
    }

    for (_, uri, _) in &main_documents {
        session.request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
            RESPONSE_TIMEOUT,
        )?;
    }

    let mut navigation = Vec::new();
    for (module, uri, text) in &main_documents {
        let position = position_of(text, "project_origin", 0)?;
        let definition = session.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": position,
            }),
            RESPONSE_TIMEOUT,
        )?;
        let references = session.request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": position,
                "context": { "includeDeclaration": true },
            }),
            RESPONSE_TIMEOUT,
        )?;

        let definition_uris = location_uris(&definition);
        let reference_uris = location_uris(&references);
        require_owned_locations(module, "definition", &definition_uris)?;
        require_owned_locations(module, "references", &reference_uris)?;
        if !definition_uris
            .iter()
            .any(|uri| uri.ends_with("/common/origin.mbt"))
        {
            return Err(format!(
                "{} definition did not resolve to common/origin.mbt: {definition}",
                module.label
            ));
        }

        navigation.push(json!({
            "module": module.label,
            "source_uri": uri,
            "position": position,
            "definition": definition,
            "definition_uris": definition_uris,
            "references": references,
            "reference_uris": reference_uris,
        }));
    }

    let messages = session.drain_messages(Duration::from_secs(1))?;
    let diagnostics = diagnostic_events(&messages);
    let nonempty_diagnostics = diagnostics
        .iter()
        .filter(|event| {
            event["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        })
        .count();
    if nonempty_diagnostics != 0 {
        return Err(format!(
            "{label} published diagnostics for valid fixtures: {}",
            Value::Array(diagnostics)
        ));
    }

    for uri in opened_uris.iter().rev() {
        session.close_document(uri)?;
    }
    let session_evidence = session.finish()?;

    Ok(json!({
        "label": label,
        "binary": binary,
        "initialize_capabilities": initialize.get("capabilities").cloned().unwrap_or(Value::Null),
        "session": session_evidence,
        "opened_uris": opened_uris,
        "diagnostics": diagnostics,
        "nonempty_diagnostic_events": nonempty_diagnostics,
        "navigation": navigation,
    }))
}

fn require_owned_locations(
    module: &ModuleCase,
    request: &str,
    uris: &BTreeSet<String>,
) -> Result<(), String> {
    if uris.is_empty() {
        return Err(format!(
            "{} {request} returned no location URIs",
            module.label
        ));
    }
    let owner_prefix = url::Url::from_directory_path(
        module
            .root
            .canonicalize()
            .map_err(|error| format!("resolve {} root: {error}", module.label))?,
    )
    .map_err(|_| format!("convert {} root to URI", module.label))?
    .to_string();
    if let Some(uri) = uris.iter().find(|uri| !uri.starts_with(&owner_prefix)) {
        return Err(format!(
            "{} {request} crossed its module boundary: {uri}",
            module.label
        ));
    }
    Ok(())
}

fn compare_module_navigation(
    direct: &Value,
    parent: &Value,
    module: &str,
) -> Result<Value, String> {
    let direct_navigation = find_navigation(direct, module)?;
    let parent_navigation = find_navigation(parent, module)?;
    for key in ["definition_uris", "reference_uris"] {
        if direct_navigation[key] != parent_navigation[key] {
            return Err(format!(
                "{module} {key} differs between direct and parent roots: direct={}, parent={}",
                direct_navigation[key], parent_navigation[key]
            ));
        }
    }
    Ok(json!({
        "module": module,
        "diagnostics_equivalent": direct["nonempty_diagnostic_events"] == parent["nonempty_diagnostic_events"],
        "definition_uris_equivalent": true,
        "reference_uris_equivalent": true,
        "owned_boundary_preserved": true,
    }))
}

fn find_navigation<'a>(scenario: &'a Value, module: &str) -> Result<&'a Value, String> {
    scenario["navigation"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["module"] == module))
        .ok_or_else(|| format!("scenario lacks navigation evidence for {module}"))
}

fn location_uris(value: &Value) -> BTreeSet<String> {
    let mut uris = BTreeSet::new();
    collect_location_uris(value, &mut uris);
    uris
}

fn collect_location_uris(value: &Value, uris: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_location_uris(item, uris);
            }
        }
        Value::Object(fields) => {
            for key in ["uri", "targetUri"] {
                if let Some(uri) = fields.get(key).and_then(Value::as_str) {
                    uris.insert(uri.to_string());
                }
            }
            for value in fields.values() {
                collect_location_uris(value, uris);
            }
        }
        _ => {}
    }
}

fn diagnostic_events(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["method"] == "textDocument/publishDiagnostics")
        .map(|(sequence, message)| normalized_diagnostic_event(message, sequence))
        .collect()
}

fn normalized_diagnostic_event(message: &Value, sequence: usize) -> Value {
    let diagnostics = message
        .pointer("/params/diagnostics")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|diagnostic| {
                    json!({
                        "range": diagnostic.get("range").cloned().unwrap_or(Value::Null),
                        "severity": diagnostic.get("severity").cloned().unwrap_or(Value::Null),
                        "code": diagnostic.get("code").cloned().unwrap_or(Value::Null),
                        "message": diagnostic.get("message").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "sequence": sequence,
        "uri": message.pointer("/params/uri").cloned().unwrap_or(Value::Null),
        "version": message.pointer("/params/version").cloned().unwrap_or(Value::Null),
        "diagnostics": diagnostics,
    })
}

fn range_of(text: &str, needle: &str) -> Result<Value, String> {
    let start = position_of(text, needle, 0)?;
    let end = position_of(text, needle, needle.len())?;
    Ok(json!({ "start": start, "end": end }))
}

fn require_capability(capabilities: &Value, name: &str) -> Result<(), String> {
    match capabilities.get(name) {
        Some(Value::Bool(true)) | Some(Value::Object(_)) => Ok(()),
        actual => Err(format!(
            "MoonBit LSP did not advertise {name}: {}",
            actual.unwrap_or(&Value::Null)
        )),
    }
}

fn read_source(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("read source {}: {error}", path.display()))
}

fn file_uri(path: &Path) -> Result<String, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve source {}: {error}", path.display()))?;
    url::Url::from_file_path(&path)
        .map(String::from)
        .map_err(|_| format!("convert source path to URI: {}", path.display()))
}

fn position_of(text: &str, needle: &str, offset_in_needle: usize) -> Result<Value, String> {
    if offset_in_needle > needle.len() || !needle.is_char_boundary(offset_in_needle) {
        return Err(format!(
            "invalid byte offset {offset_in_needle} in {needle:?}"
        ));
    }
    let start = text
        .find(needle)
        .ok_or_else(|| format!("source does not contain {needle:?}"))?;
    let offset = start + offset_in_needle;
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let character = text[line_start..offset].encode_utf16().count();
    Ok(json!({ "line": line, "character": character }))
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

    #[test]
    fn positions_use_utf16_code_units() {
        let position = position_of("fn main {\n  let x = \"🌙\"\n}\n", "\"🌙\"", 5)
            .expect("position after moon emoji");

        assert_eq!(position, json!({ "line": 1, "character": 13 }));
    }
}
