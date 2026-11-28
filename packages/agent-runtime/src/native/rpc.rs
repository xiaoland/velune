//! Bounded stdio JSON-RPC transport. Native protocols remain in their adapters.
use super::{Error, Result};
use crate::creation::{ReadAccount, ReadLease, ReadRecord, Retained};
use crate::{
    process_tree::ProcessTree,
    transport::{NotAdmitted, OwnedFrame, WriteClass, WriteReceipt},
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::{
    io::{BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const PENDING_WRITES: usize = 16;
pub(crate) const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(serde::Serialize)]
struct RequestFrame<'a> {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    method: &'a str,
    params: &'a Value,
}
#[derive(serde::Serialize)]
struct ResponseFrame<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    result: &'a Value,
}
#[derive(serde::Serialize)]
struct RejectionFrame<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    error: Rejection<'a>,
}
#[derive(serde::Serialize)]
struct Rejection<'a> {
    code: i32,
    message: &'a str,
}

pub(crate) struct RpcClient {
    /// None once handed to a thread that lets it end; see `retire`.
    child: Option<ProcessTree>,
    input: Option<crate::transport::PipeWriter>,
    writes: Vec<(&'static str, crate::transport::WriteReceipt)>,
    records: Receiver<Result<ReadRecord>>,
    read_account: Arc<ReadAccount>,
    // Permits follow actual queued/deferred/delivered DOMs, not just raw lines.
    deferred_permits: VecDeque<ReadLease>,
    queued: crate::transport::DeferredRecords,
    next: u64,
    failed: Option<Error>,
    /// Live command owners perform process cleanup explicitly, outside poll.
    defer_cleanup: bool,
    stderr: Arc<Mutex<Vec<u8>>>,
}
/// Run a short-lived helper. `stderr_limit` only bounds retained diagnostics;
/// stdout is a business payload and is read to EOF without an adapter limit.
pub(crate) fn bounded_process(
    command: Command,
    input: Option<Vec<u8>>,
    stderr_limit: usize,
) -> Result<Vec<u8>> {
    bounded_process_with_budget(
        command,
        input,
        stderr_limit,
        &crate::probe::ProbeBudget::new(CONTROL_TIMEOUT),
    )
}

pub(crate) fn bounded_process_with_budget(
    mut command: Command,
    input: Option<Vec<u8>>,
    stderr_limit: usize,
    budget: &crate::probe::ProbeBudget,
) -> Result<Vec<u8>> {
    if budget.finished() {
        return Err(probe_budget_error(budget));
    }
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ProcessTree::spawn(&mut command)
        .map_err(|error| Error::with_code(format!("运行时辅助进程无法启动：{error}"), "spawn"))?;
    if let Some(bytes) = input {
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| Error::with_code("辅助进程输入不可用", "transport"))?;
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let mut stdout = child
        .take_stdout()
        .ok_or_else(|| Error::with_code("运行时输出不可用", "transport"))?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| Error::with_code("运行时错误输出不可用", "transport"))?;
    let (tx, rx) = mpsc::sync_channel(1);
    let (stderr_tx, stderr_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let outcome = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(outcome);
    });
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut reader = stderr;
        let mut chunk = [0_u8; 8192];
        let outcome = loop {
            match reader.read(&mut chunk) {
                Ok(0) => break Ok(bytes),
                Ok(size) => {
                    bytes.extend_from_slice(&chunk[..size]);
                    if bytes.len() > stderr_limit.saturating_add(1) {
                        bytes.truncate(stderr_limit.saturating_add(1));
                    }
                }
                Err(error) => break Err(error),
            }
        };
        let _ = stderr_tx.send(outcome);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                child.stop();
                return Err(Error::with_code(
                    format!("运行时辅助进程状态不可用：{error}"),
                    "transport",
                ));
            }
        }
        if budget.finished() {
            child.stop();
            if budget.cancelled() {
                return Err(probe_budget_error(budget));
            }
            let detail = stderr_rx
                .try_recv()
                .ok()
                .and_then(|result| result.ok())
                .map(|bytes| {
                    let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(stderr_limit)])
                        .trim()
                        .to_owned();
                    if bytes.len() > stderr_limit {
                        text.push_str("\n[stderr 已截断：诊断缓存达到上限]");
                    }
                    text
                })
                .unwrap_or_default();
            return Err(Error::with_code(
                if detail.is_empty() {
                    "运行时辅助进程超时".into()
                } else {
                    format!("运行时辅助进程超时：{detail}")
                },
                "timeout",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    };
    child.stop();
    let bytes =
        receive_probe_output(&rx, budget, "运行时辅助进程输出未关闭")?.map_err(|error| {
            Error::with_code(format!("运行时辅助进程读取失败：{error}"), "transport")
        })?;
    let stderr = receive_probe_output(&stderr_rx, budget, "运行时辅助进程错误输出未关闭")?
        .map_err(|error| {
            Error::with_code(
                format!("运行时辅助进程错误输出读取失败：{error}"),
                "transport",
            )
        })?;
    if !status.success() {
        let mut detail = String::from_utf8_lossy(&stderr[..stderr.len().min(stderr_limit)])
            .trim()
            .to_owned();
        if stderr.len() > stderr_limit {
            detail.push_str("\n[stderr 已截断：诊断缓存达到上限]");
        }
        return Err(Error::with_code(
            format!("运行时辅助进程失败（{status}）：{detail}"),
            "process_exit",
        ));
    }
    Ok(bytes)
}

