//! ACP stdio transport and session/model negotiation.
use super::{Launch, Process, mcp, process::ChildTree};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, watch},
};
use velune_ai::NativeErrorKind as Error;

const MAX_LINE: u64 = 8 * 1024 * 1024;

struct Connection {
    input: ChildStdin,
    messages: mpsc::Receiver<Value>,
    reader: tokio::task::AbortHandle,
    ended: watch::Receiver<bool>,
    _child: ChildTree,
    _guard: Option<Box<dyn Send>>,
    next_id: u64,
}

pub(super) struct Run {
    connection: Connection,
    pub ended: watch::Receiver<bool>,
    bridge: mcp::Bridge,
    session: String,
    model: String,
}

/// An exact model identifier and display name advertised by the ACP session.
pub struct Model {
    pub id: String,
    pub name: String,
}

/// Check that an isolated ACP session can start and read its model catalog.
/// No model selection, authentication method, MCP server, or prompt is submitted.
/// The caller owns the timeout and the process's execution target.
pub async fn probe_models(process: Process) -> Result<Vec<Model>, Error> {
    let cwd = &process.cwd;
    if cwd.is_empty() {
        return Err(Error::InvalidInput);
    }
    let params = json!({"cwd": cwd, "mcpServers": []});
    let mut connection = Connection::open(process).await?;
    let init = connection.initialize().await?;
    let (id, session) = connection.new_session(params).await?;
    if init["agentCapabilities"]["sessionCapabilities"]["close"].is_object() {
        let _ = tokio::time::timeout(
            Duration::from_millis(150),
            connection.rpc("session/close", json!({"sessionId": id})),
        )
        .await;
    }
    Ok(catalog(&session))
}

pub(super) enum Event {
    Content(Value),
    Tool(mcp::Call),
    Stop(String),
}

impl Run {
    pub async fn start(
        launch: &dyn Launch,
        tools: mcp::Tools,
        system: &str,
        model: &str,
        has_images: bool,
    ) -> Result<Self, Error> {
        let has_tools = !tools.is_empty();
        let bridge = mcp::Bridge::start(tools).await?;
        let process = launch.prepare(bridge.endpoint.clone()).await?;
        if process.cwd.is_empty()
            || (has_tools
                && !reqwest::Url::parse(&process.mcp_endpoint).is_ok_and(|url| {
                    url.scheme() == "http"
                        && url.host_str() == Some("127.0.0.1")
                        && url.port().is_some()
                        && url.path() == "/mcp"
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                }))
        {
            return Err(Error::InvalidInput);
        }
        let mut params = json!({"cwd": process.cwd, "mcpServers": []});
        let endpoint = has_tools.then(|| process.mcp_endpoint.clone());
        let mut connection = Connection::open(process).await?;
        let init = connection.initialize().await?;
        if (has_images
            && init["agentCapabilities"]["promptCapabilities"]["image"].as_bool() != Some(true))
            || (has_tools
                && init["agentCapabilities"]["mcpCapabilities"]["http"].as_bool() == Some(false))
        {
            return Err(Error::Unsupported);
        }
        // Antigravity enables HTTP MCP in its process-per-session worker. Its
        // outer initialize response can omit mcpCapabilities.
        if let Some(endpoint) = endpoint {
            params["mcpServers"] = json!([{
                "type": "http",
                "name": "velune",
                "url": endpoint,
                "headers": [{"name": "Authorization", "value": bridge.authorization}],
                "_meta": {"antigravity": {"forceAllToolsEager": true}}
            }]);
        }
        if !system.is_empty() {
            params["_meta"]["antigravity"]["systemPrompt"]["append"] = json!(system);
        }
        let (session, settings) = connection.new_session(params).await?;
        let mut run = Self {
            ended: connection.ended.clone(),
            connection,
            bridge,
            session,
            model: model.to_owned(),
        };
        run.set_model(&settings).await?;
        Ok(run)
    }

