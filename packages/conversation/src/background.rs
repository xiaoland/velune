//! Background work has its own identity; a task need not have a conversation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundTaskStatus {
    Running,
    Stopping,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTaskInfo {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: BackgroundTaskStatus,
    pub conversation_id: Option<String>,
    pub child_session_id: Option<String>,
    pub can_stop: bool,
    pub can_read_output: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTaskList {
    pub tasks: Vec<BackgroundTaskInfo>,
    /// True means the list is an observation of a live owner.
    /// An offline list without an issue is an ordinary connection state.
    pub live: bool,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundTaskOutputStatus {
    Checking,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTaskOutput {
    pub request_id: String,
    pub task_id: String,
    pub status: BackgroundTaskOutputStatus,
    pub text: String,
    pub truncated: bool,
    pub issue: Option<String>,
}
