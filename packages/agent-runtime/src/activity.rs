//! Best-effort observation of native sessions on this machine, including
//! sessions that Harness processes outside Velune are running. The Harness
//! stays the source of truth: whether a session is running comes from the
//! Harness's own liveness record where it keeps one (Codex's thread writer
//! lock, Claude Code's session registry), and only Pi, which keeps none, is
//! judged from its processes. Session files are located and watched for
//! changes; history for display is still read through the Harness adapters.
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime},
};

/// Native session file layouts this module understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFormat {
    /// `sessions/YYYY/MM/DD/rollout-<date>-<thread id>.jsonl`; a thread
    /// continued in a later segment adds `rollout-<date>-<thread id>_<segment id>.jsonl`.
    Codex,
    /// `projects/<project>/<session id>.jsonl`
    ClaudeCode,
    /// `<session dir>/<project>/<file>.jsonl`; the native id is the path.
    Pi,
}

impl SessionFormat {
    pub fn for_runtime(kind: crate::version::RuntimeKind) -> Option<Self> {
        use crate::version::RuntimeKind;
        match kind {
            RuntimeKind::Codex166 => Some(Self::Codex),
            RuntimeKind::ClaudeCode => Some(Self::ClaudeCode),
            RuntimeKind::Pi => Some(Self::Pi),
            RuntimeKind::DeepSeek | RuntimeKind::Acp => None,
        }
    }

    /// Directory depth of session files below the session root.
    fn depth(self) -> usize {
        match self {
            Self::Codex => 4,
            Self::ClaudeCode | Self::Pi => 2,
        }
    }

    fn native_id(self, path: &Path) -> Option<String> {
        let stem = path.file_stem()?.to_str()?;
        match self {
            // The thread id is the UUID ending the rollout name, before any
            // segment id.
            Self::Codex => {
                let name = stem.strip_prefix("rollout-")?;
                let name = name.split_once('_').map_or(name, |(thread, _)| thread);
                (name.len() > 36).then(|| name[name.len() - 36..].to_owned())
            }
            Self::ClaudeCode => Some(stem.to_owned()),
            Self::Pi => path.to_str().map(str::to_owned),
        }
    }
}

/// Change marker of a session file; native writers only append or replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature {
    pub modified: SystemTime,
    pub len: u64,
}

pub fn signature(path: &Path) -> Option<Signature> {
    let metadata = fs::metadata(path).ok()?;
    Some(Signature {
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

fn pi_header(path: &Path) -> Option<Value> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?.take(65536))
        .read_line(&mut line)
        .ok()?;
    let value: Value = serde_json::from_str(&line).ok()?;
    (value["type"] == "session" && value["id"].as_str().is_some_and(|id| !id.is_empty()))
        .then_some(value)
}

fn pi_run_id(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 36
            && name.bytes().enumerate().all(|(index, value)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    value == b'-'
                } else {
                    value.is_ascii_hexdigit()
                }
            })
    })
}

fn pi_run_slot(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix("run-"))
        .is_some_and(|slot| {
            slot.parse::<u32>()
                .is_ok_and(|index| index.to_string() == slot)
        })
}

/// The parent path implied by a supported child layout, independently of its
/// header. Both facts must agree before admitting a nested native session.
fn pi_layout_parent(path: &Path) -> Option<PathBuf> {
    let source = |stem: &Path| {
        let mut file = stem.as_os_str().to_os_string();
        file.push(".jsonl");
        PathBuf::from(file)
    };
    let directory = path.parent()?;
    let run = directory.parent()?;
    if path.file_name()? == "session.jsonl"
        && pi_run_slot(directory.file_name()?)
        && pi_run_id(run.file_name()?)
    {
        return Some(source(run.parent()?));
    }
    if run.file_name()? == "forks" {
        return Some(source(run.parent()?));
    }
    None
}

