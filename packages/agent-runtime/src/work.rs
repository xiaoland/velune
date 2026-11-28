//! Work a runtime process still does for a conversation besides its turn:
//! native sub-agents and background tasks the turn started, which may
//! outlive it. Adapters report what their protocol tells; how long the
//! process and its turn's settings must then last is the host's decision.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkStatus {
    /// The adapter has no background lifecycle contract for this runtime.
    Untracked,
    Active,
    /// Observation or completion delivery still needs the connection.
    Settling,
    Settled,
}

impl WorkStatus {
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Active | Self::Settling)
    }
}

/// One piece of outstanding work. Some tasks run inside the owning session
/// rather than in a separately addressable conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTask {
    pub conversation_id: Option<String>,
    pub state: BackgroundState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundState {
    Running,
    AwaitingApproval,
    AwaitingInput,
    /// Asked to stop; it has not ended yet.
    Stopping,
}
