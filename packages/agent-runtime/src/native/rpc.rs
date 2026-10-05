//! Bounded stdio JSON-RPC transport. Native protocols remain in their adapters.
use super::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};
const MAX_RECORD: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);
pub(super) struct RpcClient {
    child: Child,
    input: ChildStdin,
    records: Receiver<Result<Value>>,
    queued: VecDeque<Value>,
    next: u64,
    failed: Option<Error>,
}
fn prepare(command: &mut Command) {
    command.env_clear();
    for key in [
        "PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "SHELL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}
pub(super) fn stop(child: &mut Child) {
    #[cfg(unix)]
    {
        // Each child is placed in its own process group before exec. Close inherited pipes and tools too.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub(crate) fn bounded_output(command: Command) -> Result<Vec<u8>> {
    bounded_process(command, None, 32768)
}
pub(crate) fn bounded_process(
    mut command: Command,
    input: Option<Vec<u8>>,
    limit: usize,
) -> Result<Vec<u8>> {
    prepare(&mut command);
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new("运行时辅助进程无法启动"))?;
    if let Some(bytes) = input {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::new("辅助进程输入不可用"))?;
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("运行时输出不可用"))?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let outcome = stdout
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(outcome);
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                stop(&mut child);
                return Err(Error::new("运行时辅助进程状态不可用"));
            }
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            return Err(Error::new("运行时辅助进程超时"));
        }
        thread::sleep(Duration::from_millis(20));
    };
    stop(&mut child);
    let bytes = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| Error::new("运行时辅助进程输出未关闭"))?
        .map_err(|_| Error::new("运行时辅助进程读取失败"))?;
    if !status.success() || bytes.len() > limit {
        return Err(Error::new("运行时辅助进程失败或输出超出限制"));
    }
    Ok(bytes)
}

impl RpcClient {
    pub(super) fn spawn(mut command: Command, environment: &[(String, String)]) -> Result<Self> {
        prepare(&mut command);
        for (key, value) in environment {
            command.env(key, value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::new("运行时进程无法启动"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| Error::new("运行时输入不可用"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| Error::new("运行时输出不可用"))?;
        let (tx, records) = mpsc::sync_channel(64);
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        thread::spawn(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut bytes = Vec::new();
                    let read = reader
                        .by_ref()
                        .take((MAX_RECORD + 1) as u64)
                        .read_until(b'\n', &mut bytes);
                    let result = match read {
                        Ok(0) => Err(Error::new("运行时进程已退出")),
                        Ok(_) if bytes.len() > MAX_RECORD => Err(Error::new("运行时协议记录过大")),
                        Ok(_) => serde_json::from_slice(&bytes)
                            .map_err(|_| Error::new("运行时返回无效协议记录")),
                        Err(_) => Err(Error::new("运行时协议读取失败")),
                    };
                    let failed = result.is_err();
                    if tx.send(result).is_err() || failed {
                        break;
                    }
                }
            })
        });
        Ok(Self {
            child,
            input,
            records,
            queued: VecDeque::new(),
            next: 1,
            failed: None,
        })
    }
    fn write(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, value)
            .map_err(|_| Error::new("运行时请求编码失败"))?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| Error::new("运行时请求发送失败"))
    }
    pub(super) fn begin(&mut self, method: &str, params: &Value) -> Result<u64> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("运行时请求序号耗尽"))?;
        self.write(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }
    pub(super) fn request(&mut self, method: &str, params: &Value) -> Result<Value> {
        let id = self.begin(method, params)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let record = self
                .records
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| Error::new("运行时控制请求超时"))??;
            if record["id"].as_u64() == Some(id) && record.get("method").is_none() {
                if record.get("error").is_some() {
                    return Err(Error::new("运行时拒绝控制请求，请检查适配器版本与配置"));
                }
                return record
                    .get("result")
                    .cloned()
                    .ok_or_else(|| Error::new("运行时响应缺少结果"));
            }
            if self.queued.len() >= 64 {
                return Err(Error::new("运行时控制响应前的事件超出限制"));
            }
            self.queued.push_back(record);
        }
    }
    pub(super) fn notify(&mut self, method: &str, params: &Value) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub(super) fn respond(&mut self, id: &Value, result: &Value) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub(super) fn reject(&mut self, id: &Value, message: &str) -> Result<()> {
        self.write(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":message}}))
    }
    pub(super) fn poll(&mut self) -> Result<Vec<Value>> {
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

    pub(super) fn shutdown(&mut self) {
        stop(&mut self.child);
    }
}
impl Drop for RpcClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