fn pi_path_admitted(path: &Path, root: &Path, depth: usize) -> bool {
    if depth > 16 || !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return false;
    }
    let Ok(within) = path.strip_prefix(root) else {
        return false;
    };
    if within
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return false;
    }
    let mut directory = path.parent();
    while let Some(current) = directory.filter(|directory| *directory != root) {
        if !fs::symlink_metadata(current).is_ok_and(|metadata| metadata.is_dir()) {
            return false;
        }
        directory = current.parent();
    }
    if within.components().count() == 2 {
        return path
            .extension()
            .is_some_and(|extension| extension == "jsonl");
    }
    let Some(parent) = pi_layout_parent(path) else {
        return false;
    };
    pi_header(path).is_some_and(|header| {
        header["parentSession"].as_str().map(Path::new) == Some(parent.as_path())
    }) && pi_header(&parent).is_some()
        && pi_path_admitted(&parent, root, depth + 1)
}

/// Session files below `root`, skipping links and Claude Code sub-agent
/// transcripts, which are not conversations of their own.
fn session_files(format: SessionFormat, root: &Path, mut visit: impl FnMut(&Path, &fs::Metadata)) {
    fn walk(directory: &Path, depth: usize, visit: &mut dyn FnMut(&Path, &fs::Metadata)) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth > 1 && entry.file_name() != "subagents" {
                walk(&path, depth - 1, visit);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                && let Ok(metadata) = entry.metadata()
            {
                visit(&path, &metadata);
            }
        }
    }
    if format == SessionFormat::Pi {
        fn pi_walk(
            directory: &Path,
            workspace: bool,
            depth: usize,
            parent: Option<&Path>,
            modern: bool,
            visit: &mut dyn FnMut(&Path, &fs::Metadata),
        ) {
            if depth > 16 {
                return;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                return;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if workspace && kind.is_dir() {
                    pi_walk(&path, false, depth + 1, None, false, visit);
                }
                if !kind.is_file() || path.extension().is_none_or(|ext| ext != "jsonl") {
                    continue;
                }
                if modern && entry.file_name() != "session.jsonl" {
                    continue;
                }
                let header = pi_header(&path);
                if let Some(parent) = parent
                    && header.as_ref().is_none_or(|header| {
                        header["parentSession"].as_str().map(Path::new) != Some(parent)
                    })
                {
                    continue;
                }
                if let Ok(metadata) = entry.metadata() {
                    visit(&path, &metadata);
                }
                if header.is_none() {
                    continue;
                }
                let stem = path.with_extension("");
                if !fs::symlink_metadata(&stem).is_ok_and(|meta| meta.is_dir()) {
                    continue;
                }
                let forks = stem.join("forks");
                if fs::symlink_metadata(&forks).is_ok_and(|meta| meta.is_dir())
                    && let Ok(runs) = fs::read_dir(forks)
                {
                    for run in runs.flatten() {
                        if run.file_type().is_ok_and(|kind| kind.is_dir()) {
                            pi_walk(&run.path(), false, depth + 1, Some(&path), false, visit);
                        }
                    }
                }
                if let Ok(runs) = fs::read_dir(stem) {
                    for run in runs.flatten() {
                        if pi_run_id(&run.file_name())
                            && run.file_type().is_ok_and(|kind| kind.is_dir())
                            && let Ok(slots) = fs::read_dir(run.path())
                        {
                            for slot in slots.flatten() {
                                if pi_run_slot(&slot.file_name())
                                    && slot.file_type().is_ok_and(|kind| kind.is_dir())
                                {
                                    pi_walk(
                                        &slot.path(),
                                        false,
                                        depth + 1,
                                        Some(&path),
                                        true,
                                        visit,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        pi_walk(root, true, 0, None, false, &mut visit);
    } else {
        walk(root, format.depth(), &mut visit);
    }
}

/// Keep in `found` whichever of it and `path` was written last.
fn latest(found: &mut Option<(PathBuf, SystemTime)>, path: &Path, metadata: &fs::Metadata) {
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if found.as_ref().is_none_or(|(_, at)| modified > *at) {
        *found = Some((path.to_owned(), modified));
    }
}

/// A runtime's native sessions on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStore {
    pub format: SessionFormat,
    /// The Harness's own directory: `CODEX_HOME`, `CLAUDE_CONFIG_DIR` or the
    /// Pi agent directory.
    pub agent_dir: PathBuf,
    /// Root of the session files.
    pub sessions: PathBuf,
}

impl SessionStore {
    /// The session file of `native_id`: the latest written of its segments.
    pub fn locate(&self, native_id: &str) -> Option<PathBuf> {
        if self.format == SessionFormat::Pi {
            let path = Path::new(native_id);
            return pi_path_admitted(path, &self.sessions, 0).then(|| path.to_owned());
        }
        if self.format == SessionFormat::ClaudeCode
            && let Some((session, agent)) = native_id.split_once("/agent-")
        {
            if agent.is_empty()
                || !agent
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return None;
            }
            let parent = self.locate(session)?;
            let directory = parent.parent()?.join(session).join("subagents");
            let path = directory.join(format!("agent-{agent}.jsonl"));
            return (path.is_file()
                && !path.is_symlink()
                && !directory.is_symlink()
                && !directory.parent().is_some_and(|parent| parent.is_symlink()))
            .then_some(path);
        }
        let mut found: Option<(PathBuf, SystemTime)> = None;
        session_files(self.format, &self.sessions, |path, metadata| {
            if self.format.native_id(path).as_deref() == Some(native_id) {
                latest(&mut found, path, metadata);
            }
        });
        found.map(|(path, _)| path)
    }

    /// The session file of each native id: the latest written of its
    /// segments. Pi's native id is its path.
    pub fn paths(&self) -> HashMap<String, PathBuf> {
        let mut found = HashMap::<String, Option<(PathBuf, SystemTime)>>::new();
        session_files(self.format, &self.sessions, |path, metadata| {
            if let Some(id) = self.format.native_id(path) {
                latest(found.entry(id).or_default(), path, metadata);
            }
        });
        found
            .into_iter()
            .filter_map(|(id, found)| Some((id, found?.0)))
            .collect()
    }

    /// Native ids of the sessions that a Harness process is running a turn of.
    pub fn running(&self, processes: &mut Processes) -> Vec<String> {
        match self.format {
            SessionFormat::Codex => {
                let writers = self.codex_writers();
                if writers.is_empty() {
                    return Vec::new();
                }
                let mut segments = HashMap::<String, Option<(PathBuf, SystemTime)>>::new();
                session_files(self.format, &self.sessions, |path, metadata| {
                    if let Some(id) = self.format.native_id(path)
                        && writers.contains(&id)
                    {
                        latest(segments.entry(id).or_default(), path, metadata);
                    }
                });
                segments
                    .into_iter()
                    .filter_map(|(id, found)| {
                        let (path, _) = found?;
                        unfinished(self.format, &path).then_some(id)
                    })
                    .collect()
            }
            SessionFormat::ClaudeCode => self.claude_busy(processes),
            SessionFormat::Pi => {
                let pis = processes.pi();
                if pis.is_empty() {
                    return Vec::new();
                }
                let mut running = Vec::new();
                session_files(self.format, &self.sessions, |path, metadata| {
                    if pi_running(path, metadata, pis)
                        && let Some(id) = self.format.native_id(path)
                    {
                        running.push(id);
                    }
                });
                running
            }
        }
    }

    /// Whether a Harness process is running a turn of the session at `path`.
    pub fn is_running(&self, native_id: &str, path: &Path, processes: &mut Processes) -> bool {
        match self.format {
            SessionFormat::Codex => {
                locked(&self.codex_lock(native_id)) && unfinished(self.format, path)
            }
            SessionFormat::ClaudeCode => {
                self.claude_busy(processes).iter().any(|id| id == native_id)
            }
            SessionFormat::Pi => {
                fs::metadata(path).is_ok_and(|metadata| pi_running(path, &metadata, processes.pi()))
            }
        }
    }

    fn codex_lock(&self, thread_id: &str) -> PathBuf {
        self.agent_dir
            .join("thread-writer-locks")
            .join(format!("{thread_id}.lock"))
    }

    /// Threads whose writer lock a Codex process holds. Codex holds
    /// `thread-writer-locks/<thread id>.lock` while a thread is loaded,
    /// running or not, and leaves the file behind when it exits.
    fn codex_writers(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.agent_dir.join("thread-writer-locks")) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let id = path.file_stem()?.to_str()?;
                (path
                    .extension()
                    .is_some_and(|extension| extension == "lock")
                    && !id.starts_with('.')
                    && locked(&path))
                .then(|| id.to_owned())
            })
            .collect()
    }

    /// Sessions that Claude Code's registry reports busy or waiting for the
    /// user. Claude Code 2.1.286 keeps `sessions/<pid>.json` per process with
    /// its session id, status (busy, shell, idle, waiting) and process start;
    /// a crashed process leaves its entry behind.
    fn claude_busy(&self, processes: &mut Processes) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.agent_dir.join("sessions")) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                // Only `<pid>.json`; the registry directory also holds keys.
                let path = entry.path();
                let pid: u32 = path
                    .file_name()?
                    .to_str()?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()?;
                let record: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                matches!(record["status"].as_str(), Some("busy" | "waiting")).then_some(())?;
                (record["pid"].as_u64() == Some(u64::from(pid))).then_some(())?;
                let session = record["sessionId"].as_str()?.to_owned();
                processes
                    .alive(pid, record["procStart"].as_str())
                    .then_some(session)
            })
            .collect()
    }
}

