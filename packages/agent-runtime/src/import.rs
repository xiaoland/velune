//! Native handoff. Source adapters export effective model context; target
//! adapters write it as native messages. Display history is never model input.
use crate::execution::ExecutionTarget;
use crate::native::{Error, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
mod claude;
mod codex;
mod context;
mod pi;
pub use claude::import_claude_conversation;
pub use codex::import_codex_conversation;
pub use context::Context;
use context::{Block, Role};
pub use pi::import_pi_conversation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedConversation {
    ClaudeCode { session_id: String },
    Codex { thread_id: String },
    Pi { path: PathBuf },
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|error| Error::new(format!("会话身份生成失败：{error}")))?;
    Ok(bytes)
}

fn uuid4() -> Result<String> {
    let mut bytes = random_bytes::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid_string(bytes))
}

/// A UUIDv7 as Codex names its threads: millisecond time, then random.
fn uuid7(now_unix_ms: i64) -> Result<String> {
    let mut bytes = random_bytes::<16>()?;
    let ms = ((now_unix_ms.max(0)) as u64 & 0xffff_ffff_ffff).to_be_bytes();
    bytes[..6].copy_from_slice(&ms[2..]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid_string(bytes))
}

fn uuid_string(bytes: [u8; 16]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn hex_id() -> Result<String> {
    Ok(random_bytes::<4>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn prefixed_id(prefix: &str) -> Result<String> {
    let hex: String = random_bytes::<12>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{prefix}{hex}"))
}

fn iso_time(unix_ms: i64) -> String {
    let t = OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        t.month() as u8,
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

fn now_unix_ms() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}

fn write_jsonl(execution: &ExecutionTarget, path: &Path, records: &[Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        execution
            .create_dir_all(parent)
            .map_err(|error| Error::new(error.to_string()))?;
    }
    let mut body = String::new();
    for record in records {
        body.push_str(
            &serde_json::to_string(record).map_err(|error| Error::new(error.to_string()))?,
        );
        body.push('\n');
    }
    execution
        .write_file_atomic(path, body.as_bytes())
        .map_err(|error| Error::new(error.to_string()))
}

/// A tool name every target accepts: Claude Code requires
/// `^[a-zA-Z0-9_-]{1,64}$`; the others take any string.
fn tool_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "tool".into()
    } else {
        cleaned.chars().take(64).collect()
    }
}

/// Claude Code keeps a session under the project folder named after the
/// process's physical working directory, non-alphanumerics replaced.
fn claude_project_directory(config_dir: &Path, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    config_dir.join("projects").join(slug)
}

fn codex_rollout_name(now_unix_ms: i64, thread_id: &str) -> String {
    let seconds = iso_time(now_unix_ms);
    let seconds = seconds.split('.').next().unwrap_or(&seconds);
    format!("rollout-{}-{thread_id}.jsonl", seconds.replace(':', "-"))
}

fn pi_session_directory(root: &Path, cwd: &Path) -> PathBuf {
    let text = cwd.to_string_lossy();
    let stripped = text.trim_start_matches(['/', '\\']);
    let slug: String = stripped
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c => c,
        })
        .collect();
    root.join(format!("--{slug}--"))
}
