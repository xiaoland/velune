//! Local AI gateway: explicit model routing and OpenAI protocol ingress.
//!
//! Callers supply configuration, ingress aliases and a
//! platform credential helper. The gateway owns neither application persistence
//! nor execution-runtime configuration.
pub mod config;
mod credential;
pub mod runtime;
pub use config::*;
pub use runtime::{GatewayError, Runner};
