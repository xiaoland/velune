//! Credential-blind Pi runtime adapter.

pub use velune_conversation as conversation;

mod client;
pub mod model_projection;
mod projection;

pub use client::*;
pub use projection::PiProjection;

pub mod authentication;
pub mod provider_source;

pub mod history;
pub mod native;
pub mod version;

pub mod transcript;
