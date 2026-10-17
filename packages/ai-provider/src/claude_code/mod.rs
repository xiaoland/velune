//! Claude Code subprocess execution. Caller tools are served over a private loopback MCP server.
mod stream;
mod usage;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex as AsyncMutex, mpsc, oneshot},
};
use velune_ai::{
    InvalidContract, OperationFuture, Payload,
    http::{Header, ResponseBody, ResponseMeta},
    messages::*,
};

const MAX_LINE: usize = 8 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(600);
#[derive(Clone)]
pub struct ClaudeCodeConfig {
    pub executable: PathBuf,
    pub config_dir: PathBuf,
    /// Explicit caller namespace and session ID, encoded by the composition root.
    pub session: Option<String>,
    /// Provider identity plus authentication binding epoch; never a credential.
    pub binding: String,
}
struct Slot {
    run: AsyncMutex<Option<Run>>,
    stop: tokio::sync::watch::Sender<bool>,
    status: AtomicU8,
    control: Mutex<Option<RunControl>>,
    session: Option<String>,
    calls: Mutex<HashSet<String>>,
    invalid_results: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}
struct RunControl {
    child: std::sync::Weak<Mutex<Child>>,
    bridge: tokio::task::AbortHandle,
    reader: tokio::task::AbortHandle,
}
impl RunControl {
    fn stop(&self) {
        if let Some(child) = self.child.upgrade() {
            let _ = child.lock().unwrap().start_kill();
        }
        self.bridge.abort();
        self.reader.abort();
    }
}
impl Slot {
    fn stop(&self) {
        self.status.store(2, Ordering::Release);
        self.stop.send_replace(true);
        if let Some(session) = &self.session {
            let mut calls = self.calls.lock().unwrap();
            if !calls.is_empty() {
                self.invalid_results
                    .lock()
                    .unwrap()
                    .entry(session.clone())
                    .or_default()
                    .extend(calls.drain());
            }
        }
        if let Some(control) = self.control.lock().unwrap().as_ref() {
            control.stop();
        }
    }
}
/// Pending tool results belong to the process that issued the call, regardless of route ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Continuation {
    None,
    Pinned { binding: String },
    Invalidated,
}
pub struct ClaudeCodePool {
    sessions: Mutex<HashMap<(String, String), Arc<Slot>>>,
    executor: Executor,
    closed: AtomicBool,
    invalid_results: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}
