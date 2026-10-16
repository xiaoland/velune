//! Work a runtime process still does for a conversation besides its turn:
//! native sub-agents and background tasks the turn started, which may
//! outlive it. Adapters report what their protocol tells; how long the
//! process and its turn's settings must then last is the host's decision.

/// One piece of outstanding work, named by the conversation it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTask {
    pub conversation_id: String,
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
