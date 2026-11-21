//! Explicit, prepared cross-Harness handoffs.
use crate::NextTurnInput;
use serde::{Deserialize, Serialize};

/// Result of checking whether a cross-runtime send can use the ordinary import path.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum HandoffRequirements {
    /// A bounded read is still checking the source's effective native context.
    Checking { request_id: String },
    /// No portable summary is required; use the existing send/queue command.
    Ready,
    /// The source contains context that must be summarized before target creation.
    PortableSummaryRequired {
        summary_basis: Box<NextTurnInput>,
        default_model_issue: Option<String>,
    },
    /// The source could not be checked; retry the read using a new request ID.
    Failed { code: String, detail: String },
}

/// Application-private prepared work identifier.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HandoffSummaryStart {
    pub work_id: String,
}

/// Acceptance of the first queued turn and its confirmed portable preparation.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct QueuedHandoffSummaryStart {
    pub work_id: String,
    pub queued_message_id: String,
    pub result: crate::SnapshotResult,
}

/// Progress of one explicitly requested source-side summary call.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum HandoffSummaryStatus {
    Running,
    Cancelling,
    Ready { prepared_token: String },
    Failed { code: String, detail: String },
    Cancelled,
}