/// A live Pi process in the session's working directory that started before
/// the session file was last written, and a turn that has not ended. Pi
/// 1.0.2 keeps no liveness record and writes a file per entry without holding
/// it open.
fn pi_running(path: &Path, metadata: &fs::Metadata, pis: &[PiProcess]) -> bool {
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let candidates: Vec<&PiProcess> = pis
        .iter()
        .filter(|process| modified + Duration::from_secs(2) >= process.started)
        .collect();
    if candidates.is_empty() {
        return false;
    }
    let Some(cwd) = pi_session_cwd(path) else {
        return false;
    };
    candidates.iter().any(|process| process.cwd == cwd) && unfinished(SessionFormat::Pi, path)
}

/// The working directory in a Pi session file's header record.
fn pi_session_cwd(path: &Path) -> Option<PathBuf> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?)
        .read_line(&mut line)
        .ok()?;
    let header: Value = serde_json::from_str(&line).ok()?;
    (header["type"] == "session").then_some(())?;
    header["cwd"].as_str().map(PathBuf::from)
}

/// Whether another open file description holds a lock on `path`. Asks with
/// `F_GETLK`, which takes no lock; on macOS it also reports `flock` locks.
#[cfg(unix)]
fn locked(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = File::open(path) else {
        return false;
    };
    // SAFETY: an all-zero `flock` is a valid value of the plain C struct.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    // SAFETY: the descriptor is open for the call and `lock` outlives it.
    let status = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) };
    status == 0 && lock.l_type != libc::F_UNLCK as _
}

