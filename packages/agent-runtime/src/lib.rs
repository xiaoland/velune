//! Credential-blind Pi runtime adapter.

pub use velune_conversation as conversation;

mod client;
pub mod model_projection;
mod projection;

pub use client::*;
pub use projection::PiProjection;

pub mod acp;
pub mod activity;
pub mod authentication;
pub mod provider_source;

pub mod execution;
pub mod history;
pub mod management;
pub mod mcp;
pub mod native;
pub mod version;

pub mod transcript;

/// One user turn as delivered through a harness's own input channel.
#[derive(Debug, Clone)]
pub struct TurnInput {
    pub text: String,
    /// A reasoning level in the harness's own vocabulary for this turn;
    /// `None` leaves it to the harness default.
    pub effort: Option<String>,
}