    async fn set_model(&mut self, session: &Value) -> Result<(), Error> {
        if let Some(option) = model_option(session) {
            if !contains_option(&option["options"], &self.model) {
                return Err(Error::Unsupported);
            }
            if option["currentValue"] != self.model {
                let id = option["id"].as_str().ok_or(Error::ProviderFailure)?;
                let result = self
                    .connection
                    .rpc(
                        "session/set_config_option",
                        json!({"sessionId": self.session, "configId": id, "value": self.model}),
                    )
                    .await?;
                if model_option(&result)
                    .is_none_or(|option| option["id"] != id || option["currentValue"] != self.model)
                {
                    return Err(Error::ProviderFailure);
                }
            }
            return Ok(());
        }
        let models = &session["models"];
        let available = models["availableModels"]
            .as_array()
            .ok_or(Error::Unsupported)?;
        if !available.iter().any(|entry| entry["modelId"] == self.model) {
            return Err(Error::Unsupported);
        }
        if models["currentModelId"] != self.model {
            self.connection
                .rpc(
                    "session/set_model",
                    json!({"sessionId": self.session, "modelId": self.model}),
                )
                .await?;
        }
        Ok(())
    }

    pub async fn prompt(&mut self, content: Vec<Value>) -> Result<u64, Error> {
        self.connection
            .request(
                "session/prompt",
                json!({"sessionId": self.session, "prompt": content}),
            )
            .await
    }

    pub async fn cancel(&mut self) {
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            self.connection.send(&json!({
                "jsonrpc": "2.0","method": "session/cancel","params": {"sessionId": self.session}
            })),
        )
        .await;
    }

    pub async fn event(&mut self, prompt_id: u64) -> Result<Event, Error> {
        loop {
            let msg = tokio::select! {
                // Prefer a terminal or already emitted text to any queued tool calls.
                biased;
                msg = self.connection.messages.recv() => msg.ok_or(Error::Transport)?,
                call = self.bridge.calls.recv() => {
                    return call.map(Event::Tool).ok_or(Error::Transport);
                }
            };
            if msg["method"] == "session/update" && msg["params"]["sessionId"] == self.session {
                let update = &msg["params"]["update"];
                match update["sessionUpdate"].as_str() {
                    Some("agent_message_chunk" | "agent_thought_chunk") => {
                        return Ok(Event::Content(update.clone()));
                    }
                    Some("config_option_update")
                        if model_option(update)
                            .is_some_and(|option| option["currentValue"] != self.model) =>
                    {
                        return Err(Error::ProviderFailure);
                    }
                    _ => {}
                }
            } else if msg.get("method").is_some() {
                self.connection.handle_request(&msg).await?;
            } else if msg["id"] == prompt_id {
                let result = result(msg)?;
                let stop = result["stopReason"]
                    .as_str()
                    .ok_or(Error::ProviderFailure)?;
                return Ok(Event::Stop(stop.to_owned()));
            }
        }
    }
}

impl Connection {
    async fn open(process: Process) -> Result<Self, Error> {
        let Process {
            command,
            input_prefix,
            guard,
            ..
        } = process;
        let mut child = ChildTree::spawn(command)?;
        let input = child.child.stdin.take().ok_or(Error::Transport)?;
        let stdout = child.child.stdout.take().ok_or(Error::Transport)?;
        let (sender, messages) = mpsc::channel(64);
        let (closed, ended) = watch::channel(false);
        let reader = tokio::spawn(async move {
            read_lines(stdout, sender).await;
            closed.send_replace(true);
        });
        let mut connection = Self {
            input,
            messages,
            reader: reader.abort_handle(),
            ended,
            _child: child,
            _guard: guard,
            next_id: 1,
        };
        if !input_prefix.is_empty() {
            connection.write(&input_prefix).await?;
        }
        Ok(connection)
    }

