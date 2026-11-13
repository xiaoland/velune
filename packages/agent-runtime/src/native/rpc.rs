//! Bounded stdio JSON-RPC transport. Native protocols remain in their adapters.
use super::{Error, Result};
use crate::process_tree::ProcessTree;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};
const MAX_STDERR_BYTES: usize = 1024 * 1024;
pub(crate) const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) struct RpcClient {
    /// None once handed to a thread that lets it end; see `retire`.
    child: Option<ProcessTree>,
    input: Option<ChildStdin>,
    records: Receiver<Result<Value>>,
    queued: VecDeque<Value>,
    next: u64,
    failed: Option<Error>,
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
        let mut input = child
            .take_stdin()
            .ok_or_else(|| Error::new("运行时输入不可用"))?;
        input
            .write_all(&prepared.input_prefix)
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
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        thread::spawn(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut bytes = Vec::new();
                    let read = reader.by_ref().read_until(b'\n', &mut bytes);
                    let result = match read {
                        Ok(0) => Err(Error::new("运行时进程已退出")),
                        Ok(_) => serde_json::from_slice(&bytes).map_err(|error| {
                            Error::new(format!("运行时返回无效协议记录：{error}"))
                        }),
                        Err(error) => Err(Error::new(format!("运行时协议读取失败：{error}"))),
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
            records,
            queued: VecDeque::new(),
            next: 1,
            failed: None,
            stderr,
        })
    }
    fn with_stderr(&self, error: Error) -> Error {
        let detail = self
            .stderr
            .lock()
            .ok()
            .map(|bytes| {
                let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_STDERR_BYTES)])
                    .trim()
                    .to_owned();
                if bytes.len() > MAX_STDERR_BYTES {
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
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| Error::new("运行时输入已关闭"))?;
        serde_json::to_writer(&mut *input, value)
            .map_err(|error| Error::new(format!("运行时请求编码失败：{error}")))?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|error| Error::new(format!("运行时请求发送失败：{error}")))
    }
    pub(crate) fn claude_control(&mut self, request: Value) -> Result<Value> {
        let method = request["subtype"].as_str().unwrap_or("control_request");
        observe_request(method, || {
            let id = format!("velune-{}", self.next);
            self.next += 1;
            self.write(&json!({"type":"control_request","request_id":id,"request":request}))?;
            let deadline = Instant::now() + CONTROL_TIMEOUT;
            loop {
                let record = self.wait_for_record(method, deadline)?;
                if record["type"] == "control_response" && record["response"]["request_id"] == id {
                    if record["response"]["subtype"] != "success" {
                        return Err(Error::new("Claude 拒绝控制请求"));
                    }
                    return Ok(record["response"]["response"].clone());
                }
                self.queued.push_back(record);
            }
        })
    }
    pub(crate) fn begin(&mut self, method: &str, params: &Value) -> Result<u64> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("运行时请求序号耗尽"))?;
        self.write(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }
    pub(crate) fn request(&mut self, method: &str, params: &Value) -> Result<Value> {
        self.request_classified(method, params, |_| None)
    }
    pub(crate) fn request_classified(
        &mut self,
        method: &str,
        params: &Value,
        classify: impl Fn(&Value) -> Option<&'static str>,
    ) -> Result<Value> {
        observe_request(method, || {
            let id = self.begin(method, params)?;
            let deadline = Instant::now() + CONTROL_TIMEOUT;
            loop {
                let mut record = self.wait_for_record(method, deadline)?;
                if record["id"].as_u64() == Some(id) && record.get("method").is_none() {
                    if let Some(error) = record.get("error") {
                        return Err(self.with_stderr(Error::with_code(
                            format!("运行时拒绝控制请求，请检查适配器版本与配置：{error}"),
                            classify(error).unwrap_or("internal"),
                        )));
                    }
                    return record
                        .as_object_mut()
                        .and_then(|record| record.remove("result"))
                        .ok_or_else(|| Error::new("运行时响应缺少结果"));
                }
                self.queued.push_back(record);
            }
        })
    }
    fn wait_for_record(&self, method: &str, deadline: Instant) -> Result<Value> {
        self.records
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| {
                self.with_stderr(match error {
                    mpsc::RecvTimeoutError::Timeout => Error::with_code(
                        format!(
                            "运行时控制请求 {method} 在 {} 秒内未响应",
                            CONTROL_TIMEOUT.as_secs()
                        ),
                        "timeout",
                    ),
                    mpsc::RecvTimeoutError::Disconnected => Error::with_code(
                        format!("运行时控制请求 {method} 的响应通道已关闭"),
                        "transport",
                    ),
                })
            })?
            .map_err(|error| self.with_stderr(error))
    }
    /// Read one record while a protocol adapter services interleaved agent requests.
    pub(crate) fn receive(&mut self, method: &str, deadline: Instant) -> Result<Value> {
        if let Some(record) = self.queued.pop_front() {
            return Ok(record);
        }
        self.wait_for_record(method, deadline)
    }
    pub(crate) fn notify(&mut self, method: &str, params: &Value) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub(crate) fn respond(&mut self, id: &Value, result: &Value) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub(crate) fn reject(&mut self, id: &Value, message: &str) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":message}}))
    }
    pub(crate) fn poll(&mut self) -> Result<Vec<Value>> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let mut values = self.queued.drain(..).collect::<Vec<_>>();
        for _ in 0..64 {
            let error = match self.records.try_recv() {
                Ok(Ok(value)) => {
                    values.push(value);
                    continue;
                }
                Ok(Err(error)) => Some(error),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Error::new("运行时协议连接已关闭")),
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
        thread::spawn(move || {
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

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
