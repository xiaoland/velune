//! The collaboration service's MCP ingress. Tool calls wait in the inbox for
//! the application; the service thread never touches it.
use super::{ConversationRef, Envelope, Inbox, Operation};
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
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{net::TcpListener, sync::Arc, thread::JoinHandle, time::Duration};

pub(super) struct Server {
    endpoint: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(inbox: Arc<Inbox>) -> Result<Self, Error> {
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
                    { let inbox = inbox.clone(); move || Ok(Handler { inbox: inbox.clone(), wait_slots: wait_slots.clone() }) },
                    Arc::new(NeverSessionManager::default()),
                    config,
                );
                let router = Router::new().nest_service("/mcp", service).layer(
                    middleware::from_fn_with_state(inbox, authenticate),
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
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
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

/// The calling session, from the ticket that authorized the request.
#[derive(Clone)]
struct Caller(String);

async fn authenticate(
    State(inbox): State<Arc<Inbox>>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let ticket = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|ticket| inbox.admits(ticket))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_owned();
    request.extensions_mut().insert(Caller(ticket));
    Ok(next.run(request).await)
}

struct Handler {
    inbox: Arc<Inbox>,
    wait_slots: Arc<tokio::sync::Semaphore>,
}

/// How long a call may wait for the application beyond its own wait: the
/// application runs tool calls when a client next advances the sessions.
const APPLICATION_GRACE: Duration = Duration::from_secs(60);

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("velune", env!("CARGO_PKG_VERSION")))
            .with_instructions("Velune 让你与本机其他 Agent 会话交流，可跨不同 Harness。list_agents 列出可用运行时，list_conversations 查找已有会话；start_conversation 新建会话并发送首条消息，send_message 给空闲会话发消息；随后用 wait_conversation 等待对方回复。xurl_instructions 提供官方 xurl CLI 用法与实例环境；resolve_agent_uri 查询 agents:// 地址，read_conversation 分页读取文字历史。conversation_id 也接受具体 agents:// 地址，歧义时指定 runtime_id。对方都是普通会话，用户可以看到并继续。对方的回复是它自己的报告，不代表已验证；对方等待审批时由用户处理。")
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(Caller(ticket)) = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Caller>())
            .cloned()
        else {
            return Ok(CallToolResult::structured_error(json!({"error":"调用方未授权"})).into());
        };
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let operation = match parse(&request.name, arguments) {
            Ok(operation) => operation,
            Err(error) => {
                return Ok(
                    CallToolResult::structured_error(json!({"error":error.to_string()})).into(),
                );
            }
        };
        // Bound blocking waits without starving other calls behind them.
        let (permit, waited) = match &operation {
            Operation::Wait(wait) => match self.wait_slots.clone().try_acquire_owned() {
                Ok(permit) => (Some(permit), Duration::from_millis(wait.timeout_ms)),
                Err(_) => {
                    return Ok(CallToolResult::structured_error(
                        json!({"error":"并行等待过多，请先处理现有等待结果"}),
                    )
                    .into());
                }
            },
            _ => (None, Duration::ZERO),
        };
        let (reply, answer) = std::sync::mpsc::channel();
        if let Err(error) = self.inbox.post(Envelope {
            ticket,
            operation,
            reply,
        }) {
            return Ok(CallToolResult::structured_error(json!({"error":error.to_string()})).into());
        }
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            answer.recv_timeout(waited + APPLICATION_GRACE)
        })
        .await;
        let response = match result {
            Ok(Ok(Ok(value))) => CallToolResult::structured(value),
            Ok(Ok(Err(error))) => {
                CallToolResult::structured_error(json!({"error":error.to_string()}))
            }
            Ok(Err(std::sync::mpsc::RecvTimeoutError::Timeout)) => {
                CallToolResult::structured_error(
                    json!({"error":"Velune 未在期限内处理此请求；结果未知，不要假定已发送、已取消或已重发"}),
                )
            }
            Ok(Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) | Err(_) => {
                CallToolResult::structured_error(
                    json!({"error":"Velune 已关闭或放弃此请求；结果未知"}),
                )
            }
        };
        Ok(response.into())
    }
}