#[cfg(not(unix))]
fn locked(_: &Path) -> bool {
    false
}

struct Process {
    pid: u32,
    parent: u32,
    elapsed: Duration,
    /// `ps` start time in UTC, as Claude Code records it.
    started: String,
    command: String,
}

struct PiProcess {
    cwd: PathBuf,
    started: SystemTime,
}

/// This machine's processes, read with `ps` on first use and kept for one
/// observation.
#[derive(Default)]
pub struct Processes {
    table: Option<Vec<Process>>,
    pis: Option<Vec<PiProcess>>,
}

impl Processes {
    fn table(&mut self) -> &[Process] {
        self.table.get_or_insert_with(read_processes)
    }

    /// Whether `pid` is still the process that started at `started`, when known.
    fn alive(&mut self, pid: u32, started: Option<&str>) -> bool {
        let started = started.map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "));
        self.table().iter().any(|process| {
            process.pid == pid
                && started
                    .as_ref()
                    .is_none_or(|value| *value == process.started)
        })
    }

    /// Pi processes other than this process's own children, with their
    /// working directories.
    fn pi(&mut self) -> &[PiProcess] {
        if self.pis.is_none() {
            let own = std::process::id();
            let now = SystemTime::now();
            let found: Vec<(u32, SystemTime)> = self
                .table()
                .iter()
                .filter(|process| process.parent != own && is_pi(&process.command))
                .map(|process| (process.pid, now - process.elapsed))
                .collect();
            let cwds = working_directories(found.iter().map(|(pid, _)| *pid));
            self.pis = Some(
                found
                    .into_iter()
                    .filter_map(|(pid, started)| {
                        Some(PiProcess {
                            cwd: cwds.get(&pid)?.clone(),
                            started,
                        })
                    })
                    .collect(),
            );
        }
        self.pis.as_deref().expect("read")
    }
}

