//! Durable definitions and run ledger. The application-home lock elects the
//! only scheduler; a trigger is committed before any runtime can accept input.
use crate::{
    Error, ScheduledRun, ScheduledRunPhase, ScheduledTask, ScheduledTaskStatus, ScheduledTasksState,
};
use rusqlite::{Connection, params};
use std::path::Path;
mod import;
mod schedule;
pub(crate) use import::{import_one, preview_import, verify_source};
pub(crate) use schedule::preview;

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn db(error: impl std::fmt::Display) -> Error {
    Error::context("定时任务存储", error)
}
pub(crate) fn id() -> Result<String, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(db)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The crate enforces an iteration budget, including unsatisfiable rules. No
/// unchecked calendar iterator is used; a budget exhaustion is a visible issue.
pub(crate) fn next_occurrence(task: &ScheduledTask, after: i64) -> Result<Option<i64>, Error> {
    Ok(occurrences(&task.rrule, &task.time_zone, after, 1)?
        .into_iter()
        .next())
}

fn occurrences(raw: &str, time_zone: &str, after: i64, count: u16) -> Result<Vec<i64>, Error> {
    use chrono::{TimeZone, Utc};
    use rrule::RRuleSet;
    let zone = time_zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| Error::invalid("请选择有效的 IANA 时区，例如 Asia/Shanghai"))?;
    let at = Utc
        .timestamp_millis_opt(after)
        .single()
        .ok_or_else(|| Error::invalid("调度时间超出范围"))?
        .with_timezone(&zone);
    let raw = raw.trim().replace("\r\n", "\n");
    if raw.len() > 4096 {
        return Err(Error::invalid("定时规则过长"));
    }
    let rule = if raw.contains("DTSTART") {
        raw
    } else {
        let rule = raw.strip_prefix("RRULE:").unwrap_or(&raw);
        format!(
            "DTSTART;TZID={}:{}\nRRULE:{rule}",
            time_zone,
            at.format("%Y%m%dT%H%M%S")
        )
    };
    let at = at.with_timezone(&rrule::Tz::Tz(zone));
    let set = rule
        .parse::<RRuleSet>()
        .map_err(|e| Error::context("定时规则无效", e))?;
    if set.get_dt_start().timezone().name() != time_zone {
        return Err(Error::invalid(
            "规则 DTSTART 的时区与任务时区不一致，请一起修改",
        ));
    }
    let result = set.after(at + chrono::Duration::milliseconds(1)).all(count);
    // `limited` also reports reaching the requested result count, which is fine.
    if !result.dates.is_empty() {
        return Ok(result
            .dates
            .iter()
            .map(|date| date.timestamp_millis())
            .collect());
    }
    if result.limited {
        return Err(Error::invalid(
            "定时规则在有限搜索内没有下一次触发，请简化或修正规则",
        ));
    }
    Ok(vec![])
}

pub(crate) fn anchor(task: &mut ScheduledTask) -> Result<(), Error> {
    task.rrule = schedule::normalize(&task.rrule, &task.time_zone, now())?;
    Ok(())
}

