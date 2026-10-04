//! Loopback OpenAI Chat Completions gateway for the Pi adapter.
//!
//! This is a deliberately narrow ingress: one explicit model route, no
//! fallback, and no LAN listener. Credential references are resolved only by
//! the host-provided platform helper at dispatch time.
use crate::gateway::{GatewayConfig, GatewayProtocol};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use velune_ai::{
    AiService, Payload,
    direct::DirectAiService,
    ids::{AttemptId, CallId, ConfigRevision, ModelId, ProviderId},
    observation::Quantity,
    provider::ProviderBinding,
    sampling::{
        FinishReason, Message, SamplingDelta, SamplingEvent, SamplingEventKind, SamplingInput,
        ToolCall, ToolDefinition,
    },
};
use velune_ai_provider::{
    config::{
        ChatCompletionsConfig, CredentialRef, HttpEndpoint, ProtocolConfig, ProviderConfig,
        Transport,
    },
    openai::ChatCompletions,
};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_HEADERS_BYTES: usize = 64 * 1024;
const MAX_REASONING_LEVEL_BYTES: usize = 64;

#[derive(Debug)]
pub struct GatewayError(&'static str);
impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for GatewayError {}

struct RouteTarget {
    model: crate::gateway::ModelDefinition,
    provider_id: String,
    service: Arc<dyn AiService>,
}

pub struct Runner {
    endpoint: String,
    token: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Runner {
    pub fn start(
        config: GatewayConfig,
        credential_resolver: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate().map_err(GatewayError)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let token = ephemeral_token()?;
        let routes = build_routes(&config, credential_resolver)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_token = token.clone();
        let handle = thread::Builder::new()
            .name("velune-ai-gateway".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => return,
                };
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = handle_connection(
                                stream,
                                &thread_token,
                                &routes,
                                &runtime,
                                &thread_stop,
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            endpoint: format!("http://127.0.0.1:{}/v1", address.port()),
            token,
            stop,
            handle: Some(handle),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

static TOKEN: AtomicU64 = AtomicU64::new(1);

fn ephemeral_token() -> Result<String, Box<dyn std::error::Error>> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| GatewayError("gateway token entropy"))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").expect("string write");
    }
    Ok(token)
}

fn build_routes(
    config: &GatewayConfig,
    credential_resolver: Option<PathBuf>,
) -> Result<BTreeMap<String, RouteTarget>, Box<dyn std::error::Error>> {
    let revision = ConfigRevision::new(1).map_err(|_| GatewayError("gateway revision"))?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()?;
    let required_providers: BTreeSet<_> = config
        .routes
        .iter()
        .map(|route| route.provider_id.as_str())
        .collect();
    let mut bindings = BTreeMap::<String, Arc<ProviderBinding>>::new();
    for provider in &config.providers {
        if !required_providers.contains(provider.id.as_str()) {
            continue;
        }
        let GatewayProtocol::ChatCompletionsV1 = provider.protocol else {
            return Err(Box::new(GatewayError("provider protocol is unsupported")));
        };
        let endpoint = parse_endpoint(&provider.endpoint)?;
        let model_ids = provider
            .models
            .iter()
            .map(|binding| ModelId::new(binding.model_id.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| GatewayError("provider model id"))?;
        let protocol = ProtocolConfig::ChatCompletions(ChatCompletionsConfig { endpoint });
        let credential_ref = provider.credential_ref.clone();
        let credential = CredentialRef::new(
            credential_ref
                .clone()
                .ok_or(GatewayError("provider credential reference is required"))?,
        )
        .map_err(|_| GatewayError("provider credential reference"))?;
        let provider_config = ProviderConfig::new(
            ProviderId::new(provider.id.clone()).map_err(|_| GatewayError("provider id"))?,
            revision,
            protocol,
            credential,
            provider
                .models
                .iter()
                .zip(model_ids.iter())
                .map(|(binding, id)| {
                    velune_ai_provider::config::ModelMapping::new(
                        id.clone(),
                        binding.external_model_id.clone(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| GatewayError("provider model mapping"))?,
        )
        .map_err(|_| GatewayError("provider configuration"))?;
        let resolver = credential_resolver.clone();
        let adapter = ChatCompletions::with_resolver(
            provider_config,
            client.clone(),
            Arc::new(move |reference| {
                #[cfg(test)]
                if reference == "fixture" {
                    return Some("loopback-fixture".into());
                }
                let path = resolver.as_ref()?;
                let output = Command::new(path).arg(reference).output().ok()?;
                if !output.status.success() {
                    return None;
                }
                let value = String::from_utf8(output.stdout).ok()?;
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_owned())
            }),
        )
        .map_err(|_| GatewayError("provider adapter"))?;
        let binding = ProviderBinding::new(
            ProviderId::new(provider.id.clone()).map_err(|_| GatewayError("provider id"))?,
            revision,
            model_ids,
            Arc::new(adapter),
        )
        .map_err(|_| GatewayError("provider binding"))?;
        bindings.insert(provider.id.clone(), Arc::new(binding));
    }
    let mut routes = BTreeMap::new();
    for route in &config.routes {
        let model = config
            .model(&route.model_id)
            .ok_or(GatewayError("model route is missing"))?;
        let provider_id = route.provider_id.clone();
        let binding = bindings
            .get(&provider_id)
            .ok_or(GatewayError("route provider is missing"))?;
        let service = Arc::new(DirectAiService::new(Arc::clone(binding))) as Arc<dyn AiService>;
        routes.insert(
            model.id.clone(),
            RouteTarget {
                model: model.clone(),
                provider_id,
                service,
            },
        );
    }
    Ok(routes)
}

fn parse_endpoint(value: &str) -> Result<HttpEndpoint, Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(value).map_err(|_| GatewayError("provider endpoint"))?;
    let transport = match url.scheme() {
        "http" => Transport::Http,
        "https" => Transport::Https,
        _ => return Err(Box::new(GatewayError("provider endpoint scheme"))),
    };
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Box::new(GatewayError("provider endpoint query")));
    }
    let host = url
        .host_str()
        .ok_or(GatewayError("provider endpoint host"))?;
    let port = url
        .port_or_known_default()
        .ok_or(GatewayError("provider endpoint port"))?;
    Ok(HttpEndpoint::new(transport, host, port, url.path())
        .map_err(|_| GatewayError("provider endpoint"))?)
}

fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    routes: &BTreeMap<String, RouteTarget>,
    runtime: &tokio::runtime::Runtime,
    stop: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":error.to_string()}}),
            )?;
            return Ok(());
        }
    };
    if request.method != "POST" || request.path != "/v1/chat/completions" {
        write_json(
            &mut stream,
            "404 Not Found",
            json!({"error":{"message":"unsupported gateway path"}}),
        )?;
        return Ok(());
    }
    if request.headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {token}"))
    {
        write_json(
            &mut stream,
            "401 Unauthorized",
            json!({"error":{"message":"gateway authorization failed"}}),
        )?;
        return Ok(());
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(_) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":"gateway request JSON"}}),
            )?;
            return Ok(());
        }
    };
    let Some(model_id) = body["model"].as_str() else {
        write_json(
            &mut stream,
            "400 Bad Request",
            json!({"error":{"message":"gateway model"}}),
        )?;
        return Ok(());
    };
    let Some(target) = routes.get(model_id) else {
        write_json(
            &mut stream,
            "422 Unprocessable Entity",
            json!({"error":{"message":"gateway model route"}}),
        )?;
        return Ok(());
    };
    let input = match sampling_input(
        &body,
        target.model.max_output_tokens,
        &target.model.reasoning_levels,
    ) {
        Ok(input) => input,
        Err(error) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":error.to_string()}}),
            )?;
            return Ok(());
        }
    };
    let call = CallId::new(format!(
        "gateway-call-{}",
        TOKEN.fetch_add(1, Ordering::Relaxed)
    ))
    .map_err(|_| GatewayError("gateway call id"))?;
    let attempt = AttemptId::new(format!(
        "gateway-attempt-{}",
        TOKEN.fetch_add(1, Ordering::Relaxed)
    ))
    .map_err(|_| GatewayError("gateway attempt id"))?;
    let request = velune_ai::sampling::SamplingRequest {
        call_id: call,
        provider: ProviderId::new(target.provider_id.clone())
            .map_err(|_| GatewayError("gateway provider id"))?,
        model: ModelId::new(model_id).map_err(|_| GatewayError("gateway model id"))?,
        input,
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    let monitor = stream.try_clone()?;
    monitor.set_read_timeout(Some(Duration::from_millis(20)))?;
    let output = Arc::new(Mutex::new(stream));
    let sink_output = Arc::clone(&output);
    let cancelled = Arc::new(AtomicBool::new(false));
    let sink_cancelled = Arc::clone(&cancelled);
    let prepared = target
        .service
        .sampling(
            request,
            attempt,
            Box::new(move |event| {
                let mut stream = sink_output.lock().expect("gateway stream");
                if write_event(&mut stream, event).is_err() {
                    sink_cancelled.store(true, Ordering::Release);
                }
            }),
        )
        .map_err(|_| GatewayError("gateway request preparation"))?;
    let completion = runtime.block_on(async {
        tokio::pin!(prepared);
        loop {
            tokio::select! {
                result = &mut prepared => break Some(result),
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if stop.load(Ordering::Acquire)
                        || cancelled.load(Ordering::Acquire)
                        || socket_closed(&monitor)
                    {
                        break None;
                    }
                }
            }
        }
    });
    let Some(completion) = completion else {
        return Ok(());
    };
    if let Err(failure) = completion.outcome.result {
        let mut stream = output.lock().expect("gateway stream");
        let body =
            json!({"error":{"message":format!("provider request failed: {:?}", failure.kind)}});
        let _ = write_sse(&mut stream, &body);
    }
    Ok(())
}

