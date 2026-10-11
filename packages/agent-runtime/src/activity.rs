//! Best-effort observation of native session files on this machine, including
//! files that Harness processes outside Velune are writing. The native files
//! stay the source of truth: this only locates a session file, notices that it
//! changed and reads its last records to tell whether a turn has not ended.
//! History for display is still read through the Harness adapters.
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Native session file layouts this module understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFormat {
    /// `sessions/YYYY/MM/DD/rollout-<date>-<thread id>.jsonl`
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
            // The thread id is the trailing UUID of the rollout name.
            Self::Codex => stem
                .strip_prefix("rollout-")
                .filter(|_| stem.len() > 36)
                .map(|_| stem[stem.len() - 36..].to_owned()),
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

/// The session file of `native_id` below `root`.
pub fn locate(format: SessionFormat, root: &Path, native_id: &str) -> Option<PathBuf> {
    if format == SessionFormat::Pi {
        let path = Path::new(native_id);
        return (path.starts_with(root) && path.is_file()).then(|| path.to_owned());
    }
    let mut found = None;
    session_files(format, root, |path, _| {
        if found.is_none() && format.native_id(path).as_deref() == Some(native_id) {
            found = Some(path.to_owned());
        }
    });
    found
}

/// Native ids of sessions below `root` modified at or after `since` whose
/// last records describe a turn that has not ended.
pub fn unfinished_since(format: SessionFormat, root: &Path, since: SystemTime) -> Vec<String> {
    let mut recent = Vec::new();
    session_files(format, root, |path, metadata| {
        if metadata.modified().is_ok_and(|modified| modified >= since) {
            recent.push(path.to_owned());
        }
    });
    recent
        .into_iter()
        .filter(|path| unfinished(format, path))
        .filter_map(|path| format.native_id(&path))
        .collect()
}

/// Bytes read from the end of a session file; records of the latest turn
/// boundary are normally well within it.
const TAIL_BYTES: u64 = 256 * 1024;

/// Whether the session's last records describe a turn that has not ended. A
/// process that died mid-turn looks the same; callers bound this by recency.
pub fn unfinished(format: SessionFormat, path: &Path) -> bool {
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
        SessionFormat::ClaudeCode => claude_boundary(records),
        SessionFormat::Pi => pi_boundary(records),
    };
    // A tail without any boundary belongs to one long turn when truncated.
    boundary.unwrap_or(truncated && format == SessionFormat::Codex)
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

fn claude_boundary(records: impl Iterator<Item = Value>) -> Option<bool> {
    records
        .filter(|record| record["isSidechain"] != true && record["isMeta"] != true)
        .find_map(|record| {
            let content = &record["message"]["content"];
            match record["type"].as_str()? {
                // A prompt or tool result the model has not answered yet.
                // Local commands and interruptions do not start a model turn.
                "user" => Some(match content {
                    Value::String(text) => {
                        !text.starts_with('<') && !text.starts_with("[Request interrupted")
                    }
                    Value::Array(blocks) => !blocks.iter().any(|block| {
                        block["text"]
                            .as_str()
                            .is_some_and(|text| text.starts_with("[Request interrupted"))
                    }),
                    _ => false,
                }),
                // A tool call waits for its result, or for permission.
                "assistant" => {
                    Some(content.as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| block["type"] == "tool_use")
                    }))
                }
                _ => None,
            }
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
