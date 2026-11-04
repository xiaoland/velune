//! One explicit activity's recovery. Waiting, start acknowledgement and interrupt
//! are advanced by poll; none waits for an app-server response.
use super::*;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAXIMUM: u32 = 5;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_EARLY_RECORDS: usize = 128;

#[derive(Default)]
pub(super) enum Phase {
    #[default]
    Running,
    Waiting {
        deadline: Instant,
        at_unix_ms: i64,
    },
    Starting {
        request: u64,
        deadline: Instant,
        cancelled: bool,
    },
    Stopping {
        request: u64,
        deadline: Instant,
    },
    Ended(Option<RuntimeRetry>),
}

#[derive(Default)]
pub(super) struct Retry {
    pub(super) attempts: u32,
    pub(super) phase: Phase,
    pub(super) native: bool,
    pub(super) effort: Option<String>,
    pub(super) permission_mode: Option<String>,
    pub(super) early: Vec<Value>,
    /// Identity confirmed by an early turn/started, checked again against ack.
    pub(super) started: Option<String>,
    pub(super) interrupt: Option<u64>,
}

fn delay(attempt: u32) -> Option<Duration> {
    (1..=MAXIMUM)
        .contains(&attempt)
        .then(|| Duration::from_secs(2_u64.pow(attempt)))
}

impl Retry {
    pub(super) fn can_steer(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }
    fn view(&self) -> Option<RuntimeRetry> {
        match &self.phase {
            Phase::Waiting { at_unix_ms, .. } => Some(RuntimeRetry::Waiting {
                attempt: self.attempts,
                maximum: MAXIMUM,
                at_unix_ms: *at_unix_ms,
            }),
            Phase::Ended(view) => view.clone(),
            _ if self.native => Some(RuntimeRetry::Native),
            _ if self.attempts > 0 => Some(RuntimeRetry::Attempted {
                attempt: self.attempts,
                maximum: MAXIMUM,
            }),
            _ => None,
        }
    }
}