/// Pi 1.0.2 titles its processes `pi` and `pi-rpc`; before it does, the
/// command names its package.
fn is_pi(command: &str) -> bool {
    matches!(command.split_whitespace().next(), Some("pi" | "pi-rpc"))
        || command
            .split_whitespace()
            .any(|argument| argument.contains("/pi-coding-agent/"))
}

fn read_processes() -> Vec<Process> {
    let Ok(output) = crate::process_tree::command("ps")
        .args(["axww", "-o", "pid=,ppid=,etime=,lstart=,command="])
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let elapsed = elapsed(fields.next()?)?;
            // `lstart` is five fields, such as `Mon Oct 12 04:51:53 2026`.
            let started: Vec<&str> = fields.by_ref().take(5).collect();
            (started.len() == 5).then_some(())?;
            Some(Process {
                pid,
                parent,
                elapsed,
                started: started.join(" "),
                command: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

/// `ps` elapsed time: `[[dd-]hh:]mm:ss`.
fn elapsed(value: &str) -> Option<Duration> {
    let (days, clock) = match value.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, value),
    };
    let mut seconds = 0;
    for part in clock.split(':') {
        seconds = seconds * 60 + part.parse::<u64>().ok()?;
    }
    Some(Duration::from_secs(days * 86_400 + seconds))
}

/// Working directories of processes, from `lsof`.
fn working_directories(pids: impl Iterator<Item = u32>) -> HashMap<u32, PathBuf> {
    let pids: Vec<String> = pids.map(|pid| pid.to_string()).collect();
    if pids.is_empty() {
        return HashMap::new();
    }
    let Ok(output) = crate::process_tree::command("lsof")
        .args(["-a", "-d", "cwd", "-Fn", "-p", &pids.join(",")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return HashMap::new();
    };
    let mut found = HashMap::new();
    let mut pid = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if let (Some(value), Some(pid)) = (line.strip_prefix('n'), pid) {
            found.insert(pid, PathBuf::from(value));
        }
    }
    found
}

/// Bytes read from the end of a session file; records of the latest turn
/// boundary are normally well within it.
const TAIL_BYTES: u64 = 256 * 1024;

/// Whether the session's last records describe a turn that has not ended. A
/// process that died mid-turn looks the same; callers pair this with liveness.
fn unfinished(format: SessionFormat, path: &Path) -> bool {
    let Some((tail, truncated)) = read_tail(path) else {
        return false;
    };
    // The first line of a truncated tail is partial; a final line without a
    // newline may still be being written.
    let mut lines: Vec<&str> = tail.split('\n').collect();
    if truncated {
        lines.remove(0);
    }
    let records = lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok());
    let boundary = match format {
        SessionFormat::Codex => codex_boundary(records),
        // Claude Code's registry reports its status directly.
        SessionFormat::ClaudeCode => None,
        SessionFormat::Pi => pi_boundary(records),
    };
    // A tail without any boundary belongs to one long turn when truncated.
    boundary.unwrap_or(truncated && format == SessionFormat::Codex)
}