fn socket_closed(stream: &TcpStream) -> bool {
    let mut byte = [0_u8; 1];
    match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            false
        }
        Err(_) => true,
    }
}

fn write_event(stream: &mut TcpStream, event: SamplingEvent) -> std::io::Result<()> {
    match event.kind {
        SamplingEventKind::Delta(delta) => match delta {
            SamplingDelta::Text(text) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"role":"assistant","content":text.get()},"finish_reason":null}]
                }),
            ),
            SamplingDelta::ToolIdentity { index, id, name } => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"id":id.as_ref().map(|v|v.as_str()),"type":"function","function":{"name":name.as_ref().map(|v|v.as_str())}}]},"finish_reason":null}]
                }),
            ),
            SamplingDelta::ToolArguments { index, fragment } => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"function":{"arguments":fragment.get()}}]},"finish_reason":null}]
                }),
            ),
            SamplingDelta::Finish(reason) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{},"finish_reason":finish_name(reason)}]
                }),
            ),
            SamplingDelta::Usage(usage) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[],
                    "usage":usage_json(usage)
                }),
            ),
        },
        SamplingEventKind::Terminal { error: None } => stream.write_all(b"data: [DONE]\n\n"),
        SamplingEventKind::Terminal { error: Some(error) } => write_sse(
            stream,
            &json!({"error":{"message":format!("provider request failed: {error:?}")}}),
        ),
    }
}

