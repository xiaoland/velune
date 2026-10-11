//! Explicit host grants and observable state for one-level Agent delegation.
use crate::{DeliveredSnapshot, ModelSelection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollaborationTarget {
    pub id: String,
    pub name: String,
    pub runtime_id: String,
    pub cwd: String,
    pub model_selection: ModelSelection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollaborationLimits {
    pub max_children: u32,
    pub turn_timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChildState {
    Starting,
    Running,
    WaitingForApproval,
    Cancelling,
    Idle,
    Failed,
    Cancelled,
    TimedOut,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildStatus {
    pub id: String,
    pub target_id: String,
    pub runtime_id: String,
    pub conversation_id: Option<String>,
    pub revision: u64,
    pub state: ChildState,
    pub turn: u64,
    /// The settled turn's final assistant text, not an independently verified completion.
    pub result: Option<String>,
    pub error: Option<String>,
    pub pending_interaction_count: u32,
}

/// Host-only detail. The parent Agent cannot answer a child's permission requests.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildSnapshot {
    pub status: ChildStatus,
    pub snapshot: Option<DeliveredSnapshot>,
}