    async fn initialize(&mut self) -> Result<Value, Error> {
        let init = self
            .rpc(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientInfo": {"name": "velune", "version": env!("CARGO_PKG_VERSION")},
                    "clientCapabilities": {}
                }),
            )
            .await?;
        if init["protocolVersion"].as_u64() != Some(1) {
            return Err(Error::Unsupported);
        }
        Ok(init)
    }

    async fn new_session(&mut self, params: Value) -> Result<(String, Value), Error> {
        let session = self.rpc("session/new", params).await?;
        let id = session["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(Error::ProviderFailure)?
            .to_owned();
        Ok((id, session))
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.request(method, params).await?;
        loop {
            let msg = self.messages.recv().await.ok_or(Error::Transport)?;
            if msg.get("method").is_some() {
                self.handle_request(&msg).await?;
            } else if msg["id"] == id {
                return result(msg);
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<u64, Error> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        Ok(id)
    }

    async fn handle_request(&mut self, msg: &Value) -> Result<(), Error> {
        let Some(id) = msg.get("id") else {
            return Ok(());
        };
        let response = if msg["method"] == "session/request_permission" {
            // Tools cross the authenticated MCP bridge. Permission notices
            // cannot authorize file, shell or other native ACP operations.
            json!({"result": {"outcome": {"outcome": "cancelled"}}})
        } else {
            json!({"error": {"code": -32601, "message": "Method not available"}})
        };
        let mut response = response;
        response["jsonrpc"] = json!("2.0");
        response["id"] = id.clone();
        self.send(&response).await
    }

    async fn send(&mut self, message: &Value) -> Result<(), Error> {
        let mut bytes = serde_json::to_vec(message).map_err(|_| Error::InvalidInput)?;
        bytes.push(b'\n');
        self.write(&bytes).await
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.input
            .write_all(bytes)
            .await
            .map_err(|_| Error::Transport)?;
        self.input.flush().await.map_err(|_| Error::Transport)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn model_option(value: &Value) -> Option<&Value> {
    value["configOptions"].as_array()?.iter().find(|option| {
        option["type"] == "select"
            && (option["category"] == "model"
                || (option["category"].is_null() && option["id"] == "model"))
    })
}

fn choices(options: &Value) -> impl Iterator<Item = &Value> {
    options.as_array().into_iter().flatten().flat_map(|entry| {
        entry["options"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(entry))
    })
}

fn contains_option(options: &Value, model: &str) -> bool {
    choices(options).any(|option| option["value"] == model)
}

fn catalog(session: &Value) -> Vec<Model> {
    if let Some(option) = model_option(session) {
        return choices(&option["options"])
            .filter_map(|entry| catalog_entry(&entry["value"], &entry["name"]))
            .collect();
    }
    session["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| catalog_entry(&entry["modelId"], &entry["name"]))
        .collect()
}

fn catalog_entry(id: &Value, name: &Value) -> Option<Model> {
    Some(Model {
        id: id.as_str()?.to_owned(),
        name: name.as_str().unwrap_or_default().to_owned(),
    })
}

fn result(mut message: Value) -> Result<Value, Error> {
    if let Some(code) = message["error"]["code"].as_i64() {
        return Err(match code {
            -32000 => Error::Authentication,
            -32601 => Error::Unsupported,
            -32800 => Error::Cancelled,
            _ => Error::ProviderFailure,
        });
    }
    message
        .get_mut("result")
        .map(Value::take)
        .ok_or(Error::ProviderFailure)
}

async fn read_lines(stdout: ChildStdout, sender: mpsc::Sender<Value>) {
    let mut stdout = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut stdout)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
            .await
        {
            Ok(count) if count > 0 && count as u64 <= MAX_LINE && line.ends_with(b"\n") => {}
            _ => return,
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            return;
        };
        if message["jsonrpc"] != "2.0" || sender.send(message).await.is_err() {
            return;
        }
    }
}