struct Executor {
    handle: tokio::runtime::Handle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Executor {
    fn new() -> Self {
        let (stop, stopped) = oneshot::channel();
        let (ready, receive) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("velune-claude-code".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Claude Code runtime");
                let _ = ready.send(runtime.handle().clone());
                runtime.block_on(async {
                    let _ = stopped.await;
                });
            })
            .expect("Claude Code runtime thread");
        Self {
            handle: receive.recv().expect("Claude Code runtime initialization"),
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl Drop for Executor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take()
            && thread.thread().id() != std::thread::current().id()
        {
            let _ = thread.join();
        }
    }
}
impl Default for ClaudeCodePool {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            executor: Executor::new(),
            closed: AtomicBool::new(false),
            invalid_results: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl ClaudeCodePool {
    pub fn new() -> Self {
        Self::default()
    }
    /// Explicit user-turn boundary. Pending or still-running work must be cancelled first.
    pub fn begin_user_turn(&self, session: &str) -> Result<(), InvalidContract> {
        if self.closed.load(Ordering::Acquire) {
            return Err(InvalidContract("Claude Code pool is closed"));
        }
        let mut sessions = self.sessions.lock().unwrap();
        for ((_, id), slot) in sessions.iter() {
            if id == session
                && (slot.status.load(Ordering::Acquire) == 1 || slot.run.try_lock().is_err())
            {
                return Err(InvalidContract("Claude Code session still has active work"));
            }
        }
        sessions.retain(|(_, id), slot| id != session || slot.status.load(Ordering::Acquire) != 2);
        Ok(())
    }
    /// Ends unfinished interactions after the caller's turn settles, preserving completed idle processes.
    pub fn finish_user_turn(&self, session: &str) {
        for ((_, id), slot) in self.sessions.lock().unwrap().iter() {
            if id != session {
                continue;
            }
            match slot.run.try_lock() {
                Ok(mut run) => {
                    if slot.status.load(Ordering::Acquire) == 1 {
                        slot.stop();
                        *run = None;
                    }
                }
                Err(_) => slot.stop(),
            }
        }
    }
    /// Cancels active work and parked tool calls across every binding for this session.
    pub fn cancel_session(&self, session: &str) {
        for ((_, id), slot) in self.sessions.lock().unwrap().iter() {
            if id == session {
                slot.stop();
                if let Ok(mut run) = slot.run.try_lock() {
                    *run = None;
                }
            }
        }
    }
    /// Checks only the IDs in the current request's continuation tail, never historical results.
    pub fn validate_result_ids(
        &self,
        session: &str,
        ids: &[String],
    ) -> Result<(), InvalidContract> {
        if self
            .invalid_results
            .lock()
            .unwrap()
            .get(session)
            .is_some_and(|invalid| ids.iter().any(|id| invalid.contains(id)))
        {
            Err(InvalidContract(
                "Claude Code tool continuation was cancelled",
            ))
        } else {
            Ok(())
        }
    }
    pub fn continuation(&self, session: &str) -> Continuation {
        let sessions = self.sessions.lock().unwrap();
        for ((binding, id), slot) in sessions.iter() {
            if id == session && slot.status.load(Ordering::Acquire) == 1 {
                return Continuation::Pinned {
                    binding: binding.clone(),
                };
            }
        }
        if sessions
            .iter()
            .any(|((_, id), slot)| id == session && slot.status.load(Ordering::Acquire) == 2)
        {
            Continuation::Invalidated
        } else {
            Continuation::None
        }
    }
    pub fn invalidate(&self, binding: &str) {
        for ((key, _), slot) in self.sessions.lock().unwrap().iter() {
            if key == binding {
                slot.stop();
                if let Ok(mut run) = slot.run.try_lock() {
                    *run = None;
                }
            }
        }
    }
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        for slot in self.sessions.lock().unwrap().values() {
            slot.stop();
            if let Ok(mut run) = slot.run.try_lock() {
                *run = None;
            }
        }
    }
    fn slot(&self, config: &ClaudeCodeConfig) -> Result<Arc<Slot>, ()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(());
        }
        let mut sessions = self.sessions.lock().unwrap();
        let key = (
            config.binding.clone(),
            config.session.clone().unwrap_or_default(),
        );
        if let Some(slot) = sessions.get(&key) {
            return Ok(slot.clone());
        }
        sessions.retain(|_, slot| match slot.run.try_lock() {
            Ok(run) => run.is_some() || slot.status.load(Ordering::Acquire) == 2,
            Err(_) => true,
        });
        let retained = sessions
            .values()
            .filter(|slot| {
                slot.status.load(Ordering::Acquire) != 2
                    && match slot.run.try_lock() {
                        Ok(run) => run.is_some(),
                        Err(_) => true,
                    }
            })
            .count();
        if retained >= 64 {
            return Err(());
        }
        let (stop, _) = tokio::sync::watch::channel(false);
        let slot = Arc::new(Slot {
            run: AsyncMutex::new(None),
            stop,
            status: AtomicU8::new(0),
            control: Mutex::new(None),
            session: config.session.clone(),
            calls: Mutex::new(HashSet::new()),
            invalid_results: self.invalid_results.clone(),
        });
        let weak = Arc::downgrade(&slot);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(IDLE).await;
                let Some(slot) = weak.upgrade() else {
                    break;
                };
                if let Ok(mut run) = slot.run.try_lock()
                    && run
                        .as_ref()
                        .is_some_and(|run| run.touched.elapsed() >= IDLE)
                {
                    if slot.status.load(Ordering::Acquire) == 1 {
                        slot.stop();
                    }
                    *run = None;
                }
            }
        });
        if config.session.is_some() {
            sessions.insert(key, slot.clone());
        }
        Ok(slot)
    }
}
impl Drop for ClaudeCodePool {
    fn drop(&mut self) {
        self.shutdown();
    }
}
struct Lease {
    slot: Arc<Slot>,
    completed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.completed {
            self.slot.stop();
        }
    }
}
pub struct ClaudeCode {
    pool: Arc<ClaudeCodePool>,
    config: ClaudeCodeConfig,
}
impl ClaudeCode {
    pub fn new(
        pool: Arc<ClaudeCodePool>,
        config: ClaudeCodeConfig,
    ) -> Result<Self, InvalidContract> {
        if config.executable.as_os_str().is_empty()
            || config.config_dir.as_os_str().is_empty()
            || config.binding.is_empty()
            || config.session.as_ref().is_some_and(|s| s.is_empty())
        {
            return Err(InvalidContract(
                "Claude Code executable, binding and explicit session must be nonempty",
            ));
        }
        Ok(Self { pool, config })
    }
}
struct ToolCall {
    id: String,
    name: String,
    input: Value,
    reply: oneshot::Sender<Value>,
}
enum Event {
    Cli(Value),
    Tool(ToolCall),
    End,
}
struct Run {
    child: Arc<Mutex<Child>>,
    input: ChildStdin,
    events: mpsc::Receiver<Event>,
    bridge: tokio::task::JoinHandle<()>,
    reader: tokio::task::JoinHandle<()>,
    directory: PathBuf,
    signature: Value,
    history: Vec<Value>,
    pending: HashMap<String, oneshot::Sender<Value>>,
    touched: Instant,
    usage: usage::TurnUsage,
}
impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().start_kill();
        self.bridge.abort();
        self.reader.abort();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn fail(kind: MessagesErrorKind, submitted: bool) -> MessagesCompletion {
    MessagesCompletion {
        result: Err(MessagesFailure {
            kind,
            response: None,
            body: None,
            submitted,
        }),
    }
}
fn normalized(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| json!({"role":m["role"],"content":blocks(&m["content"])}))
        .collect()
}
fn signature(body: &Value) -> Value {
    json!({"model":body["model"],"system":body["system"],"tools":body["tools"],"tool_choice":body["tool_choice"],"max_tokens":body["max_tokens"],"output_config":body["output_config"],"thinking":body["thinking"]})
}
fn token() -> Result<String, ()> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| ())?;
    Ok(bytes.iter().map(|v| format!("{v:02x}")).collect())
}
async fn launch(config: &ClaudeCodeConfig, body: &Value) -> Result<Run, MessagesErrorKind> {
    let directory_nonce = token().map_err(|_| MessagesErrorKind::Transport)?;
    let token = token().map_err(|_| MessagesErrorKind::Transport)?;
    // The directory is visible in argv; its nonce must not reveal the MCP capability.
    let directory = std::env::temp_dir().join(format!("velune-claude-{directory_nonce}"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&directory)
        .map_err(|_| MessagesErrorKind::Transport)?;
    let result = launch_in(config, body, directory.clone(), token).await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result
}
async fn launch_in(
    config: &ClaudeCodeConfig,
    body: &Value,
    directory: PathBuf,
    token: String,
) -> Result<Run, MessagesErrorKind> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| MessagesErrorKind::Transport)?;
    let address = listener
        .local_addr()
        .map_err(|_| MessagesErrorKind::Transport)?;
    let (sender, events) = mpsc::channel(64);
    let tools = if body["tool_choice"]["type"] == "none" {
        Vec::new()
    } else {
        body["tools"].as_array().cloned().unwrap_or_default()
    };
    let tools = Arc::new(tools);
    let output_sender = sender.clone();
    let route = format!("/{token}");
    let app=Router::new().route(&route,post(move |Json(request):Json<Value>|{
        let tools=tools.clone(); let sender=sender.clone();
        async move {
            let id=request.get("id").cloned();
            let result=match request["method"].as_str().unwrap_or("") {
                "initialize"=>json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"velune","version":"1"}}),
                "ping"=>json!({}),
                "tools/list"=>json!({"tools":tools.iter().map(|t|json!({"name":t["name"],"description":t.get("description").cloned().unwrap_or(json!("")),"inputSchema":t["input_schema"]})).collect::<Vec<_>>()}),
                "tools/call"=>{
                    let name=request["params"]["name"].as_str().unwrap_or("");
                    if !tools.iter().any(|t|t["name"].as_str()==Some(name)){json!({"content":[{"type":"text","text":"Unknown caller tool"}],"isError":true})}
                    else {let (reply,receive)=oneshot::channel(); let call=ToolCall{id:format!("toolu_{}",token_id()),name:name.to_owned(),input:request["params"]["arguments"].clone(),reply};
                    if sender.send(Event::Tool(call)).await.is_err(){json!({"content":[{"type":"text","text":"Session closed"}],"isError":true})}
                    else {receive.await.unwrap_or_else(|_|json!({"content":[{"type":"text","text":"Session closed"}],"isError":true}))}}
                },
                _=>json!({}),
            };
            if let Some(id)=id { (axum::http::StatusCode::OK,Json(json!({"jsonrpc":"2.0","id":id,"result":result}))) }
            else {(axum::http::StatusCode::ACCEPTED,Json(Value::Null))}
        }
    }));
    let config_path = directory.join("mcp.json");
    std::fs::write(&config_path,serde_json::to_vec(&json!({"mcpServers":{"velune":{"type":"http","url":format!("http://{address}/{token}")}}})).unwrap()).map_err(|_|MessagesErrorKind::Transport)?;
    let mut command = Command::new(&config.executable);
    command
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--tools",
            "",
            "--disallowedTools",
            "EndConversation",
            "--no-chrome",
            "--allowedTools",
            "mcp__velune__*",
            "--permission-mode",
            "dontAsk",
            "--setting-sources",
            "",
            "--settings",
            "{\"disableAllHooks\":true}",
            "--model",
        ])
        .arg(
            body["model"]
                .as_str()
                .ok_or(MessagesErrorKind::InvalidInput)?,
        )
        .arg("--mcp-config")
        .arg(config_path)
        .current_dir(&directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    command.env_clear();
    // macOS resolves the user's keychain through the real login identity and home.
    for key in ["PATH", "HOME", "USER", "LOGNAME"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("CLAUDE_CONFIG_DIR", &config.config_dir);
    if let Some(max) = body["max_tokens"].as_u64() {
        command.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", max.to_string());
    }
    command
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("DISABLE_AUTOUPDATER", "1");
    if !body["system"].is_null() {
        let system = match &body["system"] {
            Value::String(value) => value.clone(),
            Value::Array(values) => values
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return Err(MessagesErrorKind::InvalidInput),
        };
        let system_path = directory.join("system.txt");
        std::fs::write(&system_path, system).map_err(|_| MessagesErrorKind::Transport)?;
        command.arg("--append-system-prompt-file").arg(system_path);
    }
    if let Some(effort) = body["output_config"]["effort"].as_str() {
        command.arg("--effort").arg(effort);
    }
    let mut child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MessagesErrorKind::Unsupported
        } else {
            MessagesErrorKind::Transport
        }
    })?;
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let bridge = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let reader = tokio::spawn(async move {
        let mut reader = BufReader::new(output);
        let mut line = Vec::new();
        loop {
            line.clear();
            match (&mut reader)
                .take((MAX_LINE + 1) as u64)
                .read_until(b'\n', &mut line)
                .await
            {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > MAX_LINE {
                        break;
                    }
                    match serde_json::from_slice(&line) {
                        Ok(value) => {
                            if output_sender.send(Event::Cli(value)).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        let _ = output_sender.send(Event::End).await;
    });
    Ok(Run {
        child: Arc::new(Mutex::new(child)),
        input,
        events,
        bridge,
        reader,
        directory,
        signature: signature(body),
        history: Vec::new(),
        pending: HashMap::new(),
        touched: Instant::now(),
        usage: usage::TurnUsage::default(),
    })
}
fn token_id() -> String {
    token().unwrap_or_else(|_| {
        format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    })
}

fn request_messages(body: &Value) -> Result<Vec<Value>, MessagesErrorKind> {
    let messages = body["messages"]
        .as_array()
        .ok_or(MessagesErrorKind::InvalidInput)?;
    if messages.is_empty() {
        return Err(MessagesErrorKind::InvalidInput);
    }
    for message in messages {
        if !matches!(message["role"].as_str(), Some("user" | "assistant"))
            || !(message["content"].is_string() || message["content"].is_array())
        {
            return Err(MessagesErrorKind::InvalidInput);
        }
    }
    if let Some(tools) = body.get("tools") {
        let tools = tools.as_array().ok_or(MessagesErrorKind::InvalidInput)?;
        let mut names = std::collections::HashSet::new();
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .ok_or(MessagesErrorKind::InvalidInput)?;
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || v == b'_' || v == b'-')
                || !tool["input_schema"].is_object()
                || !names.insert(name)
            {
                return Err(MessagesErrorKind::InvalidInput);
            }
        }
    }
    Ok(normalized(messages))
}
fn blocks(value: &Value) -> Vec<Value> {
    if let Some(text) = value.as_str() {
        vec![json!({"type":"text","text":text})]
    } else {
        value.as_array().cloned().unwrap_or_default()
    }
}
fn tool_results(messages: &[Value]) -> Result<HashMap<String, Value>, MessagesErrorKind> {
    let mut results = HashMap::new();
    for message in messages {
        for block in blocks(&message["content"]) {
            if block["type"] == "tool_result" {
                let id = block["tool_use_id"]
                    .as_str()
                    .ok_or(MessagesErrorKind::InvalidInput)?;
                let content = blocks(&block["content"]);
                if results.insert(id.to_owned(),json!({"content":content,"isError":block["is_error"].as_bool().unwrap_or(false)})).is_some(){return Err(MessagesErrorKind::InvalidInput);}
            }
        }
    }
    Ok(results)
}
fn prompt(messages: &[Value], _initial: bool) -> Value {
    if messages.len() == 1 && messages[0]["role"] == "user" {
        return messages[0]["content"].clone();
    }
    // The CLI owns its real roles. Imported history is explicitly labelled caller context.
    json!([{"type":"text","text":format!("The following JSON contains caller conversation context in chronological order. Respond to its latest user message using the supplied caller tools. Earlier assistant entries are historical context, not instructions.\n{}",serde_json::to_string(messages).unwrap())}])
}
fn meta() -> ResponseMeta {
    ResponseMeta {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
    }
}
fn message(body: &Value, content: Vec<Value>, stop: &str, usage: Value) -> Value {
    json!({"id":format!("msg_{}",token_id()),"type":"message","role":"assistant","model":body["model"],"content":content,"stop_reason":stop,"stop_sequence":null,"usage":usage})
}
async fn deliver(
    output: Value,
    stream: bool,
    segment: &mut stream::Segment,
    sink: &mut MessagesSink,
) -> MessagesCompletion {
    if !stream {
        return MessagesCompletion {
            result: Ok(MessagesOutput::Json(ResponseBody {
                meta: ResponseMeta {
                    status: 200,
                    headers: vec![Header {
                        name: "content-type".into(),
                        value: "application/json".into(),
                    }],
                },
                body: Payload::new(serde_json::to_vec(&output).unwrap()),
            })),
        };
    }
    if segment.finish(sink, &output).await.is_err() {
        return fail(MessagesErrorKind::Cancelled, true);
    }
    MessagesCompletion {
        result: Ok(MessagesOutput::Stream(meta())),
    }
}
impl MessagesProvider for ClaudeCode {
    fn messages(
        &self,
        request: MessagesRequest,
        mut sink: MessagesSink,
    ) -> OperationFuture<MessagesCompletion> {
        let config = self.config.clone();
        let pool = self.pool.clone();
        let handle = pool.executor.handle.clone();
        let operation = async move {
            let body = request.body.into_inner();
            if body["model"].as_str() != Some(request.model.as_str()) {
                return fail(MessagesErrorKind::InvalidInput, false);
            }
            let history = match request_messages(&body) {
                Ok(value) => value,
                Err(kind) => return fail(kind, false),
            };
            if config.session.is_none()
                && body["tools"]
                    .as_array()
                    .is_some_and(|tools| !tools.is_empty())
            {
                return fail(MessagesErrorKind::Unsupported, false);
            }
            let latest_results = match tool_results(&history[history.len() - 1..]) {
                Ok(value) => value,
                Err(kind) => return fail(kind, false),
            };
            if let Some(session) = &config.session
                && pool
                    .validate_result_ids(
                        session,
                        &latest_results.keys().cloned().collect::<Vec<_>>(),
                    )
                    .is_err()
            {
                return fail(MessagesErrorKind::InvalidInput, false);
            }
            if config.session.is_none() && !latest_results.is_empty() {
                return fail(MessagesErrorKind::InvalidInput, false);
            }
            let slot = match pool.slot(&config) {
                Ok(value) => value,
                Err(()) => return fail(MessagesErrorKind::ProviderFailure, false),
            };
            let mut stop = slot.stop.subscribe();
            if *stop.borrow() {
                return fail(MessagesErrorKind::Cancelled, false);
            }
            let mut guard = match slot.run.try_lock() {
                Ok(guard) => guard,
                Err(_) => return fail(MessagesErrorKind::InvalidInput, false),
            };
            let mut initial = guard.is_none();
            if initial && !latest_results.is_empty() {
                return fail(MessagesErrorKind::InvalidInput, false);
            }
            let mut run = if let Some(run) = guard.take() {
                run
            } else {
                match launch(&config, &body).await {
                    Ok(value) => value,
                    Err(kind) => return fail(kind, false),
                }
            };
            *slot.control.lock().unwrap() = Some(RunControl {
                child: Arc::downgrade(&run.child),
                bridge: run.bridge.abort_handle(),
                reader: run.reader.abort_handle(),
            });
            let mut lease = Lease {
                slot: slot.clone(),
                completed: false,
            };
            if *stop.borrow() {
                return fail(MessagesErrorKind::Cancelled, true);
            }
            if !initial && (run.signature != signature(&body) || !history.starts_with(&run.history))
            {
                if !run.pending.is_empty() {
                    return fail(MessagesErrorKind::InvalidInput, true);
                }
                drop(run);
                run = match launch(&config, &body).await {
                    Ok(value) => value,
                    Err(kind) => return fail(kind, false),
                };
                *slot.control.lock().unwrap() = Some(RunControl {
                    child: Arc::downgrade(&run.child),
                    bridge: run.bridge.abort_handle(),
                    reader: run.reader.abort_handle(),
                });
                initial = true;
                if *stop.borrow() {
                    slot.stop();
                    return fail(MessagesErrorKind::Cancelled, true);
                }
            }
            let since = if initial {
                &history[..]
            } else {
                &history[run.history.len()..]
            };
            let results = match tool_results(if initial { &[][..] } else { since }) {
                Ok(value) => value,
                Err(kind) => return fail(kind, true),
            };
            if !run.pending.is_empty() {
                if since.iter().any(|message| {
                    message["role"] != "user"
                        || blocks(&message["content"])
                            .iter()
                            .any(|block| block["type"] != "tool_result")
                }) {
                    return fail(MessagesErrorKind::Unsupported, true);
                }
                if results.len() != run.pending.len()
                    || results.keys().any(|id| !run.pending.contains_key(id))
                {
                    return fail(MessagesErrorKind::InvalidInput, true);
                }
                // Validate every result before unblocking any real CLI operation.
                for (id, result) in results {
                    if run.pending.remove(&id).unwrap().send(result).is_err() {
                        return fail(MessagesErrorKind::ProviderFailure, true);
                    }
                }
            } else {
                if !results.is_empty() || since.is_empty() {
                    return fail(MessagesErrorKind::InvalidInput, true);
                }
                run.usage = usage::TurnUsage::default();
                let input = json!({"type":"user","message":{"role":"user","content":prompt(since,initial)}});
                let bytes = match serde_json::to_vec(&input) {
                    Ok(value) => value,
                    Err(_) => return fail(MessagesErrorKind::InvalidInput, true),
                };
                let write = async {
                    run.input.write_all(&bytes).await?;
                    run.input.write_all(b"\n").await?;
                    run.input.flush().await
                };
                let written = tokio::select! {_=stop.changed()=>return fail(MessagesErrorKind::Cancelled,true),written=write=>written};
                if written.is_err() {
                    return fail(MessagesErrorKind::Transport, true);
                }
            }
            let mut content = Vec::new();
            let mut segment = stream::Segment::new(request.stream, body["model"].clone());
            let mut partial = false;
            let mut indexes = HashMap::<u64, usize>::new();
            let deadline = tokio::time::sleep(Duration::from_secs(1800));
            tokio::pin!(deadline);
            let mut outcome = loop {
                let event = tokio::select! { _=stop.changed()=>return fail(MessagesErrorKind::Cancelled,true),_= &mut deadline=>return fail(MessagesErrorKind::Timeout,true),event=run.events.recv()=>event };
                if *stop.borrow() {
                    return fail(MessagesErrorKind::Cancelled, true);
                }
                match event {
                    Some(Event::Tool(call)) => {
                        {
                            let mut calls = slot.calls.lock().unwrap();
                            if slot.status.load(Ordering::Acquire) == 2 {
                                return fail(MessagesErrorKind::Cancelled, true);
                            }
                            calls.insert(call.id.clone());
                        }
                        let block = json!({"type":"tool_use","id":call.id,"name":call.name,"input":call.input});
                        if segment
                            .block(&mut sink, content.len(), &block)
                            .await
                            .is_err()
                        {
                            return fail(MessagesErrorKind::Cancelled, true);
                        }
                        content.push(block);
                        run.pending.insert(call.id, call.reply);
                        slot.status.store(1, Ordering::Release);
                        break message(&body, content, "tool_use", run.usage.tool_segment());
                    }
                    Some(Event::Cli(value)) => match value["type"].as_str() {
                        Some("stream_event") => {
                            let event = &value["event"];
                            match event["type"].as_str() {
                                Some("message_start") => {
                                    partial = false;
                                    indexes.clear();
                                    if value["parent_tool_use_id"].is_null() {
                                        run.usage.message(&event["message"], true);
                                    }
                                }
                                Some("message_delta") => {
                                    if value["parent_tool_use_id"].is_null() {
                                        run.usage.stream_delta(event);
                                    }
                                }
                                Some("message_stop") => {
                                    if value["parent_tool_use_id"].is_null() {
                                        run.usage.stream_stop();
                                    }
                                }
                                Some("content_block_start") => {
                                    let block = &event["content_block"];
                                    if matches!(
                                        block["type"].as_str(),
                                        Some("text" | "thinking" | "redacted_thinking")
                                    ) {
                                        partial = true;
                                        let Some(index) = event["index"].as_u64() else {
                                            return fail(MessagesErrorKind::ProviderFailure, true);
                                        };
                                        indexes.insert(index, content.len());
                                        if segment
                                            .block(&mut sink, content.len(), block)
                                            .await
                                            .is_err()
                                        {
                                            return fail(MessagesErrorKind::Cancelled, true);
                                        }
                                        content.push(block.clone());
                                    }
                                }
                                Some("content_block_delta") => {
                                    if let Some(index) = event["index"]
                                        .as_u64()
                                        .and_then(|index| indexes.get(&index))
                                        .copied()
                                    {
                                        let delta = &event["delta"];
                                        let key = match delta["type"].as_str() {
                                            Some("text_delta") => Some("text"),
                                            Some("thinking_delta") => Some("thinking"),
                                            Some("signature_delta") => Some("signature"),
                                            _ => None,
                                        };
                                        if let Some(key) = key
                                            && let Some(value) = delta[key].as_str()
                                        {
                                            let block = &mut content[index];
                                            let text = block[key].as_str().unwrap_or("").to_owned()
                                                + value;
                                            block[key] = json!(text);
                                        }
                                        if segment.delta(&mut sink, index, delta).await.is_err() {
                                            return fail(MessagesErrorKind::Cancelled, true);
                                        }
                                    }
                                }
                                Some("content_block_stop") => {
                                    if event["index"]
                                        .as_u64()
                                        .is_some_and(|index| indexes.contains_key(&index))
                                        && segment.close(&mut sink).await.is_err()
                                    {
                                        return fail(MessagesErrorKind::Cancelled, true);
                                    }
                                }
                                Some("error") => {
                                    return fail(MessagesErrorKind::ProviderFailure, true);
                                }
                                _ => {}
                            }
                        }
                        Some("assistant") => {
                            if let Some(error) = value["error"].as_str() {
                                let kind = match error {
                                    "authentication_failed" => MessagesErrorKind::Authentication,
                                    "rate_limit" => MessagesErrorKind::RateLimited,
                                    "billing_error" => MessagesErrorKind::Permission,
                                    "invalid_request" => MessagesErrorKind::InvalidInput,
                                    _ => MessagesErrorKind::ProviderFailure,
                                };
                                return fail(kind, true);
                            }
                            if value["parent_tool_use_id"].is_null() {
                                run.usage.message(&value["message"], false);
                            }
                            if !partial && let Some(blocks) = value["message"]["content"].as_array()
                            {
                                for block in blocks {
                                    if matches!(
                                        block["type"].as_str(),
                                        Some("text" | "thinking" | "redacted_thinking")
                                    ) {
                                        if segment
                                            .block(&mut sink, content.len(), block)
                                            .await
                                            .is_err()
                                        {
                                            return fail(MessagesErrorKind::Cancelled, true);
                                        }
                                        content.push(block.clone());
                                    }
                                }
                            }
                            partial = false;
                        }
                        Some("conversation_reset") => run.usage.reset_observed(),
                        Some("result") => {
                            if value["subtype"].as_str() != Some("success")
                                || value["is_error"].as_bool() == Some(true)
                            {
                                return fail(MessagesErrorKind::ProviderFailure, true);
                            }
                            let usage = run.usage.final_segment(&value["usage"]);
                            break message(&body, content, "end_turn", usage);
                        }
                        _ => {}
                    },
                    Some(Event::End) | None => {
                        return fail(MessagesErrorKind::ProviderFailure, true);
                    }
                }
            };
            outcome["id"] = json!(segment.id);
            run.history = history;
            run.history
                .push(json!({"role":"assistant","content":outcome["content"]}));
            run.touched = Instant::now();
            let completion = deliver(outcome, request.stream, &mut segment, &mut sink).await;
            if *stop.borrow() {
                return fail(MessagesErrorKind::Cancelled, true);
            }
            if completion.result.is_ok() {
                slot.status.store(
                    if run.pending.is_empty() { 0 } else { 1 },
                    Ordering::Release,
                );
                if config.session.is_some() {
                    *guard = Some(run);
                }
                lease.completed = true;
            }
            completion
        };
        Box::pin(async move {
            let mut task = AbortOnDrop(handle.spawn(operation));
            (&mut task.0)
                .await
                .unwrap_or_else(|_| fail(MessagesErrorKind::ProviderFailure, true))
        })
    }
}
