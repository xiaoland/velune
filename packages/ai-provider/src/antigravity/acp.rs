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

pub(super) struct Run {
    input: ChildStdin,
    messages: mpsc::Receiver<Value>,
    reader: tokio::task::AbortHandle,
    pub ended: watch::Receiver<bool>,
    bridge: mcp::Bridge,
    _child: ChildTree,
    _guard: Option<Box<dyn Send>>,
    next_id: u64,
    session: String,
    model: String,
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
        let Process {
            command,
            input_prefix,
            cwd,
            mcp_endpoint,
            guard,
        } = launch.prepare(bridge.endpoint.clone()).await?;
        if cwd.is_empty()
            || (has_tools
                && !reqwest::Url::parse(&mcp_endpoint).is_ok_and(|url| {
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
        let mut child = ChildTree::spawn(command)?;
        let input = child.child.stdin.take().ok_or(Error::Transport)?;
        let stdout = child.child.stdout.take().ok_or(Error::Transport)?;
        let (sender, messages) = mpsc::channel(64);
        let (closed, ended) = watch::channel(false);
        let reader = tokio::spawn(async move {
            read_lines(stdout, sender).await;
            closed.send_replace(true);
        });
        let mut run = Self {
            input,
            messages,
            reader: reader.abort_handle(),
            ended,
            bridge,
            _child: child,
            _guard: guard,
            next_id: 1,
            session: String::new(),
            model: model.to_owned(),
        };
        if !input_prefix.is_empty() {
            run.write(&input_prefix).await?;
        }
        let init = run
            .rpc(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientInfo": {"name": "velune", "version": env!("CARGO_PKG_VERSION")},
                    "clientCapabilities": {}
                }),
            )
            .await?;
        if init["protocolVersion"].as_u64() != Some(1)
            || (has_images
                && init["agentCapabilities"]["promptCapabilities"]["image"].as_bool() != Some(true))
            || (has_tools
                && init["agentCapabilities"]["mcpCapabilities"]["http"].as_bool() == Some(false))
        {
            return Err(Error::Unsupported);
        }
        // Antigravity enables HTTP MCP in its process-per-session worker. Its
        // outer initialize response can omit mcpCapabilities.
        let mut params = json!({"cwd": cwd, "mcpServers": []});
        if has_tools {
            params["mcpServers"] = json!([{
                "type": "http",
                "name": "velune",
                "url": mcp_endpoint,
                "headers": [{"name": "Authorization", "value": run.bridge.authorization}],
                "_meta": {"antigravity": {"forceAllToolsEager": true}}
            }]);
        }
        if !system.is_empty() {
            params["_meta"]["antigravity"]["systemPrompt"]["append"] = json!(system);
        }
        let session = run.rpc("session/new", params).await?;
        run.session = session["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(Error::ProviderFailure)?
            .to_owned();
        run.set_model(&session).await?;
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
            self.rpc(
                "session/set_model",
                json!({"sessionId": self.session, "modelId": self.model}),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn prompt(&mut self, content: Vec<Value>) -> Result<u64, Error> {
        self.request(
            "session/prompt",
            json!({"sessionId": self.session, "prompt": content}),
        )
        .await
    }

    pub async fn cancel(&mut self) {
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            self.send(&json!({
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
                msg = self.messages.recv() => msg.ok_or(Error::Transport)?,
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
                self.handle_request(&msg).await?;
            } else if msg["id"] == prompt_id {
                let result = result(msg)?;
                let stop = result["stopReason"]
                    .as_str()
                    .ok_or(Error::ProviderFailure)?;
                return Ok(Event::Stop(stop.to_owned()));
            }
        }
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

impl Drop for Run {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn model_option(value: &Value) -> Option<&Value> {
    value["configOptions"]
        .as_array()?
        .iter()
        .find(|option| option["category"] == "model" && option["type"] == "select")
}

fn contains_option(options: &Value, model: &str) -> bool {
    options.as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["value"] == model
                || entry["options"]
                    .as_array()
                    .is_some_and(|group| group.iter().any(|option| option["value"] == model))
        })
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
