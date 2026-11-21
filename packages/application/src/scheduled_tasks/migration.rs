//! This store owns its SQLite version and the definition/run records it rewrites.
use crate::{Error, ScheduledRun, ScheduledTask};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, backup::Backup};
use std::{fs, path::Path, time::Duration};

const VERSION: i64 = 1;
const CREATE_TASKS: &str = "CREATE TABLE tasks(id TEXT PRIMARY KEY, definition TEXT NOT NULL)";
const CREATE_RUNS: &str = "CREATE TABLE runs(id TEXT PRIMARY KEY, task_id TEXT NOT NULL, due INTEGER NOT NULL, slot TEXT NOT NULL, phase TEXT NOT NULL, body TEXT NOT NULL, UNIQUE(task_id,due,slot))";
const BACKUP_NAME: &str = "scheduled-tasks.before-beta-migration.sqlite3";
const TASK_ROWS: &str = "SELECT id, definition FROM tasks ORDER BY id";
const RUN_ROWS: &str = "SELECT id, task_id, due, slot, phase, body FROM runs ORDER BY id";

#[derive(PartialEq, Eq)]
enum Schema {
    Empty,
    Unversioned,
    Current,
}

/// The application holds the state-directory lock through preparation and use.
pub(super) fn prepare(db: &mut Connection, home: &Path) -> Result<(), Error> {
    let schema = read_schema(db)?;
    if schema != Schema::Empty {
        validate_rows(db)?;
    }
    match schema {
        Schema::Current => return Ok(()),
        Schema::Unversioned => ensure_backup(db, home)?,
        Schema::Empty => {}
    }

    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(super::db)?;
    if schema == Schema::Empty {
        transaction
            .execute_batch(&format!("{CREATE_TASKS}; {CREATE_RUNS};"))
            .map_err(super::db)?;
    }
    transaction
        .pragma_update(None, "user_version", VERSION)
        .map_err(super::db)?;
    transaction.commit().map_err(super::db)
}

fn read_schema(db: &Connection) -> Result<Schema, Error> {
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(super::db)?;
    if version != 0 && version != VERSION {
        return Err(Error::Invalid(format!(
            "定时任务数据库 schema {version} 不受支持；原数据库保留"
        )));
    }
    // SQLite stores the known writer's CREATE TABLE SQL without IF NOT EXISTS.
    // Include user-created indexes, views and triggers to reject foreign layouts.
    let mut statement = db
        .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT GLOB 'sqlite_*' ORDER BY name")
        .map_err(super::db)?;
    let definitions = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(super::db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::db)?;
    match (version, definitions.as_slice()) {
        (0, []) => Ok(Schema::Empty),
        (_, [runs, tasks]) if runs == CREATE_RUNS && tasks == CREATE_TASKS => Ok(if version == 0 {
            Schema::Unversioned
        } else {
            Schema::Current
        }),
        _ => Err(Error::invalid(
            "定时任务数据库表结构不符合已知格式；原数据库保留",
        )),
    }
}

fn validate_rows(db: &Connection) -> Result<(), Error> {
    let mut tasks = db.prepare(TASK_ROWS).map_err(super::db)?;
    let mut tasks = tasks.query([]).map_err(super::db)?;
    while let Some(row) = tasks.next().map_err(super::db)? {
        let id: String = row.get(0).map_err(super::db)?;
        let definition: String = row.get(1).map_err(super::db)?;
        let task: ScheduledTask = serde_json::from_str(&definition)
            .map_err(|e| Error::context("定时任务定义含未知或无效字段；原数据库保留", e))?;
        if task.id != id {
            return Err(Error::invalid("定时任务行身份与定义不一致；原数据库保留"));
        }
    }
    let mut runs = db.prepare(RUN_ROWS).map_err(super::db)?;
    let mut runs = runs.query([]).map_err(super::db)?;
    while let Some(row) = runs.next().map_err(super::db)? {
        let id: String = row.get(0).map_err(super::db)?;
        let task_id: String = row.get(1).map_err(super::db)?;
        let due: i64 = row.get(2).map_err(super::db)?;
        // The slot identifies the original project. A worktree run may later
        // record a different execution directory in its body.
        let _: String = row.get(3).map_err(super::db)?;
        let phase: String = row.get(4).map_err(super::db)?;
        let body: String = row.get(5).map_err(super::db)?;
        let run: ScheduledRun = serde_json::from_str(&body)
            .map_err(|e| Error::context("定时任务运行记录含未知或无效字段；原数据库保留", e))?;
        if run.id != id
            || run.task_id != task_id
            || run.due_at_unix_ms != due
            || format!("{:?}", run.phase) != phase
        {
            return Err(Error::invalid("定时任务运行行与记录不一致；原数据库保留"));
        }
    }
    Ok(())
}

fn ensure_backup(source: &Connection, home: &Path) -> Result<(), Error> {
    let backup_path = home.join(BACKUP_NAME);
    match fs::symlink_metadata(&backup_path) {
        Ok(metadata) if metadata.is_file() => verify_backup(source, &backup_path)?,
        Ok(_) => return Err(Error::invalid("定时任务恢复副本不是普通文件；原数据库保留")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            publish_backup(source, home, &backup_path)?
        }
        Err(e) => return Err(e.into()),
    }
    #[cfg(unix)]
    fs::File::open(home)?.sync_all()?;
    Ok(())
}

fn publish_backup(source: &Connection, home: &Path, backup_path: &Path) -> Result<(), Error> {
    // A failed or interrupted copy stays in a private staging directory. Only
    // the validated, standalone database gets the recovery name.
    let staging = home.join(format!(".scheduled-tasks-backup-{}", super::id()?));
    let mut directory = fs::DirBuilder::new();
    let mut file = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        directory.mode(0o700);
        file.mode(0o600);
    }
    directory.recursive(false).create(&staging)?;
    let temporary = staging.join(BACKUP_NAME);
    let result = (|| {
        let file = file
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut backup = Connection::open(&temporary).map_err(super::db)?;
        Backup::new(source, &mut backup)
            .and_then(|copy| copy.run_to_completion(5, Duration::from_millis(50), None))
            .map_err(|e| Error::context("定时任务一致性备份失败；原数据库保留", e))?;
        backup
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(super::db)?;
        backup.close().map_err(|(_, e)| super::db(e))?;
        file.sync_all()?;
        drop(file);
        verify_backup(source, &temporary)?;
        fs::rename(&temporary, backup_path)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(staging);
    result
}

fn verify_backup(source: &Connection, path: &Path) -> Result<(), Error> {
    let backup = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Error::context("定时任务恢复副本无法读取；原数据库保留", e))?;
    if read_schema(&backup)? != Schema::Unversioned
        || !same_rows(source, &backup, TASK_ROWS).map_err(super::db)?
        || !same_rows(source, &backup, RUN_ROWS).map_err(super::db)?
    {
        return Err(Error::invalid(
            "定时任务恢复副本与原数据库不一致；原数据库保留",
        ));
    }
    Ok(())
}

fn same_rows(source: &Connection, backup: &Connection, sql: &str) -> rusqlite::Result<bool> {
    let mut original = source.prepare(sql)?;
    let mut copy = backup.prepare(sql)?;
    let columns = original.column_count();
    let mut original = original.query([])?;
    let mut copy = copy.query([])?;
    loop {
        match (original.next()?, copy.next()?) {
            (Some(original), Some(copy)) => {
                for column in 0..columns {
                    if original.get_ref(column)? != copy.get_ref(column)? {
                        return Ok(false);
                    }
                }
            }
            (None, None) => return Ok(true),
            _ => return Ok(false),
        }
    }
}