fn finish_name(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Complete => "stop",
        FinishReason::OutputLimit => "length",
        FinishReason::ToolCalls => "tool_calls",
    }
}

fn usage_json(usage: velune_ai::observation::Usage) -> Value {
    let mut value = serde_json::Map::new();
    if let Quantity::Reported(input) = usage.input_tokens {
        value.insert("prompt_tokens".into(), json!(input));
    }
    if let Quantity::Reported(output) = usage.output_tokens {
        value.insert("completion_tokens".into(), json!(output));
    }
    Value::Object(value)
}

fn write_sse(stream: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    write!(stream, "data: {body}\n\n")?;
    stream.flush()
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, Box<dyn std::error::Error>> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while head.len() < MAX_HEADERS_BYTES {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let split = head
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(GatewayError("gateway headers"))?;
    let header_text =
        std::str::from_utf8(&head[..split]).map_err(|_| GatewayError("gateway headers"))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(GatewayError("gateway request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(GatewayError("gateway method"))?
        .to_owned();
    let path = parts.next().ok_or(GatewayError("gateway path"))?.to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(GatewayError("gateway content length"))?;
    if length > MAX_REQUEST_BYTES {
        return Err(Box::new(GatewayError("gateway request too large")));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body)?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_json(stream: &mut TcpStream, status: &str, body: Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

fn sampling_input(
    body: &Value,
    max: u32,
    reasoning_levels: &[String],
) -> Result<SamplingInput, Box<dyn std::error::Error>> {
    if body["stream"].as_bool() != Some(true) {
        return Err(Box::new(GatewayError("gateway requires streaming")));
    }
    if body["store"].as_bool() == Some(true) {
        return Err(Box::new(GatewayError("gateway store is unsupported")));
    }
    let requested = body["max_completion_tokens"]
        .as_u64()
        .or_else(|| body["max_tokens"].as_u64())
        .ok_or(GatewayError("gateway output limit"))?;
    let requested = u32::try_from(requested).map_err(|_| GatewayError("gateway output limit"))?;
    if requested == 0 || requested > max {
        return Err(Box::new(GatewayError("gateway output limit")));
    }
    let mut instructions = None;
    let mut messages = Vec::new();
    for message in body["messages"]
        .as_array()
        .ok_or(GatewayError("gateway messages"))?
    {
        let role = message["role"]
            .as_str()
            .ok_or(GatewayError("gateway message role"))?;
        match role {
            "system" | "developer" => {
                instructions = Some(Payload::new(message_text(
                    &message["content"],
                    "gateway instructions",
                )?));
            }
            "user" => messages.push(Message::User(Payload::new(message_text(
                &message["content"],
                "gateway user content",
            )?))),
            "assistant" => {
                let text = if message["content"].is_null() {
                    None
                } else {
                    Some(Payload::new(message_text(
                        &message["content"],
                        "gateway assistant content",
                    )?))
                };
                let mut calls = Vec::new();
                if let Some(tool_values) = message["tool_calls"].as_array() {
                    for tool in tool_values {
                        let id = velune_ai::ids::ToolCallId::new(
                            tool["id"]
                                .as_str()
                                .ok_or(GatewayError("gateway tool id"))?
                                .to_owned(),
                        )
                        .map_err(|_| GatewayError("gateway tool id"))?;
                        let name = velune_ai::ids::ToolName::new(
                            tool.pointer("/function/name")
                                .and_then(Value::as_str)
                                .ok_or(GatewayError("gateway tool name"))?
                                .to_owned(),
                        )
                        .map_err(|_| GatewayError("gateway tool name"))?;
                        let arguments = tool
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .ok_or(GatewayError("gateway tool arguments"))?;
                        let arguments: Value = serde_json::from_str(arguments)
                            .map_err(|_| GatewayError("gateway tool arguments"))?;
                        calls.push(ToolCall {
                            id,
                            name,
                            arguments: Payload::new(arguments),
                        });
                    }
                }
                messages.push(Message::Assistant {
                    text,
                    tool_calls: calls,
                });
            }
            "tool" => messages.push(Message::ToolResult {
                call: velune_ai::ids::ToolCallId::new(
                    message["tool_call_id"]
                        .as_str()
                        .ok_or(GatewayError("gateway tool result id"))?
                        .to_owned(),
                )
                .map_err(|_| GatewayError("gateway tool result id"))?,
                content: Payload::new(message_text(&message["content"], "gateway tool result")?),
                is_error: false,
            }),
            _ => return Err(Box::new(GatewayError("gateway message role"))),
        }
    }
    if messages.is_empty() {
        return Err(Box::new(GatewayError("gateway messages")));
    }
    let mut input = SamplingInput::new(
        instructions,
        messages,
        parse_tools(&body["tools"])?,
        requested,
    )?;
    if let Some(reasoning) = body["reasoning_effort"].as_str() {
        if reasoning.len() > MAX_REASONING_LEVEL_BYTES {
            return Err(Box::new(GatewayError("gateway reasoning level")));
        }
        if !reasoning_levels.iter().any(|level| level == reasoning) {
            return Err(Box::new(GatewayError("gateway reasoning level")));
        }
        input = input
            .with_reasoning_level(Some(reasoning.to_owned()))
            .map_err(|_| GatewayError("gateway reasoning level"))?;
    }
    Ok(input)
}

fn message_text(value: &Value, error: &'static str) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let Some(parts) = value.as_array() else {
        return Err(Box::new(GatewayError(error)));
    };
    let mut output = String::new();
    for part in parts {
        if part["type"] != "text" {
            return Err(Box::new(GatewayError(
                "gateway content part is unsupported",
            )));
        }
        output.push_str(
            part["text"]
                .as_str()
                .ok_or(GatewayError("gateway text part"))?,
        );
    }
    Ok(output)
}

fn parse_tools(value: &Value) -> Result<Vec<ToolDefinition>, Box<dyn std::error::Error>> {
    let Some(tools) = value.as_array() else {
        return Ok(Vec::new());
    };
    tools
        .iter()
        .map(|tool| {
            let function = &tool["function"];
            let name = velune_ai::ids::ToolName::new(
                function["name"]
                    .as_str()
                    .ok_or(GatewayError("gateway tool name"))?
                    .to_owned(),
            )
            .map_err(|_| GatewayError("gateway tool name"))?;
            ToolDefinition::new(
                name,
                Payload::new(
                    function["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ),
                function["parameters"].clone(),
            )
            .map_err(|_| GatewayError("gateway tool schema").into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufReader, Read},
        sync::{Mutex, mpsc},
        time::Instant,
    };

    static GATEWAY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn config(endpoint: String) -> GatewayConfig {
        GatewayConfig {
            id: "fixture-gateway".into(),
            name: "Fixture gateway".into(),
            models: vec![crate::gateway::ModelDefinition {
                id: "model".into(),
                nickname: "Fixture model".into(),
                icon: None,
                max_output_tokens: 128,
                context_window: Some(8192),
                reasoning_levels: vec!["standard".into()],
            }],
            providers: vec![crate::gateway::ProviderDefinition {
                id: "provider".into(),
                name: "Fixture provider".into(),
                protocol: GatewayProtocol::ChatCompletionsV1,
                endpoint,
                credential_ref: Some("fixture".into()),
                models: vec![crate::gateway::ProviderModelBinding {
                    model_id: "model".into(),
                    external_model_id: "external-model".into(),
                }],
            }],
            routes: vec![crate::gateway::Route {
                model_id: "model".into(),
                provider_id: "provider".into(),
            }],
            failover: crate::gateway::FailoverPolicy {
                mode: crate::gateway::FailoverMode::Disabled,
            },
        }
    }

    fn consume_http_request(stream: &mut TcpStream) {
        let mut header = Vec::new();
        let mut byte = [0_u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).expect("upstream headers");
            header.push(byte[0]);
        }
        let header_text = String::from_utf8(header).expect("upstream header text");
        let length = header_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            })
            .and_then(|value| value.parse::<usize>().ok())
            .expect("upstream content length");
        let mut body = vec![0_u8; length];
        stream.read_exact(&mut body).expect("upstream body");
    }

    #[test]
    fn chat_completions_gateway_round_trips_tool_arguments_as_json_string() {
        let _lock = GATEWAY_TEST_LOCK.lock().expect("gateway test lock");
        let upstream = TcpListener::bind("127.0.0.1:0").expect("upstream listener");
        let address = upstream.local_addr().expect("upstream address");
        let upstream_thread = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().expect("upstream connection");
            let mut header = Vec::new();
            let mut byte = [0_u8; 1];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).expect("upstream headers");
                header.push(byte[0]);
            }
            let header_text = String::from_utf8(header).expect("upstream header text");
            let length = header_text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .and_then(|value| value.parse::<usize>().ok())
                .expect("upstream content length");
            let mut body = vec![0_u8; length];
            stream.read_exact(&mut body).expect("upstream body");
            let body: Value = serde_json::from_slice(&body).expect("upstream JSON");
            assert_eq!(body["model"], "external-model");
            assert_eq!(body["max_completion_tokens"], 64);
            assert_eq!(body["reasoning_effort"], "standard");
            assert!(body["messages"][1]["tool_calls"][0]["function"]["arguments"].is_string());
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .expect("upstream response headers");
            stream
                .write_all(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"fixture-ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n")
                .expect("upstream stream");
        });
        let mut gateway_config = config(format!("http://{}/v1", address));
        gateway_config.models.push(crate::gateway::ModelDefinition {
            id: "draft-only".into(),
            nickname: "Draft only".into(),
            icon: None,
            max_output_tokens: 64,
            context_window: Some(8192),
            reasoning_levels: Vec::new(),
        });
        let runner = Runner::start(gateway_config, None).expect("gateway startup");
        let url = reqwest::Url::parse(runner.endpoint()).expect("gateway URL");
        let mut invalid = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway invalid connection");
        write!(
            invalid,
            "POST /v1/chat/completions HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            runner.token()
        )
        .expect("gateway invalid request");
        let mut invalid_response = String::new();
        BufReader::new(invalid)
            .read_to_string(&mut invalid_response)
            .expect("gateway invalid response");
        assert!(invalid_response.starts_with("HTTP/1.1 400 Bad Request"));
        let mut client = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway connection");
        let body = json!({
            "model":"model",
            "messages":[
                {"role":"user","content":[{"type":"text","text":"use tool"}]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"add","arguments":"{\"a\":1}"}}]},
                {"role":"tool","tool_call_id":"call-1","content":[{"type":"text","text":"2"}]}
            ],
            "tools":[{"type":"function","function":{"name":"add","description":"add","parameters":{"type":"object","properties":{"a":{"type":"integer"}}}}}],
            "max_completion_tokens":64,
            "reasoning_effort":"standard",
            "stream":true
        });
        let body = serde_json::to_vec(&body).expect("gateway request body");
        write!(
            client,
            "POST /v1/chat/completions HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            runner.token(),
            body.len()
        )
        .expect("gateway request headers");
        client.write_all(&body).expect("gateway request");
        let mut response = String::new();
        BufReader::new(client)
            .read_to_string(&mut response)
            .expect("gateway response");
        assert!(response.contains("fixture-ok"));
        assert!(response.contains("data: [DONE]"));
        upstream_thread.join().expect("upstream thread");
    }

    #[test]
    fn gateway_drops_upstream_when_client_disconnects_before_first_chunk() {
        let _lock = GATEWAY_TEST_LOCK.lock().expect("gateway test lock");
        let upstream = TcpListener::bind("127.0.0.1:0").expect("upstream listener");
        let address = upstream.local_addr().expect("upstream address");
        let (ready_tx, ready_rx) = mpsc::channel();
        let upstream_thread = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().expect("upstream connection");
            let mut header = Vec::new();
            let mut byte = [0_u8; 1];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).expect("upstream headers");
                header.push(byte[0]);
            }
            let header_text = String::from_utf8(header).expect("upstream header text");
            let length = header_text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .and_then(|value| value.parse::<usize>().ok())
                .expect("upstream content length");
            let mut body = vec![0_u8; length];
            stream.read_exact(&mut body).expect("upstream body");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .expect("upstream response headers");
            stream.flush().expect("upstream response flush");
            ready_tx.send(()).expect("upstream ready");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("upstream timeout");
            let mut probe = [0_u8; 1];
            loop {
                match stream.read(&mut probe) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(_) => break,
                }
            }
        });
        let runner =
            Runner::start(config(format!("http://{}/v1", address)), None).expect("gateway startup");
        let url = reqwest::Url::parse(runner.endpoint()).expect("gateway URL");
        let mut client = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway connection");
        let body = serde_json::to_vec(&json!({
            "model":"model",
            "messages":[{"role":"user","content":"wait"}],
            "max_completion_tokens":64,
            "stream":true
        }))
        .expect("gateway body");
        write!(
            client,
            "POST /v1/chat/completions HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            runner.token(),
            body.len()
        )
        .expect("gateway request headers");
        client.write_all(&body).expect("gateway request");
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("upstream received request");
        let started = Instant::now();
        let mut headers = Vec::new();
        let mut byte = [0_u8; 1];
        while !headers.ends_with(b"\r\n\r\n") {
            client
                .read_exact(&mut byte)
                .expect("gateway response headers");
            headers.push(byte[0]);
        }
        drop(client);
        let mut probe = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway accepts another request after cancellation");
        probe
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("probe timeout");
        probe
            .write_all(b"GET /health HTTP/1.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .expect("probe request");
        let mut probe_response = String::new();
        BufReader::new(probe)
            .read_to_string(&mut probe_response)
            .expect("probe response");
        assert!(
            probe_response.starts_with("HTTP/1.1 404 Not Found"),
            "probe response: {probe_response:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(runner);
        upstream_thread.join().expect("upstream thread");
    }

    #[test]
    fn dropping_runner_stops_an_active_upstream_request() {
        let _lock = GATEWAY_TEST_LOCK.lock().expect("gateway test lock");
        let upstream = TcpListener::bind("127.0.0.1:0").expect("upstream listener");
        let address = upstream.local_addr().expect("upstream address");
        let (ready_tx, ready_rx) = mpsc::channel();
        let upstream_thread = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().expect("upstream connection");
            ready_tx.send(()).expect("upstream ready");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("upstream timeout");
            let mut probe = [0_u8; 1];
            loop {
                match stream.read(&mut probe) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(_) => break,
                }
            }
        });
        let runner =
            Runner::start(config(format!("http://{address}/v1")), None).expect("gateway startup");
        let url = reqwest::Url::parse(runner.endpoint()).expect("gateway URL");
        let mut client = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway connection");
        let body = serde_json::to_vec(&json!({
            "model":"model",
            "messages":[{"role":"user","content":"wait"}],
            "max_completion_tokens":64,
            "stream":true
        }))
        .expect("gateway body");
        write!(
            client,
            "POST /v1/chat/completions HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            runner.token(),
            body.len()
        )
        .expect("gateway request headers");
        client.write_all(&body).expect("gateway request");
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("upstream received request");
        let mut headers = Vec::new();
        let mut byte = [0_u8; 1];
        while !headers.ends_with(b"\r\n\r\n") {
            client
                .read_exact(&mut byte)
                .expect("gateway response headers");
            headers.push(byte[0]);
        }
        let started = Instant::now();
        drop(runner);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(client);
        upstream_thread.join().expect("upstream thread");
    }

    #[test]
    fn provider_failure_stream_has_error_without_done_marker() {
        let _lock = GATEWAY_TEST_LOCK.lock().expect("gateway test lock");
        let upstream = TcpListener::bind("127.0.0.1:0").expect("upstream listener");
        let address = upstream.local_addr().expect("upstream address");
        let upstream_thread = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().expect("upstream connection");
            consume_http_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{{\"error\":\"fixture\"}}"
            )
            .expect("upstream failure");
            stream.flush().expect("upstream failure flush");
        });
        let runner =
            Runner::start(config(format!("http://{address}/v1")), None).expect("gateway startup");
        let url = reqwest::Url::parse(runner.endpoint()).expect("gateway URL");
        let mut client = TcpStream::connect(("127.0.0.1", url.port().expect("gateway port")))
            .expect("gateway connection");
        let body = serde_json::to_vec(&json!({
            "model":"model",
            "messages":[{"role":"user","content":"failure"}],
            "max_completion_tokens":64,
            "stream":true
        }))
        .expect("gateway body");
        write!(
            client,
            "POST /v1/chat/completions HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            runner.token(),
            body.len()
        )
        .expect("gateway request headers");
        client.write_all(&body).expect("gateway request");
        let mut response = String::new();
        BufReader::new(client)
            .read_to_string(&mut response)
            .expect("gateway response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("provider request failed"));
        assert!(!response.contains("data: [DONE]"));
        upstream_thread.join().expect("upstream thread");
    }
}