fn parse(name: &str, args: Value) -> Result<Operation, Error> {
    fn decode<T: DeserializeOwned>(args: Value) -> Result<T, Error> {
        Ok(serde_json::from_value(args)?)
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    Ok(match name {
        "scheduled_tasks" => {
            let operation: crate::ScheduledTasksOperation = decode(args)?;
            if matches!(
                operation,
                crate::ScheduledTasksOperation::ConfirmHandover { .. }
                    | crate::ScheduledTasksOperation::Import { .. }
                    | crate::ScheduledTasksOperation::PreviewImport { .. }
            ) {
                return Err(Error::invalid("来源导入和宿主交接由用户在定时任务窗口处理"));
            }
            Operation::Scheduled(Box::new(operation))
        }
        "list_agents" => {
            let _: Empty = decode(args)?;
            Operation::ListAgents
        }
        "xurl_instructions" => {
            let _: Empty = decode(args)?;
            Operation::XurlInstructions
        }
        "resolve_agent_uri" => Operation::ResolveUri(decode(args)?),
        "read_conversation" => Operation::Read(decode(args)?),
        "list_conversations" => Operation::ListConversations(decode(args)?),
        "start_conversation" => Operation::Start(decode(args)?),
        "send_message" => Operation::Send(decode(args)?),
        "get_conversation" => Operation::Get(decode::<ConversationRef>(args)?),
        "wait_conversation" => Operation::Wait(decode(args)?),
        "cancel_turn" => Operation::Cancel(decode::<ConversationRef>(args)?),
        _ => return Err(Error::invalid("未知协作工具")),
    })
}

fn tools() -> Vec<Tool> {
    let text = json!({"type":"string","minLength":1});
    let conversation = json!({"conversation_id":text,"runtime_id":text});
    [
        ("scheduled_tasks", "管理 Velune 执行设备的本地定时任务。operation=list 返回完整定义和 revision；save 的 parameters.task 传完整定义（新建 id 为空、revision=0；更新必须保留当前 revision）。支持 delete、runNow（parameters 为 id/revision）、setEnabled（另加 enabled 布尔值，暂停无需修复配置）、previewSchedule（parameters 为 rrule/timeZone 和可选 pattern，仅预览）。规则为 RFC5545 RRULE、timeZone 为 IANA 时区，状态 active/paused；新建 task 必填 name、prompt、rrule、timeZone、status、target，其他项可省略。target={kind:newConversation,runtimeId,workingDirectories:[绝对路径],worktree:false}，或 {kind:existingConversation,runtimeId,conversationId}；model 可省略为运行时默认，也可用 list_agents 给出的模型身份。任务随 Velune 启停，退出错过的触发不补跑。原宿主交接只能由用户在窗口确认。", json!({"operation":{"type":"string","enum":["list","save","delete","runNow","setEnabled","previewSchedule"]},"parameters":{"type":"object"}}), vec!["operation"]),
        ("xurl_instructions", "取得 Xuanwo/xurl 官方 CLI 的查询、读取、发现与独立续接指引；先读指引，再用 list_agents 的可执行文件和环境。", json!({}), vec![]),
        ("resolve_agent_uri", "在当前设备已启用实例中定位 agents://codex、claude、pi 会话或查询 collection。q 按标题搜索，limit 为 1–100。返回 conversations 与 failures；多匹配需选 runtime_id，不要猜。路径、角色和全文搜索使用官方 CLI。", json!({"uri":text,"runtime_id":text}), vec!["uri"]),
        ("read_conversation", "分页读取会话文字历史（正文、思考、工具记录，不含图片字节）。conversation_id 接受 Velune ID 或具体 agents:// 地址。offset/limit 按 Unicode 字符计数，后续页传 expected_revision 避免历史变化导致错页。", json!({"conversation_id":text,"runtime_id":text,"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":64000},"expected_revision":text}), vec!["conversation_id"]),
        ("list_agents", "列出本机 Velune 中已启用的 Agent 运行时（可能是不同的 Harness），包括运行位置。", json!({}), vec![]),
        ("list_conversations", "列出已有会话（不含你自己的），按最近更新排序，含状态。可按工作目录精确过滤或按标题搜索。", json!({"cwd":text,"query":text,"limit":{"type":"integer","minimum":1,"maximum":100}}), vec![]),
        ("start_conversation", "在指定运行时新建一个普通会话并发送首条消息，立即返回；用户能在会话列表看到它。cwd 缺省时沿用你的工作目录（同一台机器时），model 缺省为该运行时默认模型。用 wait_conversation 取得回复。", json!({"runtime_id":text,"message":text,"cwd":text,"model":text}), vec!["runtime_id","message"]),
        ("send_message", "向一个空闲会话发送消息，作为它的下一轮，立即返回。对方正在执行、等待审批或在 Velune 之外运行时拒绝，不排队。", json!({"conversation_id":text,"runtime_id":text,"message":text}), vec!["conversation_id","message"]),
        ("get_conversation", "读取会话状态与最近一轮的回复文本。state 为 running、waiting_for_approval（由用户处理）、idle、failed 或 running_elsewhere。", conversation.clone(), vec!["conversation_id"]),
        ("wait_conversation", "等待会话进入你尚未读到的非运行状态（回复完成、失败或等待审批），最长 30000ms。超时（timed_out）只结束本次等待，不会取消或重发。", json!({"conversation_id":text,"runtime_id":text,"timeout_ms":{"type":"integer","minimum":0,"maximum":30000}}), vec!["conversation_id","timeout_ms"]),
        ("cancel_turn", "取消会话当前正在执行的一轮；不删除会话，也不自动重发。", conversation, vec!["conversation_id"]),
    ].into_iter().map(|(name, description, properties, required)| {
        Tool::new(name, description, json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}).as_object().expect("static tool schema").clone())
    }).collect()
}
