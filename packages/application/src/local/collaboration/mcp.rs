//! Session-scoped MCP ingress. Tools never acquire the foreground Application lock.
use super::Coordinator;
use crate::Error;
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
    },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{net::TcpListener, sync::Arc, thread::JoinHandle, time::Duration};

pub(super) struct Server {
    endpoint: String,
    token: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(coordinator: Arc<Coordinator>) -> Result<Self, Error> {
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| Error::invalid("无法生成协作会话票据"))?;
        let token = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}/mcp", listener.local_addr()?);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let listener = {
            let _entered = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let authorization = Arc::new(format!("Bearer {token}"));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new().name("velune-collaboration-mcp".into()).spawn(move || {
            runtime.block_on(async move {
                let config = StreamableHttpServerConfig::default()
                    .with_legacy_session_mode(false)
                    .with_json_response(true)
                    .with_max_request_body_bytes(256 * 1024)
                    .enforce_origin_validation();
                let cancel = config.cancellation_token.clone();
                let wait_slots = Arc::new(tokio::sync::Semaphore::new(16));
                let service = StreamableHttpService::new(
                    move || Ok(Handler { coordinator: coordinator.clone(), wait_slots: wait_slots.clone() }),
                    Arc::new(NeverSessionManager::default()),
                    config,
                );
                let router = Router::new().nest_service("/mcp", service).layer(
                    middleware::from_fn_with_state(authorization, authenticate),
                );
                let server = axum::serve(listener, router);
                tokio::select! {
                    result = server => { if result.is_err() { tracing::warn!("collaboration MCP listener stopped"); } },
                    _ = stopped => {},
                }
                cancel.cancel();
            });
            // A cancelled HTTP wait must not hold shutdown until its blocking worker returns.
            runtime.shutdown_timeout(Duration::from_secs(1));
        })?;
        Ok(Self {
            endpoint,
            token,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(super) fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn authenticate(
    State(expected): State<Arc<String>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = request
        .headers()
        .get("authorization")
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    let expected = expected.as_bytes();
    let mismatch = provided
        .iter()
        .zip(expected)
        .fold(0u8, |diff, (left, right)| diff | (left ^ right));
    if provided.len() != expected.len() || mismatch != 0 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

struct Handler {
    coordinator: Arc<Coordinator>,
    wait_slots: Arc<tokio::sync::Semaphore>,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("velune-collaboration", env!("CARGO_PKG_VERSION")))
            .with_instructions("这些工具在宿主已授权的执行环境中委派子任务。先用 agents 取得 target_id；spawn 返回子任务句柄后，用 status/wait 取得结果。结果是子 Agent 报告，不代表验收通过。待审批必须由用户处理，不得通过后续 prompt 绕过。")
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let operation = match Operation::parse(&request.name, arguments) {
            Ok(operation) => operation,
            Err(error) => {
                return Ok(
                    CallToolResult::structured_error(json!({"error":error.to_string()})).into(),
                );
            }
        };
        // Bound blocking waits without starving cancel/status requests behind them.
        let permit = if matches!(&operation, Operation::Wait(_)) {
            match self.wait_slots.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    return Ok(CallToolResult::structured_error(
                        json!({"error":"并行等待过多，请先处理现有等待结果"}),
                    )
                    .into());
                }
            }
        } else {
            None
        };
        let coordinator = self.coordinator.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation.run(&coordinator)
        })
        .await;
        let response = match result {
            Ok(Ok(value)) => CallToolResult::structured(value),
            Ok(Err(error)) => CallToolResult::structured_error(json!({"error":error.to_string()})),
            Err(_) => CallToolResult::structured_error(
                json!({"error":"协作请求执行失败；不要假定子任务已取消或自动重发任务"}),
            ),
        };
        Ok(response.into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spawn {
    target_id: String,
    prompt: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    child_id: String,
    prompt: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Child {
    child_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wait {
    child_id: String,
    after_revision: u64,
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

enum Operation {
    Agents,
    Spawn(Spawn),
    Send(Send),
    Status(Child),
    Wait(Wait),
    Cancel(Child),
    Close(Child),
}

impl Operation {
    fn parse(name: &str, args: Value) -> Result<Self, Error> {
        fn decode<T: DeserializeOwned>(args: Value) -> Result<T, Error> {
            Ok(serde_json::from_value(args)?)
        }
        Ok(match name {
            "agents" => {
                let _: Empty = decode(args)?;
                Self::Agents
            }
            "spawn" => Self::Spawn(decode(args)?),
            "send" => Self::Send(decode(args)?),
            "status" => Self::Status(decode(args)?),
            "wait" => {
                let args: Wait = decode(args)?;
                if args.timeout_ms > 30_000 {
                    return Err(Error::invalid("wait timeout_ms 必须在 0 到 30000 之间"));
                }
                Self::Wait(args)
            }
            "cancel" => Self::Cancel(decode(args)?),
            "close" => Self::Close(decode(args)?),
            _ => return Err(Error::invalid("未知协作工具")),
        })
    }

    fn run(self, coordinator: &Arc<Coordinator>) -> Result<Value, Error> {
        fn value(result: impl Serialize) -> Result<Value, Error> {
            Ok(serde_json::to_value(result)?)
        }
        match self {
            Self::Agents => Ok(json!({"targets":coordinator.agents()})),
            Self::Spawn(args) => value(coordinator.spawn(&args.target_id, &args.prompt)?),
            Self::Send(args) => value(coordinator.send(&args.child_id, &args.prompt)?),
            Self::Status(args) => value(coordinator.status(&args.child_id)?),
            Self::Wait(args) => {
                value(coordinator.wait(&args.child_id, args.after_revision, args.timeout_ms)?)
            }
            Self::Cancel(args) => value(coordinator.cancel(&args.child_id)?),
            Self::Close(args) => value(coordinator.close(&args.child_id)?),
        }
    }
}

fn tools() -> Vec<Tool> {
    let text = json!({"type":"string","minLength":1});
    let child = json!({"child_id":text});
    [
        ("agents", "列出宿主为本父会话授权的子 Agent 执行目标。不能自行指定额外程序、工作目录或模型。", json!({}), vec![]),
        ("spawn", "向一个授权目标委派任务，立即返回子任务句柄。任务异步执行；返回 starting/running 不代表完成。不同子任务可并行。", json!({"target_id":text,"prompt":text}), vec!["target_id","prompt"]),
        ("send", "给现有空闲子 Agent 发送后续任务，保留其原生会话上下文。忙碌时拒绝，不排队或强行插话。", json!({"child_id":text,"prompt":text}), vec!["child_id","prompt"]),
        ("status", "读取子任务状态、当前轮次和最终报告。waitingForApproval 需要用户处理；失败、取消或关闭不能当成成功。", child.clone(), vec!["child_id"]),
        ("wait", "等待子任务 revision 变化，最长 30000ms。使用已读 revision；超时仅返回当前状态，不会取消或重发。", json!({"child_id":text,"after_revision":{"type":"integer","minimum":0},"timeout_ms":{"type":"integer","minimum":0,"maximum":30000}}), vec!["child_id","after_revision","timeout_ms"]),
        ("cancel", "取消指定子 Agent 的当前 turn；不自动重新执行，也不删除会话历史。", child.clone(), vec!["child_id"]),
        ("close", "关闭指定子 Agent 并清理其执行进程；活动任务会中断，原生历史仍归该 Harness。", child, vec!["child_id"]),
    ].into_iter().map(|(name, description, properties, required)| {
        Tool::new(name, description, json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}).as_object().expect("static tool schema").clone())
    }).collect()
}
