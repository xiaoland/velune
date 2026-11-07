//! Scheduler dispatch reuses the ordinary session pool, control admission,
//! model selection, gateway and native approval flow.
use super::*;
use crate::scheduled_tasks::{next_occurrence, now, verify_source};
use crate::{
    ScheduledRun, ScheduledRunPhase as Phase, ScheduledTarget as Target, ScheduledTask,
    ScheduledTaskStatus as Status, ScheduledTasksOperation as Operation, ScheduledTasksState,
};

impl CoreRuntime {
    pub(crate) fn scheduled_tasks(
        &mut self,
        operation: Operation,
    ) -> Result<ScheduledTasksState, RuntimeError> {
        match operation {
            Operation::List => {}
            Operation::Save { task } => {
                self.validate_scheduled_target(&task)?;
                self.scheduled_tasks.save(task)?;
            }
            Operation::SetEnabled {
                id,
                revision,
                enabled,
            } => {
                let mut task = self.scheduled_tasks.checked(&id, revision)?;
                if enabled {
                    task.status = Status::Active;
                    self.validate_scheduled_target(&task)?;
                    self.check_handover(&task)?;
                    self.scheduled_tasks.save(task)?;
                } else {
                    // Pausing remains possible even when an execution target or
                    // model has disappeared. A revision change also skips waits.
                    self.scheduled_tasks.pause(task)?;
                }
            }
            Operation::PreviewSchedule {
                rrule,
                time_zone,
                pattern,
            } => {
                return Ok(ScheduledTasksState {
                    schedule_preview: Some(crate::scheduled_tasks::preview(
                        &rrule,
                        &time_zone,
                        pattern,
                        now(),
                    )?),
                    ..Default::default()
                });
            }
            Operation::Delete { id, revision } => {
                let task = self.scheduled_tasks.checked(&id, revision)?;
                self.scheduled_tasks.delete(&task)?;
            }
            Operation::RunNow { id, revision } => {
                let task = self.scheduled_tasks.checked(&id, revision)?;
                let mut active = task.clone();
                active.status = Status::Active;
                self.validate_scheduled_target(&active)?;
                self.check_handover(&task)?;
                if self.scheduled_tasks.runs()?.iter().any(|r| {
                    r.task_id == id
                        && matches!(
                            r.phase,
                            Phase::Waiting | Phase::Dispatching | Phase::Running
                        )
                }) {
                    return Err(RuntimeError::invalid("此任务已有待执行或正在运行的触发"));
                }
                self.scheduled_tasks
                    .claim(&task, now(), task.next_run_at_unix_ms)?;
                self.schedule_next_check = std::time::Instant::now();
            }
            Operation::PreviewImport {
                source_directory,
                runtime_id,
                time_zone,
            } => {
                self.require_codex_import_target(&runtime_id)?;
                let (preview, issues) = crate::scheduled_tasks::preview_import(
                    Path::new(&source_directory),
                    &runtime_id,
                    &time_zone,
                )?;
                let mut state = self.scheduled_tasks.state()?;
                state.import_preview = preview;
                state.import_issues = issues;
                return Ok(state);
            }
            Operation::Import {
                source_path,
                fingerprint,
                runtime_id,
                time_zone,
                expected_revision,
            } => {
                self.require_codex_import_target(&runtime_id)?;
                let mut task = crate::scheduled_tasks::import_one(
                    Path::new(&source_path),
                    &runtime_id,
                    &time_zone,
                )?;
                if task.source.as_ref().expect("import source").fingerprint != fingerprint {
                    return Err(RuntimeError::invalid("来源已改变，请重新预览"));
                }
                let source_path = &task.source.as_ref().expect("source").path;
                if let Some(current) = self
                    .scheduled_tasks
                    .tasks()?
                    .into_iter()
                    .find(|t| t.source.as_ref().is_some_and(|s| &s.path == source_path))
                {
                    if expected_revision != Some(current.revision) {
                        return Err(RuntimeError::invalid(
                            "Velune 任务已改变，请重新预览并核对覆盖差异",
                        ));
                    }
                    task.id = current.id;
                    task.revision = current.revision + 1;
                } else {
                    if expected_revision.is_some() {
                        return Err(RuntimeError::invalid("原 Velune 任务已删除，请重新预览"));
                    }
                    if self.scheduled_tasks.tasks()?.len() >= 256 {
                        return Err(RuntimeError::invalid("最多保存 256 个定时任务"));
                    }
                    task.id = crate::scheduled_tasks::id()?;
                    task.revision = 1;
                }
                crate::scheduled_tasks::anchor(&mut task)?;
                self.scheduled_tasks.put_task(&task)?;
            }
            Operation::ConfirmHandover { id, revision } => {
                let mut task = self.scheduled_tasks.checked(&id, revision)?;
                verify_source(&task)?;
                task.source
                    .as_mut()
                    .ok_or_else(|| RuntimeError::invalid("此任务没有待交接的来源"))?
                    .handover_confirmed = true;
                task.status = Status::Paused;
                task.revision += 1;
                task.issue =
                    Some("原宿主停用与在途运行处理已由用户确认；核对执行配置后可启用".into());
                self.scheduled_tasks.put_task(&task)?;
            }
        }
        let mut state = self.scheduled_tasks.state()?;
        state.availability = state
            .tasks
            .iter()
            .map(|task| {
                let mut active = task.clone();
                active.status = Status::Active;
                let enable_issue = self
                    .validate_scheduled_target(&active)
                    .and_then(|_| self.check_handover(task))
                    .err()
                    .map(|error| crate::Failure::from(error).message);
                let run_issue = enable_issue.clone().or_else(|| {
                    state
                        .runs
                        .iter()
                        .any(|run| {
                            run.task_id == task.id
                                && matches!(
                                    run.phase,
                                    Phase::Waiting | Phase::Dispatching | Phase::Running
                                )
                        })
                        .then(|| "已有待执行或正在运行的触发".into())
                });
                crate::ScheduledTaskAvailability {
                    task_id: task.id.clone(),
                    enable_issue,
                    run_issue,
                }
            })
            .collect();
        Ok(state)
    }
    fn require_codex_import_target(&self, runtime_id: &str) -> Result<(), RuntimeError> {
        if !matches!(
            self.runtime_instance(runtime_id)?.type_id,
            RuntimeKind::Codex166
        ) {
            return Err(RuntimeError::invalid(
                "请先选择对应 CODEX_HOME 的 Codex 运行时",
            ));
        }
        Ok(())
    }
    fn validate_scheduled_target(&self, task: &ScheduledTask) -> Result<(), RuntimeError> {
        let runtime = self.runtime_instance(task.target.runtime_id())?;
        if task.status == Status::Active && !runtime.enabled {
            return Err(RuntimeError::invalid("请先启用目标运行时"));
        }
        self.validate_model_selection(runtime, task.model.as_ref())?;
        if let Some(effort) = &task.effort {
            self.validate_effort(runtime, task.model.as_ref(), effort)?;
        }
        if let Some(mode) = &task.permission_mode {
            self.validate_permission_mode(runtime, mode)?;
        }
        match &task.target {
            Target::NewConversation {
                working_directories,
                worktree,
                ..
            } => {
                if working_directories.len() > 32 || (*worktree && working_directories.is_empty()) {
                    return Err(RuntimeError::invalid(
                        "工作树任务需要至少一个 Git 工作目录；最多 32 个目录",
                    ));
                }
                let mut seen = std::collections::BTreeSet::new();
                for cwd in working_directories {
                    if !seen.insert(cwd) {
                        return Err(RuntimeError::invalid("工作目录不能重复"));
                    }
                    self.validate_runtime_cwd(runtime, Path::new(cwd))?;
                }
            }
            Target::ExistingConversation {
                conversation_id, ..
            } if conversation_id.is_empty() => return Err(RuntimeError::invalid("请选择已有会话")),
            _ => {}
        }
        Ok(())
    }
    fn check_handover(&self, task: &ScheduledTask) -> Result<(), RuntimeError> {
        verify_source(task)?;
        if task.source.as_ref().is_some_and(|s| !s.handover_confirmed) {
            return Err(RuntimeError::invalid(
                "请先确认原宿主已暂停且在途运行已处理；导入任务不会自动接管执行",
            ));
        }
        Ok(())
    }
    /// A turn owns this binding once; a later manual turn cannot settle it.
    pub(super) fn settle_scheduled_turn(&mut self, outcome: crate::TurnOutcome) -> bool {
        let Some((id, failures_only)) = self.session.scheduled_run.take() else {
            return false;
        };
        let recorded = (|| {
            let Some(mut run) = self
                .scheduled_tasks
                .runs()?
                .into_iter()
                .find(|run| run.id == id)
            else {
                return Ok::<_, RuntimeError>(());
            };
            run.turn_outcome = Some(outcome);
            run.phase = if self.session.executing() {
                Phase::Running
            } else if outcome == crate::TurnOutcome::Completed {
                Phase::Completed
            } else {
                Phase::Failed
            };
            if outcome == crate::TurnOutcome::Cancelled {
                run.detail = Some("运行已由用户停止".into());
            }
            self.scheduled_tasks.put_run(&run)
        })();
        if let Err(error) = recorded {
            tracing::warn!(event="scheduled_run_settlement_failed",detail=%error);
        }
        failures_only && outcome == crate::TurnOutcome::Completed
    }
    pub(super) fn advance_scheduled_tasks(&mut self) {
        if std::time::Instant::now() < self.schedule_next_check {
            return;
        }
        self.schedule_next_check = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let focus = self.session.key;
        if let Err(error) = self.sweep_scheduled_tasks() {
            tracing::warn!(target:"velune_application",event="scheduled_task_advance_failed",detail=%error);
        }
        self.focus_key(focus);
    }
    fn pause_schedule(
        &mut self,
        task: &ScheduledTask,
        detail: String,
        invalidate_handover: bool,
    ) -> Result<(), RuntimeError> {
        let mut task = task.clone();
        task.status = Status::NeedsConfiguration;
        task.revision += 1;
        task.next_run_at_unix_ms = None;
        task.issue = Some(detail);
        if invalidate_handover && let Some(source) = &mut task.source {
            source.handover_confirmed = false;
        }
        self.scheduled_tasks.put_task(&task)
    }
    fn sweep_scheduled_tasks(&mut self) -> Result<(), RuntimeError> {
        let mut runs = self.scheduled_tasks.runs()?;
        for run in &mut runs {
            if run.phase != Phase::Running {
                continue;
            }
            let Some(id) = &run.conversation_id else {
                continue;
            };
            let session = std::iter::once(&self.session)
                .chain(&self.parked)
                .find(|s| s.presents(id));
            let Some(session) = session else {
                run.phase = Phase::OutcomeUnknown;
                run.detail = Some("运行会话已关闭；请查看会话结果，未自动重发".into());
                self.scheduled_tasks.put_run(run)?;
                continue;
            };
            if session.executing()
                || session
                    .projected()
                    .is_some_and(|s| !s.pending_interactions.is_empty())
            {
                continue;
            }
            run.phase = match run.turn_outcome {
                Some(crate::TurnOutcome::Completed) => Phase::Completed,
                Some(_) => Phase::Failed,
                None => Phase::OutcomeUnknown,
            };
            self.scheduled_tasks.put_run(run)?;
        }
        let tasks = self.scheduled_tasks.tasks()?;
        for task in &tasks {
            if task.status != Status::Active {
                continue;
            }
            if let Err(error) = self.validate_scheduled_target(task) {
                self.pause_schedule(task, error.to_string(), false)?;
                continue;
            }
            if let Err(error) = self.check_handover(task) {
                self.pause_schedule(task, error.to_string(), true)?;
                continue;
            }
            if let Some(due) = task.next_run_at_unix_ms.filter(|due| *due <= now()) {
                // While awake, one pending invocation waits; subsequent ticks do
                // not pile up duplicate prompts behind a busy context.
                let next = match next_occurrence(task, now()) {
                    Ok(next) => next,
                    Err(e) => {
                        self.pause_schedule(task, e.to_string(), false)?;
                        continue;
                    }
                };
                self.scheduled_tasks.claim_due(task, due, next)?;
            }
        }
        // Dispatch at most one invocation per sweep so clients can observe and
        // approve work between launches. The session pool owns running turns.
        let mut waiting = self
            .scheduled_tasks
            .runs()?
            .into_iter()
            .filter(|r| r.phase == Phase::Waiting)
            .collect::<Vec<_>>();
        waiting.sort_by_key(|r| r.due_at_unix_ms);
        for mut run in waiting {
            if runs.iter().any(|r| {
                r.task_id == run.task_id && matches!(r.phase, Phase::Dispatching | Phase::Running)
            }) {
                continue;
            }
            let Some(task) = tasks
                .iter()
                .find(|t| t.id == run.task_id && t.revision == run.definition_revision)
            else {
                run.phase = Phase::Skipped;
                run.detail = Some("任务定义已修改或删除".into());
                self.scheduled_tasks.put_run(&run)?;
                continue;
            };
            if let Err(error) = self
                .validate_scheduled_target(task)
                .and_then(|_| self.check_handover(task))
            {
                run.phase = Phase::Failed;
                run.detail = Some(error.to_string());
                self.scheduled_tasks.put_run(&run)?;
                continue;
            }
            self.dispatch_scheduled_run(task, &mut run)?;
            break;
        }
        Ok(())
    }
    fn dispatch_scheduled_run(
        &mut self,
        task: &ScheduledTask,
        run: &mut ScheduledRun,
    ) -> Result<(), RuntimeError> {
        // Keep the run waiting rather than preparing it against credentials
        // that an interactive login is about to replace.
        if self.login_running() {
            return Ok(());
        }
        match &task.target {
            Target::ExistingConversation {
                runtime_id,
                conversation_id,
            } => {
                self.focus_conversation(conversation_id);
                if self.session.projected().is_none()
                    && let Err(error) = self.open_conversation(runtime_id, conversation_id)
                {
                    run.phase = Phase::Failed;
                    let detail = format!("无法载入目标会话，请核对目标和连接后重新启用：{error}");
                    run.detail = Some(detail.clone());
                    self.scheduled_tasks.put_run(run)?;
                    self.pause_schedule(task, detail, false)?;
                    return Ok(());
                }
                let Some(snapshot) = self.current_snapshot() else {
                    return Ok(());
                };
                if snapshot.actions.input_access == crate::conversation::InputAccess::ReadOnly {
                    run.phase = Phase::Failed;
                    let detail = "目标会话只可阅读；请选择可继续的会话".to_owned();
                    run.detail = Some(detail.clone());
                    self.scheduled_tasks.put_run(run)?;
                    self.pause_schedule(task, detail, false)?;
                    return Ok(());
                }
                if snapshot.actions.input_access != crate::conversation::InputAccess::Editable
                    || self.session.executing()
                    || !snapshot.pending_interactions.is_empty()
                {
                    run.detail = Some("等待会话空闲、审批结束或外部客户端释放控制权".into());
                    self.scheduled_tasks.put_run(run)?;
                    return Ok(());
                }
            }
            Target::NewConversation { .. } => self.focus_new(),
        }
        run.phase = Phase::Dispatching;
        run.detail = None;
        self.scheduled_tasks.put_run(run)?;
        if let Target::NewConversation {
            runtime_id,
            worktree,
            ..
        } = &task.target
        {
            let created = (|| {
                let cwd = if *worktree {
                    Some(self.schedule_worktree(runtime_id, run)?)
                } else {
                    run.cwd.clone()
                };
                run.cwd = cwd.clone();
                self.scheduled_tasks.put_run(run)?;
                self.create_conversation(runtime_id, cwd, task.model.clone())
            })();
            if let Err(error) = created {
                run.phase = Phase::Failed;
                let detail = error.to_string();
                run.detail = Some(detail.clone());
                self.scheduled_tasks.put_run(run)?;
                self.pause_schedule(task, detail, false)?;
                return Ok(());
            }
        }
        let snapshot = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("定时任务会话未创建"))?;
        run.conversation_id = Some(snapshot.conversation.id.clone());
        self.scheduled_tasks.put_run(run)?;
        let request = conversations::TurnRequest {
            source_runtime_id: snapshot.conversation.runtime_id.clone(),
            conversation_id: snapshot.conversation.id,
            runtime_id: task.target.runtime_id().into(),
            selection: task.model.clone(),
            turn: crate::ComposedTurn {
                text: task.prompt.clone(),
                attachments: Vec::new(),
                effort: task.effort.clone(),
                permission_mode: task.permission_mode.clone(),
            },
        };
        self.session.scheduled_run = Some((run.id.clone(), task.notify_only_failures));
        let sent = self.send_turn(request);
        if sent.is_err() {
            self.session.scheduled_run = None;
        }
        match sent {
            Ok(_) => {
                // A very short turn may already have settled inside send_turn.
                if let Some(settled) = self
                    .scheduled_tasks
                    .runs()?
                    .into_iter()
                    .find(|r| r.id == run.id)
                {
                    *run = settled;
                    if run.phase == Phase::Dispatching {
                        run.phase = Phase::Running;
                    }
                } else {
                    run.phase = Phase::Running;
                }
            }
            Err(RuntimeError::Contract(failure)) if failure.code == "control_held_elsewhere" => {
                run.phase = Phase::Waiting;
                run.detail = Some("等待外部客户端释放控制权；尚未发送".into());
            }
            Err(error) => {
                run.phase = Phase::OutcomeUnknown;
                run.detail = Some(format!("派发未能确认：{error}；请检查会话，未自动重发"));
            }
        }
        self.scheduled_tasks.put_run(run)
    }
    fn schedule_worktree(
        &self,
        runtime_id: &str,
        run: &ScheduledRun,
    ) -> Result<String, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        let (target, os_home) = self.runtime_location(runtime)?;
        let cwd = run
            .cwd
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("工作树任务缺少仓库目录"))?;
        // Worktrees are durable output and do not require a Harness agentDir.
        let root = if target.is_local() {
            self.options.home_directory.join("scheduled-worktrees")
        } else {
            PathBuf::from(
                runtime
                    .execution()?
                    .remote_resources_directory
                    .ok_or_else(|| {
                        RuntimeError::invalid("SSH 工作树需要配置执行设备上的资源目录")
                    })?,
            )
            .join("scheduled-worktrees")
        };
        let destination = root.join(&run.id);
        let mut command = velune_agent_runtime::process::command("git");
        command
            .args(["-C", cwd, "worktree", "add", "--detach"])
            .arg(&destination)
            .arg("HEAD");
        target
            .output(command, None, os_home.as_deref())
            .map_err(|e| RuntimeError::context("创建任务工作树", e))?;
        Ok(destination.to_string_lossy().into_owned())
    }
}