impl NativeSession {
    pub(super) fn codex_retry_transient(&mut self, message: &mut Message) -> bool {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.run_state == RunState::Stopping)
            || self.compacting
            || self.codex.retry.native
            || matches!(self.codex.retry.phase, Phase::Stopping { .. })
            || !message.blocks.iter().any(|b| {
                matches!(
                    b,
                    MessageBlock::RuntimeFailure {
                        failure: RuntimeFailureKind::ServerOverloaded
                            | RuntimeFailureKind::RateLimited,
                        ..
                    }
                )
            })
        {
            return false;
        }
        let attempt = self.codex.retry.attempts + 1;
        let Some(wait) = delay(attempt) else {
            self.codex.retry.phase = Phase::Ended(Some(RuntimeRetry::Exhausted {
                attempts: self.codex.retry.attempts,
            }));
            return false;
        };
        self.codex.retry.attempts = attempt;
        let at_unix_ms = (SystemTime::now() + wait)
            .duration_since(UNIX_EPOCH)
            .map_or(0, |v| v.as_millis() as i64);
        self.codex.retry.phase = Phase::Waiting {
            deadline: Instant::now() + wait,
            at_unix_ms,
        };
        self.turn_id = None;
        self.running();
        true
    }

    pub(super) fn codex_poll_retry(&mut self) -> Result<()> {
        match self.codex.retry.phase {
            Phase::Waiting { deadline, .. } if Instant::now() >= deadline => {
                let input = crate::TurnInput {
                    text: String::new(),
                    images: vec![],
                    effort: self.codex.retry.effort.clone(),
                    permission_mode: self.codex.retry.permission_mode.clone(),
                };
                let params = self.codex_turn_params(&input)?;
                let request = self.rpc.begin("turn/start", &params)?;
                self.codex.retry.native = false;
                self.codex.retry.started = None;
                self.codex.retry.phase = Phase::Starting {
                    request,
                    deadline: Instant::now() + RESPONSE_TIMEOUT,
                    cancelled: false,
                };
            }
            Phase::Starting { deadline, .. } | Phase::Stopping { deadline, .. }
                if Instant::now() >= deadline =>
            {
                self.notice("Codex 恢复控制请求未在期限内确认；已停止自动恢复，未重新派发。 ");
                return Err(Error::new("Codex 恢复控制请求超时"));
            }
            _ => {}
        }
        self.codex_retry_view();
        Ok(())
    }

    /// Called before ordinary record handling. A response belongs to one request
    /// of this connection only; a stale response cannot activate a later activity.
    pub(super) fn codex_recovery_record(&mut self, record: &Value) -> Result<bool> {
        if record.get("method").is_none() {
            let Some(id) = record["id"].as_u64() else {
                return Ok(false);
            };
            if matches!(self.codex.retry.phase, Phase::Ended(_)) {
                return Ok(true);
            }
            if self.codex.retry.interrupt == Some(id) {
                if record.get("error").is_some() {
                    return Err(Error::new(format!("Codex 未确认停止：{}", record["error"])));
                }
                return Ok(true);
            }
            match self.codex.retry.phase {
                Phase::Starting {
                    request, cancelled, ..
                } if id == request => {
                    if record.get("error").is_some() {
                        self.codex_recovery_failure(format!(
                            "Codex 拒绝恢复请求：{}",
                            record["error"]
                        ));
                        self.codex_end_recovery(Some(TurnOutcome::Failed), false);
                        self.finish_turn(RunState::Failed)?;
                        return Ok(true);
                    }
                    let turn = record["result"]["turn"]["id"]
                        .as_str()
                        .ok_or_else(|| Error::new("Codex 恢复应答缺少轮次身份"))?;
                    self.codex_bind_recovery_turn(turn)?;
                    self.codex.retry.phase = Phase::Running;
                    if cancelled {
                        self.codex_interrupt_recovery()?;
                    }
                    self.codex_replay_early()?;
                    return Ok(true);
                }
                Phase::Stopping { request, .. } if id == request => {
                    if record.get("error").is_some() {
                        return Err(Error::new(format!("Codex 未确认停止：{}", record["error"])));
                    }
                    // Acknowledging interrupt is not terminal evidence. Wait for turn/completed.
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }
        let params = &record["params"];
        if params["threadId"]
            .as_str()
            .is_some_and(|id| Some(id) != self.native_id.as_deref())
        {
            return Ok(false);
        }
        if let Phase::Starting { cancelled, .. } = self.codex.retry.phase {
            if record["method"] == "turn/started" {
                let turn = params["turn"]["id"]
                    .as_str()
                    .ok_or_else(|| Error::new("Codex 恢复通知缺少轮次身份"))?;
                self.codex_bind_recovery_turn(turn)?;
                if cancelled {
                    // Keep the start correlation until its ack, but send interrupt only once.
                    self.codex_send_early_interrupt()?;
                }
                self.codex_replay_early()?;
            } else if self.turn_id.is_none()
                && (params["turnId"].is_string() || record["method"] == "turn/completed")
            {
                if self.codex.retry.early.len() >= MAX_EARLY_RECORDS {
                    return Err(Error::new("Codex 恢复通知在身份确认前超出上限"));
                }
                self.codex.retry.early.push(record.clone());
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn codex_bind_recovery_turn(&mut self, turn: &str) -> Result<()> {
        if self
            .codex
            .retry
            .started
            .as_deref()
            .is_some_and(|id| id != turn)
        {
            return Err(Error::new("Codex 恢复应答与通知的轮次身份不一致"));
        }
        self.codex.retry.started = Some(turn.into());
        self.turn_id = Some(turn.into());
        Ok(())
    }

    fn codex_replay_early(&mut self) -> Result<()> {
        for record in std::mem::take(&mut self.codex.retry.early) {
            self.codex_handle_record(record)?;
        }
        Ok(())
    }

    fn codex_send_early_interrupt(&mut self) -> Result<()> {
        // Record the one interrupt separately while the start ack is outstanding.
        if self.codex.retry.interrupt.is_some() {
            return Ok(());
        }
        let turn = self
            .turn_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 未确认恢复轮次"))?;
        self.codex.retry.interrupt = Some(self.rpc.begin(
            "turn/interrupt",
            &json!({"threadId":self.native_id,"turnId":turn}),
        )?);
        Ok(())
    }

    fn codex_interrupt_recovery(&mut self) -> Result<()> {
        if let Some(request) = self.codex.retry.interrupt {
            self.codex.retry.phase = Phase::Stopping {
                request,
                deadline: Instant::now() + RESPONSE_TIMEOUT,
            };
        } else {
            let turn = self
                .turn_id
                .as_deref()
                .ok_or_else(|| Error::new("Codex 未确认恢复轮次"))?;
            let request = self.rpc.begin(
                "turn/interrupt",
                &json!({"threadId":self.native_id,"turnId":turn}),
            )?;
            self.codex.retry.phase = Phase::Stopping {
                request,
                deadline: Instant::now() + RESPONSE_TIMEOUT,
            };
        }
        Ok(())
    }

    pub(super) fn codex_cancel_recovery(&mut self) -> Result<()> {
        match &mut self.codex.retry.phase {
            Phase::Waiting { .. } => {
                self.codex_end_recovery(Some(TurnOutcome::Cancelled), false);
                return self.finish_turn(RunState::Idle);
            }
            Phase::Starting { cancelled, .. } => {
                *cancelled = true;
                if self.turn_id.is_some() {
                    self.codex_send_early_interrupt()?;
                }
            }
            Phase::Stopping { .. } | Phase::Ended(_) => return Ok(()),
            Phase::Running => self.codex_interrupt_recovery()?,
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        self.codex_retry_view();
        Ok(())
    }

    pub(super) fn codex_recovery_failure(&mut self, detail: String) {
        if self
            .snapshot
            .as_ref()
            .and_then(|s| s.executions.current.as_ref())
            .is_none_or(|a| a.ended)
        {
            return;
        }
        self.message(Message {
            id: format!(
                "codex-recovery-failure-{}",
                self.snapshot.as_ref().map_or(0, |s| s.revision)
            ),
            role: MessageRole::System,
            completed: true,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::RuntimeFailure {
                failure: RuntimeFailureKind::Other,
                detail,
                retry: None,
            }],
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        });
        self.codex_end_recovery(Some(TurnOutcome::Failed), false);
    }

    pub(super) fn codex_end_recovery(&mut self, outcome: Option<TurnOutcome>, exhausted: bool) {
        if self
            .snapshot
            .as_ref()
            .and_then(|s| s.executions.current.as_ref())
            .is_some_and(|a| a.ended)
        {
            return;
        }
        let view = if exhausted {
            Some(RuntimeRetry::Exhausted {
                attempts: self.codex.retry.attempts,
            })
        } else {
            match outcome {
                Some(TurnOutcome::Completed) => Some(RuntimeRetry::Recovered),
                Some(TurnOutcome::Cancelled) => Some(RuntimeRetry::Cancelled),
                _ => None,
            }
        };
        self.codex.retry.phase = Phase::Ended(view);
        self.codex.retry.early.clear();
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.executions.finish(outcome);
        }
        self.codex_retry_view();
    }

    pub(super) fn codex_retry_view(&mut self) {
        let view = self.codex.retry.view();
        if let Some(snapshot) = &mut self.snapshot
            && let Some(activity) = &snapshot.executions.current
            && let Some(execution) = snapshot
                .executions
                .records
                .iter()
                .find(|r| r.id == activity.execution_id)
        {
            let ids = &execution.message_ids;
            for message in &mut snapshot.messages {
                if !ids.contains(&message.id) {
                    continue;
                }
                for block in &mut message.blocks {
                    if let MessageBlock::RuntimeFailure { detail, retry, .. } = block
                        && *retry != view
                    {
                        // Older clients use this adapter-owned note during
                        // native retry. It must not contradict the terminal
                        // typed state once that retry has ended.
                        if *retry == Some(RuntimeRetry::Native)
                            && let Some(raw) =
                                detail.strip_suffix(super::codex::NATIVE_RETRY_DETAIL)
                        {
                            *detail = raw.to_owned();
                        }
                        *retry = view.clone();
                        snapshot.revision += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overload_backoff_is_bounded_and_exponential() {
        assert_eq!(
            (1..=5)
                .map(|n| delay(n).unwrap().as_secs())
                .collect::<Vec<_>>(),
            vec![2, 4, 8, 16, 32]
        );
        assert!(delay(6).is_none());
        assert!(delay(0).is_none());
    }
}

#[cfg(all(test, unix))]
mod protocol_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn recovery_waits_resumes_without_input_and_respects_native_retry_and_stop() {
        let root = std::env::temp_dir().join(format!(
            "velune-overload-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join("codex");
        std::fs::write(&binary, r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'codex-cli 0.159.3'; exit 0; fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CODEX_HOME/requests.log"
  case "$line" in
    *'"method":"turn/start"'*) printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9]*\).*/{"id":\1,"result":{"turn":{"id":"retry-turn"}}}/p' ;;
    *'"id":'*) printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9]*\).*/{"id":\1,"result":{}}/p' ;;
  esac
done
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::Codex,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        let messages = serde_json::from_value(json!([
            {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"ORIGINAL_PROMPT"}]},
            {"id":"tool","role":"assistant","completed":true,"blocks":[{"kind":"tool","toolID":"call","title":"done","state":"completed","output":"already executed"}]}
        ])).unwrap();
        session.replace_snapshot("synthetic", Some(&root), "codex", messages);
        {
            let snapshot = session.snapshot.as_mut().unwrap();
            snapshot.executions.begin(ExecutionKind::UserTurn, false);
            for message in &snapshot.messages {
                snapshot.executions.attach(&message.id);
            }
        }
        let fail = |session: &mut NativeSession, turn: &str, code: &str, native: bool| {
            if session
                .snapshot
                .as_ref()
                .unwrap()
                .executions
                .current
                .as_ref()
                .unwrap()
                .ended
            {
                session
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .executions
                    .begin(ExecutionKind::UserTurn, true);
                session.codex.retry.phase = Phase::Running;
            }
            session.turn_id = Some(turn.into());
            session.running();
            let info: Value = serde_json::from_str(code).unwrap_or_else(|_| json!(code));
            let error = json!({"message":"synthetic capacity","codexErrorInfo":info});
            session.codex_handle_record(json!({"method":"error","params":{"turnId":turn,"willRetry":native,"error":error}})).unwrap();
            session.codex_handle_record(json!({"method":"turn/completed","params":{"turn":{"id":turn,"status":"failed","error":error}}})).unwrap();
        };
        fail(&mut session, "first", "serverOverloaded", false);
        assert!(session.turn_running());
        assert!(!session.can_steer());
        session.codex_poll_retry().unwrap();
        assert!(
            !std::fs::read_to_string(root.join("requests.log"))
                .unwrap()
                .contains("turn/start")
        );
        if let Phase::Waiting { deadline, .. } = &mut session.codex.retry.phase {
            *deadline = Instant::now();
        }

        session.codex_poll_retry().unwrap();
        for _ in 0..100 {
            session.poll().unwrap();
            if session.turn_id.as_deref() == Some("retry-turn") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let requests = std::fs::read_to_string(root.join("requests.log")).unwrap();
        assert!(requests.contains("\"input\":[]"));
        assert!(!requests.contains("ORIGINAL_PROMPT"));
        assert_eq!(
            session
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::User)
                .count(),
            1
        );
        session.codex_handle_record(json!({"method":"turn/completed","params":{"turn":{"id":"retry-turn","status":"completed"}}})).unwrap();
        assert_eq!(
            session
                .snapshot()
                .unwrap()
                .executions
                .record("user")
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
        assert!(matches!(
            &session.snapshot().unwrap().messages[1].blocks[0],
            MessageBlock::Tool {
                state: ToolState::Completed,
                ..
            }
        ));
        fail(&mut session, "second", "serverOverloaded", false);
        session.cancel().unwrap();
        assert!(!session.turn_running());
        assert!(!matches!(session.codex.retry.phase, Phase::Waiting { .. }));
        fail(&mut session, "native", "serverOverloaded", true);
        assert!(!session.turn_running());
        assert!(!matches!(session.codex.retry.phase, Phase::Waiting { .. }));
        assert!(session.snapshot().unwrap().messages.iter().all(|message| {
            message.blocks.iter().all(|block| match block {
                MessageBlock::RuntimeFailure { detail, retry, .. } => {
                    *retry == Some(RuntimeRetry::Native)
                        || !detail.ends_with(super::super::codex::NATIVE_RETRY_DETAIL)
                }
                _ => true,
            })
        }));
        session.codex.retry.native = false;
        fail(&mut session, "other", "internalServerError", false);
        assert!(!session.turn_running());
        assert!(!matches!(session.codex.retry.phase, Phase::Waiting { .. }));
        for code in [
            "rateLimitExceeded",
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":429}}"#,
        ] {
            fail(&mut session, "frequency-limited", code, false);
            assert!(session.turn_running());
            assert!(matches!(session.codex.retry.phase, Phase::Waiting { .. }));
            session.cancel().unwrap();
        }
        fail(&mut session, "account-cap", "usageLimitExceeded", false);
        assert!(!session.turn_running());
        assert!(!matches!(session.codex.retry.phase, Phase::Waiting { .. }));
        session.codex.retry.attempts = MAXIMUM;
        fail(&mut session, "exhausted", "serverOverloaded", false);
        assert!(!session.turn_running());
        let old_activity = session
            .snapshot()
            .unwrap()
            .executions
            .current
            .as_ref()
            .unwrap()
            .id
            .clone();
        session.resume(Some("high".into()), None).unwrap();
        assert_eq!(session.codex.retry.attempts, 0);
        assert_eq!(session.codex.retry.effort.as_deref(), Some("high"));
        assert_ne!(
            session
                .snapshot()
                .unwrap()
                .executions
                .current
                .as_ref()
                .unwrap()
                .id,
            old_activity
        );
        fail(&mut session, "retry-turn", "serverOverloaded", false);
        assert!(matches!(session.codex.retry.phase, Phase::Waiting { .. }));
        session.fail_connection();
        assert!(!session.turn_running());
        assert!(
            session
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .all(|m| m.blocks.iter().all(|b| !matches!(
                    b,
                    MessageBlock::RuntimeFailure {
                        retry: Some(RuntimeRetry::Waiting { .. } | RuntimeRetry::Native),
                        ..
                    }
                )))
        );
        session.truncate_snapshot(1);
        assert_eq!(
            session
                .snapshot()
                .unwrap()
                .executions
                .record("user")
                .unwrap()
                .outcome,
            None
        );
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn delayed_retry_ack_does_not_block_poll_or_stop_and_early_completion_is_kept() {
        let root = std::env::temp_dir().join(format!(
            "velune-retry-barrier-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join("codex");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import sys,json,os,threading,time
if '--version' in sys.argv:
 print('codex-cli 0.159.3');sys.exit()
root=os.environ['CODEX_HOME']
lock=threading.Lock()
def emit(value):
 with lock: print(json.dumps(value),flush=True)
def start(request):
 open(root+'/waiting','w').close()
 while not os.path.exists(root+'/release'): time.sleep(.005)
 # Notifications can precede the start reply, including items and terminal.
 emit({'method':'item/completed','params':{'threadId':'test','turnId':'recovery','item':{'id':'reply','type':'agentMessage','text':'done'}}})
 emit({'method':'turn/started','params':{'threadId':'test','turn':{'id':'recovery'}}})
 time.sleep(.02)
 emit({'id':request,'result':{'turn':{'id':'recovery'}}})
for line in sys.stdin:
 r=json.loads(line)
 with open(root+'/requests','a') as f:f.write(line)
 if r.get('method')=='turn/start':threading.Thread(target=start,args=(r['id'],),daemon=True).start()
 elif r.get('method')=='turn/interrupt':
  emit({'id':r['id'],'result':{}})
  emit({'method':'turn/completed','params':{'threadId':'test','turn':{'id':r['params']['turnId'],'status':'completed'}}})
 elif 'id' in r:emit({'id':r['id'],'result':{}})
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::Codex,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            mcp_servers: vec![],
            session_agents: vec![],
        };
        let mut a = NativeSession::connect(config.clone()).unwrap();
        let mut b = NativeSession::connect(config).unwrap();
        for session in [&mut a, &mut b] {
            session.replace_snapshot("test", Some(&root), "codex", vec![]);
            session
                .snapshot
                .as_mut()
                .unwrap()
                .executions
                .begin(ExecutionKind::UserTurn, false);
            session.message(serde_json::from_value(json!({"id":"u","role":"user","completed":true,"blocks":[{"kind":"text","text":"ORIGINAL"}]})).unwrap());
            session.turn_id = Some("original".into());
            session.running();
        }
        let error = json!({"message":"busy","codexErrorInfo":"serverOverloaded"});
        a.codex_handle_record(json!({"method":"turn/completed","params":{"turn":{"id":"original","status":"failed","error":error}}})).unwrap();
        if let Phase::Waiting { deadline, .. } = &mut a.codex.retry.phase {
            *deadline = Instant::now();
        }
        let started = Instant::now();
        a.poll().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        for _ in 0..100 {
            if root.join("waiting").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("waiting").exists());
        // Loaded B and both stops are serviced before A's start ack is released.
        assert_eq!(b.snapshot().unwrap().conversation.id, "codex:test");
        let started = Instant::now();
        b.cancel().unwrap();
        a.cancel().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(a.snapshot().unwrap().run_state, RunState::Stopping);
        std::fs::write(root.join("release"), "").unwrap();
        for _ in 0..200 {
            a.poll().unwrap();
            if !a.turn_running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!a.turn_running());
        // Native completion wins a cancellation race, and its early item was not lost.
        assert_eq!(
            a.snapshot()
                .unwrap()
                .executions
                .record("u")
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
        assert!(
            a.snapshot()
                .unwrap()
                .messages
                .iter()
                .any(|m| m.id == "reply")
        );
        let requests = std::fs::read_to_string(root.join("requests")).unwrap();
        assert_eq!(
            requests
                .lines()
                .filter(|l| l.contains("turn/start"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .lines()
                .filter(|l| l.contains("turn/interrupt") && l.contains("recovery"))
                .count(),
            1
        );
        assert!(!requests.contains("ORIGINAL"));
        // A new explicit cycle can fail while its retry start is still unconfirmed.
        std::fs::remove_file(root.join("release")).unwrap();
        b.poll().unwrap();
        b.snapshot
            .as_mut()
            .unwrap()
            .executions
            .begin(ExecutionKind::UserTurn, true);
        b.codex.retry = Retry::default();
        b.turn_id = Some("second".into());
        b.running();
        b.codex_handle_record(json!({"method":"turn/completed","params":{"turn":{"id":"second","status":"failed","error":{"message":"busy","codexErrorInfo":"serverOverloaded"}}}})).unwrap();
        if let Phase::Waiting { deadline, .. } = &mut b.codex.retry.phase {
            *deadline = Instant::now();
        }
        b.poll().unwrap();
        let request = match b.codex.retry.phase {
            Phase::Starting { request, .. } => request,
            _ => panic!("retry was not dispatched"),
        };
        b.fail_connection();
        b.codex_handle_record(json!({"id":request,"result":{"turn":{"id":"late"}}}))
            .unwrap();
        assert_eq!(b.snapshot().unwrap().run_state, RunState::Failed);
        assert!(
            b.snapshot()
                .unwrap()
                .messages
                .iter()
                .all(|m| m.blocks.iter().all(|block| !matches!(
                    block,
                    MessageBlock::RuntimeFailure {
                        retry: Some(RuntimeRetry::Waiting { .. } | RuntimeRetry::Native),
                        ..
                    }
                )))
        );
        a.snapshot
            .as_mut()
            .unwrap()
            .executions
            .begin(ExecutionKind::UserTurn, true);
        a.codex.retry = Retry::default();
        a.turn_id = Some("rejected-source".into());
        a.running();
        a.codex_handle_record(json!({"method":"turn/completed","params":{"turn":{"id":"rejected-source","status":"failed","error":{"message":"busy","codexErrorInfo":"serverOverloaded"}}}})).unwrap();
        if let Phase::Waiting { deadline, .. } = &mut a.codex.retry.phase {
            *deadline = Instant::now();
        }
        a.poll().unwrap();
        let request = match a.codex.retry.phase {
            Phase::Starting { request, .. } => request,
            _ => panic!("retry was not dispatched"),
        };
        a.codex_handle_record(json!({"id":request,"error":{"message":"synthetic rejection"}}))
            .unwrap();
        assert!(!a.turn_running());
        assert!(a.snapshot().unwrap().messages.iter().any(|m| m.blocks.iter().any(|block| matches!(block, MessageBlock::RuntimeFailure { detail, retry: None, .. } if detail.contains("synthetic rejection")))));
        drop(a);
        drop(b);
        std::fs::remove_dir_all(root).unwrap();
    }
}
