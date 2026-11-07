//! Device-owned schedules. Presentation clients never run a second scheduler.
use crate::conversation::ModelSelection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ScheduledTarget {
    NewConversation {
        runtime_id: String,
        #[serde(default)]
        working_directories: Vec<String>,
        #[serde(default)]
        worktree: bool,
    },
    ExistingConversation {
        runtime_id: String,
        conversation_id: String,
    },
}
impl ScheduledTarget {
    pub fn runtime_id(&self) -> &str {
        match self {
            Self::NewConversation { runtime_id, .. }
            | Self::ExistingConversation { runtime_id, .. } => runtime_id,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledTaskStatus {
    Paused,
    Active,
    NeedsConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledTask {
    /// Empty only when creating. Updates require the current definition revision.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub revision: u64,
    pub name: String,
    pub prompt: String,
    /// RFC 5545 RRULE, optionally preceded by DTSTART. Time zone is always explicit.
    pub rrule: String,
    pub time_zone: String,
    pub status: ScheduledTaskStatus,
    pub target: ScheduledTarget,
    pub model: Option<ModelSelection>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub notify_only_failures: bool,
    pub next_run_at_unix_ms: Option<i64>,
    /// Import provenance is assigned by the owner, never editable by a caller.
    pub source: Option<ScheduledTaskSource>,
    pub issue: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledTaskSource {
    pub path: String,
    pub fingerprint: String,
    pub source_id: String,
    pub original_model: Option<String>,
    pub original_status: String,
    pub original_metadata: String,
    /// Explicit user attestation after pausing the old host and resolving in-flight runs.
    pub handover_confirmed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledRunPhase {
    Waiting,
    Dispatching,
    Running,
    Completed,
    Failed,
    OutcomeUnknown,
    Skipped,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledRun {
    pub id: String,
    pub task_id: String,
    pub definition_revision: u64,
    pub due_at_unix_ms: i64,
    pub phase: ScheduledRunPhase,
    #[serde(default)]
    pub turn_outcome: Option<crate::conversation::TurnOutcome>,
    pub cwd: Option<String>,
    pub runtime_id: String,
    pub conversation_id: Option<String>,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTasksState {
    pub tasks: Vec<ScheduledTask>,
    pub runs: Vec<ScheduledRun>,
    pub import_preview: Vec<ScheduledTask>,
    pub import_issues: Vec<String>,
    pub schedule_preview: Option<SchedulePreview>,
    pub availability: Vec<ScheduledTaskAvailability>,
}
/// A derived editor representation. RRULE and its time zone remain authoritative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchedulePattern {
    Daily {
        hour: u8,
        minute: u8,
    },
    /// ISO weekdays, Monday = 1.
    Weekly {
        weekdays: Vec<u8>,
        hour: u8,
        minute: u8,
    },
    Hourly {
        interval: u16,
    },
    Custom,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreview {
    pub rrule: String,
    pub time_zone: String,
    pub pattern: SchedulePattern,
    pub next_at_unix_ms: Vec<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskAvailability {
    pub task_id: String,
    pub enable_issue: Option<String>,
    pub run_issue: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "operation",
    content = "parameters",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
// Foreign-language value contract; the service boxes command envelopes at its queue boundary.
#[allow(clippy::large_enum_variant)]
pub enum ScheduledTasksOperation {
    List,
    Save {
        task: ScheduledTask,
    },
    SetEnabled {
        id: String,
        revision: u64,
        enabled: bool,
    },
    PreviewSchedule {
        rrule: String,
        time_zone: String,
        pattern: Option<SchedulePattern>,
    },
    Delete {
        id: String,
        revision: u64,
    },
    RunNow {
        id: String,
        revision: u64,
    },
    PreviewImport {
        source_directory: String,
        runtime_id: String,
        time_zone: String,
    },
    Import {
        source_path: String,
        fingerprint: String,
        runtime_id: String,
        time_zone: String,
        /// Revision whose replacement the user reviewed; None for a first import.
        expected_revision: Option<u64>,
    },
    ConfirmHandover {
        id: String,
        revision: u64,
    },
}