/// Whether the session's latest ended turn failed, as the Harness recorded
/// it: Codex ends it with `task_complete` carrying an `error`, Claude
/// Code 2.1.286 records the failure as an assistant message with
/// `isApiErrorMessage`, and Pi 1.0.2 as an assistant message whose
/// `stopReason` is `error`. A turn the user stopped did not fail.
pub fn last_turn_failed(format: SessionFormat, path: &Path) -> bool {
    let Some((tail, truncated)) = read_tail(path) else {
        return false;
    };
    let mut lines: Vec<&str> = tail.split('\n').collect();
    if truncated {
        lines.remove(0);
    }
    let mut records = lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok());
    match format {
        SessionFormat::Codex => records
            .filter(|record| record["type"] == "event_msg")
            .find_map(|record| match record["payload"]["type"].as_str()? {
                "task_complete" | "turn_complete" => Some(!record["payload"]["error"].is_null()),
                "task_started" | "turn_started" | "turn_aborted" => Some(false),
                _ => None,
            })
            .unwrap_or(false),
        SessionFormat::ClaudeCode => records
            .find(|record| matches!(record["type"].as_str(), Some("user" | "assistant")))
            .is_some_and(|record| record["isApiErrorMessage"] == true),
        SessionFormat::Pi => records
            .find(|record| {
                record["type"] == "message"
                    && matches!(
                        record["message"]["role"].as_str(),
                        Some("user" | "assistant" | "toolResult")
                    )
            })
            .is_some_and(|record| record["message"]["stopReason"] == "error"),
    }
}

fn read_tail(path: &Path) -> Option<(String, bool)> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some((String::from_utf8_lossy(&bytes).into_owned(), start > 0))
}

/// `Some(true)` when the most recent turn boundary starts a turn.
fn codex_boundary(records: impl Iterator<Item = Value>) -> Option<bool> {
    records
        .filter(|record| record["type"] == "event_msg")
        .find_map(|record| match record["payload"]["type"].as_str()? {
            // Native rollouts name these turn events with both spellings.
            "task_started" | "turn_started" => Some(true),
            "task_complete" | "turn_complete" | "turn_aborted" => Some(false),
            _ => None,
        })
}

fn pi_boundary(records: impl Iterator<Item = Value>) -> Option<bool> {
    records
        .filter(|record| record["type"] == "message")
        .find_map(|record| match record["message"]["role"].as_str()? {
            "user" | "toolResult" => Some(true),
            "assistant" => Some(record["message"]["stopReason"] == "toolUse"),
            _ => None,
        })
}

