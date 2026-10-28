//! Provider execution: native protocol requests over HTTP, the Claude Code
//! local backend, protocol translation and response metrics. Configuration and
//! credentials arrive resolved; no environment, file or configuration reads here.
#![deny(unsafe_code)]
pub mod claude_code;
pub mod config;
mod http_client;
pub use http_client::HttpClient;
pub mod metrics;
mod native;
pub use native::NativeHttp;
mod responses;
mod service_error;
pub mod translation;
