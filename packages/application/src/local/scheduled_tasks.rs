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
        self.scheduled_tasks
            .note_turn_result(id, outcome, self.session.executing());
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
        self.scheduled_tasks.retry_turn_results();
        let mut runs = self.scheduled_tasks.runs()?;
        for run in &mut runs {
            if run.phase != Phase::Running || self.scheduled_tasks.has_pending_turn_result(&run.id)
            {
                continue;
            }
            let Some(id) = &run.conversation_id else {
                continue;
            };
            let session = std::iter::once(&self.session)
                .chain(&self.parked)
                .find(|s| s.presents(id));
            if session.is_some_and(|session| {
                session.executing()
                    || session.projected().is_some_and(|snapshot| {
                        snapshot
                            .pending_interactions
                            .iter()
                            .any(|interaction| interaction.blocks_turn())
                    })
            }) {
                continue;
            }
            let unknown_detail = session
                .is_none()
                .then_some("运行会话已关闭；请查看会话结果，未自动重发");
            self.scheduled_tasks
                .settle_inactive_run(run, unknown_detail)?;
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
            if let Target::ExistingConversation {
                conversation_id, ..
            } = &task.target
                && self.scheduled_handoff_blocks_dispatch(conversation_id)
            {
                let detail = "等待用户恢复 Velune 会话控制";
                if run.detail.as_deref() != Some(detail) {
                    run.detail = Some(detail.into());
                    self.scheduled_tasks.put_run(&run)?;
                }
                continue;
            }
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
                if self.check_handoff_input(&snapshot).is_err() {
                    let detail = "等待用户恢复 Velune 会话控制";
                    if run.detail.as_deref() != Some(detail) {
                        run.detail = Some(detail.into());
                        self.scheduled_tasks.put_run(run)?;
                    }
                    return Ok(());
                }
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
                    || snapshot
                        .pending_interactions
                        .iter()
                        .any(|interaction| interaction.blocks_turn())
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
            submission_id: None,
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
            Ok(_) => run.phase = Phase::Running,
            Err(RuntimeError::Contract(failure)) if failure.code == "control_held_elsewhere" => {
                run.phase = Phase::Waiting;
                run.detail = Some("等待外部客户端释放控制权；尚未发送".into());
            }
            Err(error) => {
                run.phase = Phase::OutcomeUnknown;
                run.detail = Some(format!("派发未能确认：{error}；请检查会话，未自动重发"));
            }
        }
        self.scheduled_tasks.put_dispatch_result(run)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnOutcome;
    use velune_test_support::TempRoot;

    fn paused_schedule(core: &mut CoreRuntime) -> ScheduledTask {
        let task = ScheduledTask {
            id: String::new(),
            revision: 0,
            name: "Synthetic previously scheduled work".into(),
            prompt: "This fixture does not send input".into(),
            rrule: "FREQ=DAILY".into(),
            time_zone: "Asia/Shanghai".into(),
            status: Status::Paused,
            target: Target::ExistingConversation {
                runtime_id: "synthetic".into(),
                conversation_id: "synthetic:closed".into(),
            },
            model: None,
            effort: None,
            permission_mode: None,
            notify_only_failures: true,
            next_run_at_unix_ms: None,
            source: None,
            issue: None,
        };
        core.scheduled_tasks.save(task).unwrap();
        core.scheduled_tasks.tasks().unwrap().remove(0)
    }

    fn running_schedule(core: &mut CoreRuntime, task: &ScheduledTask, due: i64) -> ScheduledRun {
        core.scheduled_tasks.claim(task, due, None).unwrap();
        let mut run = core
            .scheduled_tasks
            .runs()
            .unwrap()
            .into_iter()
            .find(|run| run.due_at_unix_ms == due)
            .unwrap();
        run.phase = Phase::Running;
        run.conversation_id = Some("synthetic:closed".into());
        core.scheduled_tasks.put_run(&run).unwrap();
        run
    }

    #[test]
    fn known_scheduled_result_survives_a_failed_sqlite_write_without_rebinding_to_the_session() {
        let temp = TempRoot::new("scheduled-result-retry");
        let home = temp.dir("home");
        let mut application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: temp.dir("resources"),
        })
        .unwrap();
        let core = &mut application.runtime;
        let task = paused_schedule(core);
        let run = running_schedule(core, &task, now());

        // Reject the terminal write only. An inadvertent OutcomeUnknown write
        // would succeed, so the sweep assertion tests protection of the fact.
        let fault = rusqlite::Connection::open(home.join("scheduled-tasks.sqlite3")).unwrap();
        fault
            .execute_batch(
                "
            CREATE TRIGGER reject_completed BEFORE UPDATE ON runs
            WHEN NEW.phase = 'Completed'
            BEGIN SELECT RAISE(FAIL, 'synthetic write failure'); END;
        ",
            )
            .unwrap();
        core.session.scheduled_run = Some((run.id.clone(), task.notify_only_failures));
        assert!(core.settle_scheduled_turn(TurnOutcome::Completed));
        assert!(core.session.scheduled_run.is_none());
        assert!(
            !core.settle_scheduled_turn(TurnOutcome::Failed),
            "manual turn is not scheduled"
        );

        let mut dispatch_result = run.clone();
        dispatch_result.phase = Phase::OutcomeUnknown;
        core.scheduled_tasks
            .put_dispatch_result(&dispatch_result)
            .unwrap();
        core.sweep_scheduled_tasks().unwrap();
        let during = core.scheduled_tasks.run(&run.id).unwrap().unwrap();
        assert_eq!(during.phase, Phase::Running);
        assert_eq!(during.turn_outcome, None);

        fault
            .execute_batch("DROP TRIGGER reject_completed")
            .unwrap();
        core.sweep_scheduled_tasks().unwrap();
        let after = core.scheduled_tasks.run(&run.id).unwrap().unwrap();
        assert_eq!(after.phase, Phase::Completed);
        assert_eq!(after.turn_outcome, Some(TurnOutcome::Completed));
        assert_eq!(core.scheduled_tasks.runs().unwrap().len(), 1);
        drop(fault);
        drop(application);

        let reopened = crate::scheduled_tasks::Store::open(&home).unwrap();
        let restored = reopened.run(&run.id).unwrap().unwrap();
        assert_eq!(restored.phase, Phase::Completed);
        assert_eq!(restored.turn_outcome, Some(TurnOutcome::Completed));
    }

    #[test]
    fn known_running_results_settle_after_session_closure_or_restart() {
        let temp = TempRoot::new("scheduled-result-orphan");
        let home = temp.dir("home");
        let mut application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: temp.dir("resources"),
        })
        .unwrap();
        let core = &mut application.runtime;
        let task = paused_schedule(core);
        let expected = [
            (Some(TurnOutcome::Completed), Phase::Completed),
            (Some(TurnOutcome::Failed), Phase::Failed),
            (Some(TurnOutcome::Cancelled), Phase::Failed),
            (None, Phase::OutcomeUnknown),
        ];
        let mut after_restart = Vec::new();
        let mut after_sweep = Vec::new();
        for (index, (outcome, phase)) in expected.iter().cloned().enumerate() {
            let due = i64::try_from(index).unwrap() * 2;
            let run = running_schedule(core, &task, due);
            if let Some(outcome) = outcome {
                core.scheduled_tasks
                    .note_turn_result(run.id.clone(), outcome, true);
            }
            let stored = core.scheduled_tasks.run(&run.id).unwrap().unwrap();
            assert_eq!(stored.phase, Phase::Running);
            assert_eq!(stored.turn_outcome, outcome);
            after_sweep.push((run.id, phase, outcome));
        }
        core.sweep_scheduled_tasks().unwrap();
        for (id, phase, outcome) in &after_sweep {
            let stored = core.scheduled_tasks.run(id).unwrap().unwrap();
            assert_eq!((&stored.phase, stored.turn_outcome), (phase, *outcome));
        }
        for (index, (outcome, phase)) in expected.iter().cloned().enumerate() {
            let due = i64::try_from(index).unwrap() * 2 + 1;
            let run = running_schedule(core, &task, due);
            if let Some(outcome) = outcome {
                core.scheduled_tasks
                    .note_turn_result(run.id.clone(), outcome, true);
            }
            after_restart.push((run.id, phase, outcome));
        }
        drop(application);

        let reopened = crate::scheduled_tasks::Store::open(&home).unwrap();
        for (id, phase, outcome) in after_restart {
            let stored = reopened.run(&id).unwrap().unwrap();
            assert_eq!((stored.phase, stored.turn_outcome), (phase, outcome));
        }
        assert_eq!(reopened.runs().unwrap().len(), 8);
    }
}