pub(crate) struct Store {
    connection: Connection,
}
impl Store {
    pub(crate) fn open(home: &Path) -> Result<Self, Error> {
        let connection = Connection::open(home.join("scheduled-tasks.sqlite3")).map_err(db)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(db)?;
        connection.execute_batch("PRAGMA journal_mode=WAL;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, definition TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, due INTEGER NOT NULL, slot TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL, UNIQUE(task_id,due,slot));
            PRAGMA user_version=1;
            COMMIT;").map_err(db)?;
        let mut store = Self { connection };
        // A prior owner may have crossed the native acceptance boundary. Never
        // replay it, even when the process died before recording its reply.
        for mut run in store.runs()? {
            match run.phase {
                ScheduledRunPhase::Dispatching | ScheduledRunPhase::Running => {
                    run.phase = ScheduledRunPhase::OutcomeUnknown;
                    run.detail = Some("上次退出时运行尚未结算；请查看会话，未自动重发".into());
                }
                ScheduledRunPhase::Waiting => {
                    run.phase = ScheduledRunPhase::Skipped;
                    run.detail = Some("应用退出期间的待执行触发已跳过".into());
                }
                _ => continue,
            }
            store.put_run(&run)?;
        }
        for mut task in store.tasks()? {
            if task.status == ScheduledTaskStatus::Active {
                match next_occurrence(&task, now()) {
                    Ok(next) => task.next_run_at_unix_ms = next,
                    Err(e) => {
                        task.status = ScheduledTaskStatus::NeedsConfiguration;
                        task.revision += 1;
                        task.issue = Some(e.to_string());
                        task.next_run_at_unix_ms = None;
                    }
                }
                store.put_task(&task)?;
            }
        }
        Ok(store)
    }
    pub(crate) fn tasks(&self) -> Result<Vec<ScheduledTask>, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT definition FROM tasks ORDER BY id")
            .map_err(db)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db)?
            .map(|row| Ok(serde_json::from_str(&row.map_err(db)?)?))
            .collect()
    }
    pub(crate) fn runs(&self) -> Result<Vec<ScheduledRun>, Error> {
        let mut statement = self.connection.prepare("SELECT body FROM runs WHERE phase IN ('Waiting','Dispatching','Running') OR id IN (SELECT id FROM runs ORDER BY due DESC,id DESC LIMIT 200) ORDER BY due DESC,id DESC").map_err(db)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db)?
            .map(|row| Ok(serde_json::from_str(&row.map_err(db)?)?))
            .collect()
    }
    pub(crate) fn state(&self) -> Result<ScheduledTasksState, Error> {
        Ok(ScheduledTasksState {
            tasks: self.tasks()?,
            runs: self.runs()?.into_iter().take(200).collect(),
            ..Default::default()
        })
    }
    pub(crate) fn checked(&self, id: &str, revision: u64) -> Result<ScheduledTask, Error> {
        self.tasks()?
            .into_iter()
            .find(|task| task.id == id && task.revision == revision)
            .ok_or_else(|| Error::invalid("任务已修改或删除，请刷新后重试"))
    }
    pub(crate) fn save(&mut self, mut task: ScheduledTask) -> Result<(), Error> {
        if task.name.trim().is_empty()
            || task.prompt.trim().is_empty()
            || task.name.len() > 512
            || task.prompt.len() > 128 * 1024
        {
            return Err(Error::invalid("请填写任务名称和指令（指令最多 128 KiB）"));
        }
        if task.id.is_empty() {
            if self.tasks()?.len() >= 256 {
                return Err(Error::invalid("最多保存 256 个定时任务"));
            }
            task.id = id()?;
            task.revision = 1;
            task.source = None;
        } else {
            let current = self.checked(&task.id, task.revision)?;
            task.source = current.source;
            task.revision += 1;
        }
        task.issue = None;
        anchor(&mut task)?;
        let next = next_occurrence(&task, now())?;
        if task.status == ScheduledTaskStatus::Active {
            verify_source(&task)?;
            if task.source.as_ref().is_some_and(|s| !s.handover_confirmed) {
                return Err(Error::invalid(
                    "请先在原宿主暂停任务、处理在途运行，再确认交接",
                ));
            }
            if next.is_none() {
                return Err(Error::invalid("此规则已无未来触发时间"));
            }
        }
        task.next_run_at_unix_ms = if task.status == ScheduledTaskStatus::Active {
            next
        } else {
            None
        };
        self.put_task(&task)
    }
    pub(crate) fn put_task(&mut self, task: &ScheduledTask) -> Result<(), Error> {
        self.connection.execute("INSERT INTO tasks VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET definition=excluded.definition", params![task.id, serde_json::to_string(task)?]).map_err(db)?;
        Ok(())
    }
    pub(crate) fn pause(&mut self, mut task: ScheduledTask) -> Result<(), Error> {
        task.status = ScheduledTaskStatus::Paused;
        task.next_run_at_unix_ms = None;
        task.revision += 1;
        self.put_task(&task)
    }
    pub(crate) fn delete(&mut self, task: &ScheduledTask) -> Result<(), Error> {
        self.connection
            .execute("DELETE FROM tasks WHERE id=?1", [&task.id])
            .map_err(db)?;
        for mut run in self
            .runs()?
            .into_iter()
            .filter(|r| r.task_id == task.id && r.phase == ScheduledRunPhase::Waiting)
        {
            run.phase = ScheduledRunPhase::Skipped;
            run.detail = Some("任务已删除".into());
            self.put_run(&run)?;
        }
        Ok(())
    }
    pub(crate) fn put_run(&mut self, run: &ScheduledRun) -> Result<(), Error> {
        self.connection
            .execute(
                "UPDATE runs SET phase=?2,body=?3 WHERE id=?1",
                params![
                    run.id,
                    format!("{:?}", run.phase),
                    serde_json::to_string(run)?
                ],
            )
            .map_err(db)?;
        Ok(())
    }
    /// Keep one future invocation behind the invocation already executing. A
    /// multi-directory invocation can still have waiting slots of its own.
    pub(crate) fn claim_due(
        &mut self,
        task: &ScheduledTask,
        due: i64,
        next: Option<i64>,
    ) -> Result<(), Error> {
        let runs = self.runs()?;
        let executing = runs
            .iter()
            .filter(|run| {
                run.task_id == task.id
                    && matches!(
                        run.phase,
                        ScheduledRunPhase::Dispatching | ScheduledRunPhase::Running
                    )
            })
            .map(|run| run.due_at_unix_ms)
            .collect::<std::collections::BTreeSet<_>>();
        let queued = runs.iter().any(|run| {
            run.task_id == task.id
                && run.phase == ScheduledRunPhase::Waiting
                && !executing.contains(&run.due_at_unix_ms)
        });
        if queued {
            let mut task = task.clone();
            task.next_run_at_unix_ms = next;
            self.put_task(&task)
        } else {
            self.claim(task, due, next)
        }
    }
    /// Transactionally claim every project's slot and advance the cursor.
    pub(crate) fn claim(
        &mut self,
        task: &ScheduledTask,
        due: i64,
        next: Option<i64>,
    ) -> Result<(), Error> {
        let cwds = match &task.target {
            crate::ScheduledTarget::NewConversation {
                working_directories,
                ..
            } if !working_directories.is_empty() => {
                working_directories.iter().cloned().map(Some).collect()
            }
            _ => vec![None],
        };
        let transaction = self.connection.transaction().map_err(db)?;
        for cwd in cwds {
            let run = ScheduledRun {
                id: id()?,
                task_id: task.id.clone(),
                definition_revision: task.revision,
                due_at_unix_ms: due,
                phase: ScheduledRunPhase::Waiting,
                turn_outcome: None,
                cwd: cwd.clone(),
                runtime_id: task.target.runtime_id().into(),
                conversation_id: None,
                detail: None,
            };
            transaction
                .execute(
                    "INSERT OR IGNORE INTO runs VALUES(?1,?2,?3,?4,'Waiting',?5)",
                    params![
                        run.id,
                        task.id,
                        due,
                        cwd.unwrap_or_default(),
                        serde_json::to_string(&run)?
                    ],
                )
                .map_err(db)?;
        }
        let mut task = task.clone();
        task.next_run_at_unix_ms = next;
        transaction
            .execute(
                "UPDATE tasks SET definition=?2 WHERE id=?1",
                params![task.id, serde_json::to_string(&task)?],
            )
            .map_err(db)?;
        transaction.commit().map_err(db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScheduledTarget;
    fn task() -> ScheduledTask {
        ScheduledTask { id:String::new(),revision:0,name:"合成任务".into(),prompt:"合成指令，不调用模型".into(),
            rrule:"DTSTART;TZID=Asia/Shanghai:20260101T090000\nRRULE:FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=0".into(),time_zone:"Asia/Shanghai".into(),
            status:ScheduledTaskStatus::Active,target:ScheduledTarget::NewConversation {runtime_id:"synthetic".into(),working_directories:vec![],worktree:false},model:None,effort:None,permission_mode:None,notify_only_failures:false,next_run_at_unix_ms:None,source:None,issue:None }
    }
    fn timestamp(date: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(date)
            .unwrap()
            .timestamp_millis()
    }
    fn home() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("velune-schedules-{}", id().unwrap()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn calendar_respects_zone_and_strict_next() {
        let task = task();
        assert_eq!(
            next_occurrence(&task, timestamp("2026-11-02T09:00:00+08:00")).unwrap(),
            Some(timestamp("2026-11-03T09:00:00+08:00"))
        );
        let mut task = task;
        task.time_zone = "America/New_York".into();
        task.rrule = "DTSTART;TZID=America/New_York:20260101T090000\nRRULE:FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=0".into();
        let spring = timestamp("2026-03-07T09:00:00-05:00");
        assert_eq!(
            next_occurrence(&task, spring).unwrap(),
            Some(spring + 23 * 3600 * 1000)
        );
        let fall = timestamp("2026-10-31T09:00:00-04:00");
        assert_eq!(
            next_occurrence(&task, fall).unwrap(),
            Some(fall + 25 * 3600 * 1000)
        );
    }
    #[test]
    fn impossible_rules_are_bounded_and_visible() {
        let mut task = task();
        task.rrule =
            "DTSTART;TZID=Asia/Shanghai:20260101T090000\nRRULE:FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30"
                .into();
        let start = std::time::Instant::now();
        assert!(matches!(next_occurrence(&task, now()), Err(_) | Ok(None)));
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        task.time_zone = "not/a-zone".into();
        assert!(next_occurrence(&task, now()).is_err());
    }
    #[test]
    fn claim_is_unique_and_restart_never_replays_uncertain_input() {
        let home = home();
        let mut store = Store::open(&home).unwrap();
        store.save(task()).unwrap();
        let task = store.tasks().unwrap().remove(0);
        let due = now() - 10000;
        store.claim(&task, due, Some(due + 1000)).unwrap();
        store.claim(&task, due, Some(due + 1000)).unwrap();
        assert_eq!(store.runs().unwrap().len(), 1);
        let mut run = store.runs().unwrap().remove(0);
        run.phase = ScheduledRunPhase::Dispatching;
        store.put_run(&run).unwrap();
        drop(store);
        let store = Store::open(&home).unwrap();
        assert_eq!(
            store.runs().unwrap()[0].phase,
            ScheduledRunPhase::OutcomeUnknown
        );
        let restored = &store.tasks().unwrap()[0];
        assert_eq!(restored.status, ScheduledTaskStatus::Active);
        assert!(restored.next_run_at_unix_ms.unwrap() > now());
        drop(store);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn running_invocation_keeps_one_pending_trigger_across_project_slots() {
        let home = home();
        let mut store = Store::open(&home).unwrap();
        let mut definition = task();
        definition.target = ScheduledTarget::NewConversation {
            runtime_id: "synthetic".into(),
            working_directories: vec!["/synthetic/a".into(), "/synthetic/b".into()],
            worktree: false,
        };
        store.save(definition).unwrap();
        let task = store.tasks().unwrap().remove(0);
        store.claim_due(&task, 1000, Some(2000)).unwrap();
        let mut running = store.runs().unwrap().remove(0);
        running.phase = ScheduledRunPhase::Running;
        store.put_run(&running).unwrap();
        store.claim_due(&task, 2000, Some(3000)).unwrap();
        assert_eq!(
            store.runs().unwrap().len(),
            4,
            "one future trigger must survive while the previous trigger runs"
        );
        store.claim_due(&task, 3000, Some(4000)).unwrap();
        assert_eq!(
            store.runs().unwrap().len(),
            4,
            "later ticks must not accumulate prompts"
        );
        assert_eq!(store.tasks().unwrap()[0].next_run_at_unix_ms, Some(4000));
        for mut run in store.runs().unwrap() {
            if run.due_at_unix_ms == 1000 {
                run.phase = ScheduledRunPhase::Completed;
                store.put_run(&run).unwrap();
            }
        }
        let mut next_running = store
            .runs()
            .unwrap()
            .into_iter()
            .find(|run| run.due_at_unix_ms == 2000)
            .unwrap();
        next_running.phase = ScheduledRunPhase::Running;
        store.put_run(&next_running).unwrap();
        store.claim_due(&task, 4000, Some(5000)).unwrap();
        assert_eq!(
            store.runs().unwrap().len(),
            6,
            "finishing the first invocation makes room for one new pending trigger"
        );
        drop(store);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn busy_waiting_invocation_does_not_accumulate_future_triggers() {
        let home = home();
        let mut store = Store::open(&home).unwrap();
        store.save(task()).unwrap();
        let task = store.tasks().unwrap().remove(0);
        store.claim_due(&task, 1000, Some(2000)).unwrap();
        store.claim_due(&task, 2000, Some(3000)).unwrap();
        assert_eq!(store.runs().unwrap().len(), 1);
        assert_eq!(store.runs().unwrap()[0].due_at_unix_ms, 1000);
        assert_eq!(store.tasks().unwrap()[0].next_run_at_unix_ms, Some(3000));
        drop(store);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn stale_definition_cannot_overwrite_and_waiting_skips_on_reopen() {
        let home = home();
        let mut store = Store::open(&home).unwrap();
        store.save(task()).unwrap();
        let task = store.tasks().unwrap().remove(0);
        store.claim(&task, now(), None).unwrap();
        store.save(task.clone()).unwrap();
        assert!(store.save(task).is_err());
        drop(store);
        let store = Store::open(&home).unwrap();
        assert_eq!(store.runs().unwrap()[0].phase, ScheduledRunPhase::Skipped);
        drop(store);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn pause_does_not_require_a_valid_execution_definition() {
        let home = home();
        let mut store = Store::open(&home).unwrap();
        store.save(task()).unwrap();
        let mut broken = store.tasks().unwrap().remove(0);
        broken.rrule = "invalid".into();
        broken.model = Some(crate::ModelSelection::Managed {
            model_record_key: "deleted-model".into(),
        });
        store.put_task(&broken).unwrap();
        store.pause(broken.clone()).unwrap();
        let paused = store.tasks().unwrap().remove(0);
        assert_eq!(paused.status, ScheduledTaskStatus::Paused);
        assert_eq!(paused.revision, broken.revision + 1);
        assert_eq!(paused.next_run_at_unix_ms, None);
        assert!(store.checked(&broken.id, broken.revision).is_err());
        drop(store);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn source_activation_and_mutation_require_handover() {
        let home = home();
        let directory = home.join("automations/task");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("automation.toml");
        let text = "version=1\nid='synthetic'\nname='合成导入'\nprompt='不执行模型请求'\nstatus='ACTIVE'\nkind='heartbeat'\ntarget_thread_id='synthetic-thread'\nrrule='FREQ=HOURLY;INTERVAL=1'\ncreated_at=0\nupdated_at=0\n";
        std::fs::write(&path, text).unwrap();
        let (preview, issues) = preview_import(&home, "synthetic", "Asia/Shanghai").unwrap();
        assert_eq!(preview.len(), 1);
        assert!(issues.is_empty());
        let task = &preview[0];
        assert_eq!(task.status, ScheduledTaskStatus::Paused);
        assert!(!task.source.as_ref().unwrap().handover_confirmed);
        assert!(verify_source(task).is_err());
        std::fs::write(&path, text.replace("ACTIVE", "PAUSED")).unwrap();
        assert!(verify_source(task).is_ok());
        std::fs::write(
            &path,
            text.replace("ACTIVE", "PAUSED")
                .replace("不执行模型请求", "改变的指令"),
        )
        .unwrap();
        assert!(verify_source(task).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(test)]
mod source_database_tests {
    use super::*;
    #[test]
    fn file_pause_does_not_hide_running_old_host() {
        let home = std::env::temp_dir().join(format!("velune-host-state-{}", id().unwrap()));
        let source_dir = home.join("automations/task");
        std::fs::create_dir_all(&source_dir).unwrap();
        let path = source_dir.join("automation.toml");
        std::fs::write(&path,"version=1\nid='synthetic'\nname='合成任务'\nprompt='不执行模型'\nstatus='PAUSED'\nrrule='FREQ=DAILY'\n").unwrap();
        std::fs::create_dir(home.join("sqlite")).unwrap();
        let connection = Connection::open(home.join("sqlite/codex.db")).unwrap();
        connection.execute_batch("CREATE TABLE automations(id TEXT,status TEXT,next_run_at INTEGER);CREATE TABLE automation_runs(automation_id TEXT,status TEXT);INSERT INTO automations VALUES('synthetic','ACTIVE',NULL);").unwrap();
        let task = import_one(&path, "synthetic", "Asia/Shanghai").unwrap();
        assert!(verify_source(&task).is_err());
        connection.execute_batch("UPDATE automations SET status='PAUSED';INSERT INTO automation_runs VALUES('synthetic','IN_PROGRESS');").unwrap();
        assert!(verify_source(&task).is_err());
        connection
            .execute_batch("UPDATE automation_runs SET status='PENDING_REVIEW';")
            .unwrap();
        assert!(verify_source(&task).is_ok());
        drop(connection);
        std::fs::remove_dir_all(home).unwrap();
    }
}