fn receive_probe_output<T>(
    receiver: &Receiver<T>,
    budget: &crate::probe::ProbeBudget,
    phase: &str,
) -> Result<T> {
    let drain_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if budget.finished() {
            return Err(probe_budget_error(budget));
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < drain_deadline => {}
            Err(error) => {
                return Err(Error::with_code(format!("{phase}：{error}"), "transport"));
            }
        }
    }
}

fn probe_budget_error(budget: &crate::probe::ProbeBudget) -> Error {
    if budget.cancelled() {
        Error::with_code("运行时辅助进程已取消", "cancelled")
    } else {
        Error::with_code("运行时辅助进程超时", "timeout")
    }
}

impl RpcClient {
    pub(crate) fn spawn_at(
        command: Command,
        execution: &crate::execution::ExecutionTarget,
        os_home: Option<&std::path::Path>,
    ) -> Result<Self> {
        let prepared = execution.prepare(command, os_home)?;
        let mut command = prepared.command;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ProcessTree::spawn(&mut command)
            .map_err(|error| Error::new(format!("运行时进程无法启动：{error}")))?;
        let input = crate::transport::PipeWriter::new(
            child
                .take_stdin()
                .ok_or_else(|| Error::new("运行时输入不可用"))?,
        )
        .map_err(|error| Error::new(format!("运行时写入服务无法启动：{error}")))?;
        input
            .write(prepared.input_prefix, Instant::now() + CONTROL_TIMEOUT)
            .map_err(|error| Error::new(format!("运行时启动请求发送失败：{error}")))?;
        let output = child
            .take_stdout()
            .ok_or_else(|| Error::new("运行时输出不可用"))?;
        let stderr_output = child
            .take_stderr()
            .ok_or_else(|| Error::new("运行时错误输出不可用"))?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_copy = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut output = stderr_output;
            let mut chunk = [0_u8; 8192];
            loop {
                match output.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        if let Ok(mut target) = stderr_copy.lock()
                            && target.len() <= MAX_STDERR_BYTES
                        {
                            let remaining = MAX_STDERR_BYTES + 1 - target.len();
                            target.extend_from_slice(&chunk[..size.min(remaining)]);
                        }
                    }
                }
            }
        });
        let (tx, records) = mpsc::sync_channel(64);
        let read_account = ReadAccount::new();
        let reader_account = read_account.clone();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        thread::spawn(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let mut reader = BufReader::new(output);
                loop {
                    let result = match crate::creation::read_record(&mut reader, &reader_account) {
                        Ok(Some(record)) => Ok(record),
                        Ok(None) => Err(Error::with_code("运行时进程已退出", "transport")),
                        Err(error) => Err(Error::with_code(
                            crate::creation::detail(error),
                            "transport",
                        )),
                    };
                    let failed = result.is_err();
                    if tx.send(result).is_err() || failed {
                        break;
                    }
                }
            })
        });
        Ok(Self {
            child: Some(child),
            input: Some(input),
            writes: Vec::new(),
            records,
            read_account,
            deferred_permits: VecDeque::new(),
            queued: Default::default(),
            next: 1,
            failed: None,
            defer_cleanup: false,
            stderr,
        })
    }
    fn deliver(&mut self, record: ReadRecord) -> Retained<Value> {
        Retained::from_record(record.value, record.lease)
    }
    fn with_stderr(&self, error: Error) -> Error {
        let detail = self
            .stderr
            .lock()
            .ok()
            .map(|bytes| {
                let limit = MAX_STDERR_BYTES;
                let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(limit)])
                    .trim()
                    .to_owned();
                if bytes.len() > limit {
                    text.push_str("\n[stderr 已截断：诊断缓存达到上限]");
                }
                text
            })
            .unwrap_or_default();
        if detail.is_empty() {
            return error;
        }
        let code = error.code();
        Error::with_code(format!("{}；原始错误：{}", error, detail), code)
    }
    pub(crate) fn write(&mut self, value: &Value) -> Result<()> {
        self.write_until(value, Instant::now() + CONTROL_TIMEOUT)
    }
    pub(crate) fn write_with_class(&mut self, value: &Value, class: WriteClass) -> Result<()> {
        self.write_until_class(value, class, Instant::now() + CONTROL_TIMEOUT)
    }
    fn write_until(&mut self, value: &impl serde::Serialize, deadline: Instant) -> Result<()> {
        self.write_until_class(value, WriteClass::Ordinary, deadline)
    }
    fn write_until_class(
        &mut self,
        value: &impl serde::Serialize,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<()> {
        // Local rejection is provably not dispatched; do not retire a healthy
        // connection or misreport it as an unknown execution outcome.
        let receipt = self.enqueue_owned_frame(value, class, deadline)?;
        if let Err(error) = receipt.wait() {
            self.transport_failed();
            return Err(self.with_stderr(Error::with_code(
                format!("运行时请求发送未确认：{error}"),
                if error.kind() == std::io::ErrorKind::TimedOut {
                    "timeout"
                } else {
                    "transport"
                },
            )));
        }
        Ok(())
    }
    fn defer(&mut self, record: Retained<Value>) -> Result<()> {
        let (record, ownership) = record.into_parts();
        let mut permit = ownership
            .into_record()
            .map_err(|error| Error::with_code(error.to_string(), "transport"))?;
        permit
            .transfer_to_queue()
            .map_err(|error| Error::with_code(error.to_string(), "transport"))?;
        if let Err(error) = self.queued.push(record) {
            self.shutdown();
            return Err(Error::with_code(
                format!("运行时通知缓冲已满；连接已停止，未自动重发：{error}"),
                "transport",
            ));
        }
        self.deferred_permits.push_back(permit);
        Ok(())
    }
    pub(crate) fn claude_control(&mut self, request: Value) -> Result<Retained<Value>> {
        self.claude_control_with_class(request, WriteClass::Ordinary)
    }
    pub(crate) fn claude_control_with_class(
        &mut self,
        request: Value,
        class: WriteClass,
    ) -> Result<Retained<Value>> {
        let method = request["subtype"].as_str().unwrap_or("control_request");
        observe_request(method, || {
            let deadline = Instant::now() + CONTROL_TIMEOUT;
            let id = self.begin_claude_control(&request, class, deadline)?;
            loop {
                let record = self.wait_for_record(method, deadline)?;
                if record["type"] == "control_response" && record["response"]["request_id"] == id {
                    if record["response"]["subtype"] != "success" {
                        return Err(Error::new("Claude 拒绝控制请求"));
                    }
                    return Ok(record.map(|mut value| value["response"]["response"].take()));
                }
                self.defer(record)?;
            }
        })
    }
    /// Issue the control frame. The live session owner can process native
    /// output in order while it waits for the matching response.
    pub(crate) fn begin_claude_control(
        &mut self,
        request: &Value,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<String> {
        let id = format!("velune-{}", self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("运行时请求序号耗尽"))?;
        self.write_until_class(
            &json!({"type":"control_request","request_id":id,"request":request}),
            class,
            deadline,
        )?;
        Ok(id)
    }
    /// Admit a request without waiting for stdin or its protocol response.
    /// The caller owns response correlation and the same absolute deadline;
    /// poll reports terminal write failures, never replays the request.
    pub(crate) fn enqueue_request(
        &mut self,
        method: &'static str,
        params: &Value,
        deadline: Instant,
    ) -> Result<u64> {
        self.enqueue_request_with_class(method, params, WriteClass::Ordinary, deadline)
    }

    pub(crate) fn enqueue_request_with_class(
        &mut self,
        method: &'static str,
        params: &Value,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<u64> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("运行时请求序号耗尽"))?;
        self.enqueue_frame(
            method,
            &RequestFrame {
                jsonrpc: "2.0",
                id: Some(id),
                method,
                params,
            },
            class,
            deadline,
        )?;
        Ok(id)
    }

    /// Claude stream-json has its own envelope, not JSON-RPC. The port
    /// reserves the actual encoded allocation until the writer completes.
    pub(crate) fn enqueue_claude_frame_receipt(
        &self,
        frame: &Value,
        class: WriteClass,
        deadline: Instant,
        available_bytes: usize,
    ) -> Result<(crate::transport::WriteReceipt, usize)> {
        let frame = self
            .encode_frame(
                frame,
                class,
                available_bytes.min(crate::transport::writer_policy_limit()),
                deadline,
            )
            .map_err(admission_error)?;
        let retained = frame.bytes.capacity();
        let receipt = self
            .try_enqueue_owned_frame(frame, class, deadline)
            .map_err(|rejected| admission_error(rejected.into_error()))?;
        Ok((receipt, retained))
    }

    pub(crate) fn enqueue_claude_automatic_reply(&mut self, frame: &Value) -> Result<()> {
        self.enqueue_automatic_reply(frame)
    }

    /// Legacy receipt pressure hint only; physical count/byte admission is authoritative.
    pub(crate) fn command_write_available(&self) -> bool {
        self.writes.len() < PENDING_WRITES - 4
    }

    pub(crate) fn enqueue_notification_with_class(
        &mut self,
        method: &'static str,
        params: &Value,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<()> {
        self.enqueue_frame(
            method,
            &RequestFrame {
                jsonrpc: "2.0",
                id: None,
                method,
                params,
            },
            class,
            deadline,
        )
    }

    /// Automatic protocol replies do not wait for stdin.
    pub(crate) fn enqueue_rejection(&mut self, id: &Value, message: &str) -> Result<()> {
        self.enqueue_automatic_reply(&RejectionFrame {
            jsonrpc: "2.0",
            id,
            error: Rejection {
                code: -32601,
                message,
            },
        })
    }

    pub(crate) fn enqueue_response(&mut self, id: &Value, result: &Value) -> Result<()> {
        self.enqueue_automatic_reply(&ResponseFrame {
            jsonrpc: "2.0",
            id,
            result,
        })
    }

    fn enqueue_automatic_reply(&mut self, frame: &impl serde::Serialize) -> Result<()> {
        self.enqueue_frame(
            "protocol/reply",
            frame,
            WriteClass::ProtocolReplyCancel,
            Instant::now() + CONTROL_TIMEOUT,
        )
    }

    pub(crate) fn defer_cleanup(&mut self) {
        self.defer_cleanup = true;
    }

    fn transport_failed(&mut self) {
        if !self.defer_cleanup {
            self.shutdown();
        }
    }

    /// The command owner retains this receipt; flush is the only delivery
    /// boundary a one-way JSON-RPC response can prove.
    pub(crate) fn enqueue_reply_receipt(
        &mut self,
        id: &Value,
        result: &Value,
        deadline: Instant,
    ) -> Result<crate::transport::WriteReceipt> {
        self.enqueue_owned_frame(
            &ResponseFrame {
                jsonrpc: "2.0",
                id,
                result,
            },
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
    }

    pub(crate) fn has_pending_writes(&self) -> bool {
        !self.writes.is_empty()
    }

    fn enqueue_frame(
        &mut self,
        method: &'static str,
        frame: &impl serde::Serialize,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<()> {
        // Legacy synchronous facades retain eager harvesting. The command
        // port harvests only from poll, so admission cannot trigger cleanup.
        if !self.defer_cleanup {
            self.reap_writes()?;
        }
        if self.writes.len() >= PENDING_WRITES {
            return Err(Error::with_code(
                "运行时写入回执队列已满；请求尚未派发",
                "queue_full",
            ));
        }
        let receipt = self.enqueue_owned_frame(frame, class, deadline)?;
        self.writes.push((method, receipt));
        Ok(())
    }

    fn enqueue_owned_frame(
        &self,
        frame: &impl serde::Serialize,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<crate::transport::WriteReceipt> {
        let frame = self
            .encode_frame(
                frame,
                class,
                crate::transport::writer_policy_limit(),
                deadline,
            )
            .map_err(admission_error)?;
        self.try_enqueue_owned_frame(frame, class, deadline)
            .map_err(|rejected| admission_error(rejected.into_error()))
    }

    /// Budget-aware codec bridge for typed owners. The borrowed source is not
    /// copied into an intermediate Value; failure/slot checks precede encoding.
    fn encode_frame(
        &self,
        frame: &impl serde::Serialize,
        class: WriteClass,
        allocation_limit: usize,
        deadline: Instant,
    ) -> std::io::Result<OwnedFrame> {
        self.write_input()?
            .encode_json(frame, class, allocation_limit, deadline)
    }

    /// Retry this owned frame unchanged after local rejection. Every class
    /// crosses the same recorded terminal barrier before physical admission.
    #[allow(clippy::result_large_err)]
    fn try_enqueue_owned_frame(
        &self,
        frame: OwnedFrame,
        class: WriteClass,
        deadline: Instant,
    ) -> std::result::Result<WriteReceipt, NotAdmitted> {
        let mut frame = frame;
        frame.retain_lifetime(self.read_account.clone());
        match self.write_input() {
            Ok(input) => input.try_enqueue(frame, class, deadline),
            Err(error) => Err(NotAdmitted::new(frame, class, deadline, error)),
        }
    }

    fn write_input(&self) -> std::io::Result<&crate::transport::PipeWriter> {
        // The barrier precedes even bounded encoding for every class/facade.
        if let Some(error) = &self.failed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                error.to_string(),
            ));
        }
        self.input
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "运行时输入已关闭"))
    }

    fn reap_writes(&mut self) -> Result<()> {
        use crate::transport::WriteOutcome;
        let mut index = 0;
        while index < self.writes.len() {
            let Some(outcome) = self.writes[index].1.try_outcome() else {
                index += 1;
                continue;
            };
            if matches!(outcome, WriteOutcome::Written(c) if c.met_deadline()) {
                self.writes.remove(index);
                continue;
            }
            let method = self.writes[index].0;
            let (detail, code) = match outcome {
                WriteOutcome::Written(c) => (format!("physically Written late: {c:?}"), "timeout"),
                WriteOutcome::NotWritten(error) => {
                    (format!("known not written: {error}"), "transport")
                }
                WriteOutcome::Indeterminate(error) => (
                    format!("physical write indeterminate: {error}"),
                    "transport",
                ),
                WriteOutcome::ObservationTimeout { .. } => (
                    "outcome not yet observed; receipt retained".into(),
                    "timeout",
                ),
            };
            // A nonterminal observation never consumes the physical receipt. A
            // failed terminal fact is retained too, until explicit owner cleanup.
            self.transport_failed();
            return Err(self.with_stderr(Error::with_code(
                format!("Runtime {method}: {detail}; not replayed"),
                code,
            )));
        }
        Ok(())
    }

    pub(crate) fn begin(&mut self, method: &str, params: &Value) -> Result<u64> {
        self.begin_until(method, params, Instant::now() + CONTROL_TIMEOUT)
    }
    pub(crate) fn begin_until(
        &mut self,
        method: &str,
        params: &Value,
        deadline: Instant,
    ) -> Result<u64> {
        self.begin_until_with_class(method, params, WriteClass::Ordinary, deadline)
    }
    fn begin_until_with_class(
        &mut self,
        method: &str,
        params: &Value,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<u64> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("运行时请求序号耗尽"))?;
        self.write_until_class(
            &RequestFrame {
                jsonrpc: "2.0",
                id: Some(id),
                method,
                params,
            },
            class,
            deadline,
        )?;
        Ok(id)
    }
    pub(crate) fn request(&mut self, method: &str, params: &Value) -> Result<Retained<Value>> {
        self.request_classified(method, params, |_| None)
    }
    pub(crate) fn request_classified(
        &mut self,
        method: &str,
        params: &Value,
        classify: impl Fn(&Value) -> Option<&'static str>,
    ) -> Result<Retained<Value>> {
        self.request_with_classified(method, params, WriteClass::Ordinary, classify)
    }
    pub(crate) fn request_with_class(
        &mut self,
        method: &str,
        params: &Value,
        class: WriteClass,
    ) -> Result<Retained<Value>> {
        self.request_with_classified(method, params, class, |_| None)
    }
    fn request_with_classified(
        &mut self,
        method: &str,
        params: &Value,
        class: WriteClass,
        classify: impl Fn(&Value) -> Option<&'static str>,
    ) -> Result<Retained<Value>> {
        observe_request(method, || {
            let deadline = Instant::now() + CONTROL_TIMEOUT;
            let id = self.begin_until_with_class(method, params, class, deadline)?;
            loop {
                let record = self.wait_for_record(method, deadline)?;
                if record["id"].as_u64() == Some(id) && record.get("method").is_none() {
                    if let Some(error) = record.get("error") {
                        return Err(self.with_stderr(Error::with_code(
                            format!("运行时拒绝控制请求，请检查适配器版本与配置：{error}"),
                            classify(error).unwrap_or("internal"),
                        )));
                    }
                    return record.try_map(|mut value| {
                        value
                            .as_object_mut()
                            .and_then(|object| object.remove("result"))
                            .ok_or_else(|| Error::with_code("运行时响应缺少结果", "transport"))
                    });
                }
                self.defer(record)?;
            }
        })
    }
    fn wait_for_record(&mut self, method: &str, deadline: Instant) -> Result<Retained<Value>> {
        let result = if Instant::now() >= deadline {
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            self.records
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        };
        match result {
            Ok(Ok(record)) => Ok(self.deliver(record)),
            Ok(Err(error)) => Err(self.with_stderr(error)),
            Err(error) => {
                // A late reply cannot be attributed safely to a retried command.
                // Retire this connection rather than continuing on an ambiguous stream.
                self.shutdown();
                Err(self.with_stderr(match error {
                    mpsc::RecvTimeoutError::Timeout => Error::with_code(
                        format!("运行时控制请求 {method} 超时；连接已停止，未自动重发"),
                        "timeout",
                    ),
                    mpsc::RecvTimeoutError::Disconnected => Error::with_code(
                        format!("运行时控制请求 {method} 的响应通道已关闭"),
                        "transport",
                    ),
                }))
            }
        }
    }
    /// Read one record while a protocol adapter services interleaved agent requests.
    pub(crate) fn receive(&mut self, method: &str, deadline: Instant) -> Result<Retained<Value>> {
        if let Some(record) = self.queued.pop() {
            let permit = self
                .deferred_permits
                .pop_front()
                .expect("deferred record permit");
            return Ok(Retained::from_record(record, permit));
        }
        self.wait_for_record(method, deadline)
    }
    pub(crate) fn notify(&mut self, method: &str, params: &Value) -> Result<()> {
        self.notify_with_class(method, params, WriteClass::Ordinary)
    }
    pub(crate) fn notify_with_class(
        &mut self,
        method: &str,
        params: &Value,
        class: WriteClass,
    ) -> Result<()> {
        self.write_until_class(
            &RequestFrame {
                jsonrpc: "2.0",
                id: None,
                method,
                params,
            },
            class,
            Instant::now() + CONTROL_TIMEOUT,
        )
    }
    pub(crate) fn respond(&mut self, id: &Value, result: &Value) -> Result<()> {
        self.write_until_class(
            &ResponseFrame {
                jsonrpc: "2.0",
                id,
                result,
            },
            WriteClass::ProtocolReplyCancel,
            Instant::now() + CONTROL_TIMEOUT,
        )
    }
    pub(crate) fn reject(&mut self, id: &Value, message: &str) -> Result<()> {
        self.write_until_class(
            &RejectionFrame {
                jsonrpc: "2.0",
                id,
                error: Rejection {
                    code: -32601,
                    message,
                },
            },
            WriteClass::ProtocolReplyCancel,
            Instant::now() + CONTROL_TIMEOUT,
        )
    }
    /// `poll` preserves valid records before a terminal error in the same
    /// batch. After consuming that evidence, adapters must cross this barrier
    /// before dispatching more mutations. Reaping writes is nonblocking; live
    /// command owners still perform process cleanup explicitly.
    pub(crate) fn check_failure(&mut self) -> Result<()> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        self.reap_writes()
    }

    pub(crate) fn poll(&mut self) -> Result<Vec<Retained<Value>>> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let write_error = self.reap_writes().err();
        let mut values = self
            .queued
            .drain_batch(64)
            .into_iter()
            .map(|record| {
                let permit = self
                    .deferred_permits
                    .pop_front()
                    .expect("deferred record permit");
                Retained::from_record(record, permit)
            })
            .collect::<Vec<_>>();
        for _ in values.len()..64 {
            let error = match self.records.try_recv() {
                Ok(Ok(record)) => {
                    values.push(self.deliver(record));
                    continue;
                }
                Ok(Err(error)) => Some(error),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Error::with_code("运行时协议连接已关闭", "transport"))
                }
            };
            if let Some(error) = error {
                let error = self.with_stderr(error);
                if values.is_empty() {
                    return Err(error);
                }
                // Commit already received output before reporting terminal transport failure.
                self.failed = Some(error);
            }
            break;
        }
        if let Some(error) = write_error {
            if values.is_empty() {
                return Err(error);
            }
            self.failed = Some(error);
        }
        Ok(values)
    }

    pub(crate) fn finish(&mut self) {
        self.input.take();
        for _ in 0..50 {
            if self
                .child
                .as_mut()
                .is_none_or(|child| child.try_wait().is_ok_and(|status| status.is_some()))
            {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.shutdown();
    }
    /// Let the process end on its own without waiting here: its input
    /// closes, and a thread stops it if it has not ended within seconds.
    /// A Claude Code process ended this way removes its own registration.
    pub(crate) fn retire(&mut self) {
        self.input.take();
        let Some(mut child) = self.child.take() else {
            return;
        };
        let read_owner = self.read_account.clone();
        thread::spawn(move || {
            let _read_owner = read_owner;
            for _ in 0..250 {
                if child.try_wait().is_ok_and(|status| status.is_some()) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            child.stop();
        });
    }
    pub(crate) fn shutdown(&mut self) {
        if let Some(child) = &mut self.child {
            child.stop();
        }
        // Explicit cleanup has reaped the process. Dropping receipt handles
        // now cannot release a live process or move writes to a new stream.
        self.input.take();
        self.writes.clear();
    }
}

pub(crate) fn observe_request<T>(method: &str, request: impl FnOnce() -> Result<T>) -> Result<T> {
    let started = Instant::now();
    let result = request();
    let elapsed_ms = started.elapsed().as_millis();
    if let Err(error) = &result {
        tracing::warn!(target: "velune_agent_runtime", event = "native_rpc_failed", phase = "control_rpc",
            method, elapsed_ms, failure_kind = error.code());
    } else {
        tracing::debug!(target: "velune_agent_runtime", event = "native_rpc_completed", phase = "control_rpc",
            method, elapsed_ms);
    }
    result
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn admission_error(error: std::io::Error) -> Error {
    Error::with_code(
        format!("运行时请求尚未派发：{error}"),
        match error.kind() {
            std::io::ErrorKind::WouldBlock => "queue_full",
            std::io::ErrorKind::InvalidInput => "allocation_budget",
            std::io::ErrorKind::TimedOut => "timeout",
            _ => "transport",
        },
    )
}

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod probe_tests;