#[cfg(test)]
mod pi_fork_discovery_tests {
    use super::*;
    #[test]
    fn pi_activity_scans_nested_children_without_entering_artifact_trees() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-pi-activity-{:x}",
            u64::from_le_bytes(nonce)
        ));
        let workspace = root.join("--synthetic--");
        let child = workspace.join("parent.with.dot/forks/run/child.jsonl");
        let grandchild =
            workspace.join("parent.with.dot/forks/run/child/forks/next/grandchild.jsonl");
        let run = "01234567-89ab-4cde-8fab-0123456789ab";
        let modern = workspace.join(format!("parent.with.dot/{run}/run-0/session.jsonl"));
        let modern_second = workspace.join(format!("parent.with.dot/{run}/run-1/session.jsonl"));
        let overflow_slot = workspace.join(format!(
            "parent.with.dot/{run}/run-4294967296/session.jsonl"
        ));
        let padded_slot = workspace.join(format!("parent.with.dot/{run}/run-01/session.jsonl"));
        let negative_slot = workspace.join(format!("parent.with.dot/{run}/run--1/session.jsonl"));
        let nested = modern
            .with_extension("")
            .join(run)
            .join("run-0/session.jsonl");
        let mismatch = workspace.join("parent.with.dot/forks/run/mismatch.jsonl");
        let modern_mismatch = workspace
            .join("parent.with.dot/11234567-89ab-4cde-8fab-0123456789ab/run-0/session.jsonl");
        let wrong_run = workspace.join("parent.with.dot/not-a-run-uuid/run-0/session.jsonl");
        let wrong_name = modern.parent().unwrap().join("intruder.jsonl");
        let unrelated = workspace.join("artifacts/unrelated.jsonl");
        let parent = workspace.join("parent.with.dot.jsonl");
        let ordinary = workspace.join("ordinary.jsonl");
        for (path, ancestor) in [
            (&parent, None),
            (&ordinary, Some(&parent)),
            (&child, Some(&parent)),
            (&grandchild, Some(&child)),
            (&modern, Some(&parent)),
            (&modern_second, Some(&parent)),
            (&nested, Some(&modern)),
            (&mismatch, Some(&ordinary)),
            (&modern_mismatch, Some(&ordinary)),
            (&wrong_run, Some(&parent)),
            (&wrong_name, Some(&parent)),
            (&unrelated, Some(&parent)),
            (&overflow_slot, Some(&parent)),
            (&padded_slot, Some(&parent)),
            (&negative_slot, Some(&parent)),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut header = serde_json::json!({"type":"session","version":3,"id":"synthetic","cwd":"/synthetic"});
            if let Some(ancestor) = ancestor {
                header["parentSession"] = serde_json::json!(ancestor);
            }
            fs::write(path, format!("{header}\n")).unwrap();
        }
        let mut rejected = vec![
            mismatch,
            modern_mismatch,
            wrong_run,
            wrong_name,
            unrelated,
            overflow_slot,
            padded_slot,
            negative_slot,
        ];
        #[cfg(unix)]
        {
            let linked = workspace.join("parent.with.dot/21234567-89ab-4cde-8fab-0123456789ab");
            std::os::unix::fs::symlink(modern.parent().unwrap().parent().unwrap(), &linked)
                .unwrap();
            rejected.push(linked.join("run-0/session.jsonl"));
            let linked_file = child.parent().unwrap().join("linked.jsonl");
            std::os::unix::fs::symlink(&child, &linked_file).unwrap();
            rejected.push(linked_file);
            let linked_workspace = root.join("--linked-workspace--");
            std::os::unix::fs::symlink(&workspace, &linked_workspace).unwrap();
            rejected.push(linked_workspace.join("parent.with.dot.jsonl"));
        }
        let mut found = Vec::new();
        session_files(SessionFormat::Pi, &root, |path, _| {
            found.push(path.to_path_buf())
        });
        found.sort();
        let mut expected = vec![
            parent,
            ordinary,
            child,
            grandchild,
            modern,
            modern_second,
            nested,
        ];
        expected.sort();
        assert_eq!(found, expected);
        let store = SessionStore {
            format: SessionFormat::Pi,
            agent_dir: root.clone(),
            sessions: root.clone(),
        };
        for file in &expected {
            assert_eq!(store.locate(file.to_str().unwrap()).as_ref(), Some(file));
        }
        rejected.push(workspace.join("../--synthetic--/parent.with.dot.jsonl"));
        for file in rejected {
            assert!(
                store.locate(file.to_str().unwrap()).is_none(),
                "unverified source admitted: {}",
                file.display()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
