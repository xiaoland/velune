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
    pub fn for_runtime_type(type_id: &str) -> Option<Self> {
        match crate::version::variant(type_id)?.family_id {
            "codex" => Some(Self::Codex),
            "claude-code" => Some(Self::ClaudeCode),
            "pi" => Some(Self::Pi),
            _ => None,
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
    walk(root, format.depth(), &mut visit);
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
            return (path.starts_with(&self.sessions) && path.is_file()).then(|| path.to_owned());
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

    /// Threads whose writer lock a Codex process holds. Codex 0.159.3 holds
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
    status == 0 && i32::from(lock.l_type) != i32::from(libc::F_UNLCK)
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
/// it: Codex 0.159.3 ends it with `task_complete` carrying an `error`, Claude
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
            // Codex 0.159.3 names these turn events with both spellings.
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
