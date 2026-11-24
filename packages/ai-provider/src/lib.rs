//! Provider execution: native protocol requests over HTTP, the Claude Code
//! local backend, protocol translation and response metrics. Configuration and
//! credentials arrive resolved; no environment, file or configuration reads here.
#![deny(unsafe_code)]
pub mod claude_code;
pub mod config;
pub use config::valid_codex_client_version;
mod gemini;
mod http_client;
pub use http_client::HttpClient;
pub mod metrics;
mod native;
pub use native::NativeHttp;
mod responses;
pub use responses::codex_user_agent;
mod service_error;
pub mod text;
pub mod translation;
