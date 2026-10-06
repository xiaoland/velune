//! Local AI gateway: explicit model routing and OpenAI protocol ingress.
//!
//! Callers supply configuration, ingress aliases and a
//! asynchronous credential resolver. The gateway owns neither application persistence
//! nor execution-runtime configuration.
pub mod config;
pub mod runtime;
pub use config::*;
pub use runtime::{
    CredentialResolutionError, CredentialResolver, CredentialTarget, GatewayError,
    ResolvedCredential, Runner,
};

mod observation;
