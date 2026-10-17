//! Local AI gateway: explicit model routing and OpenAI protocol ingress.
//!
//! Callers supply configuration, ingress aliases and a
//! asynchronous credential resolver. The gateway owns neither application persistence
//! nor execution-runtime configuration.
pub mod analytics;
pub mod config;
pub mod runtime;
pub mod scheduling;
pub use analytics::{
    AnalyticsOutcome, AnalyticsProtocol, AnalyticsRecord, AnalyticsSink, SharedAnalyticsSink,
    TokenUsage,
};
pub use config::*;
pub use runtime::{
    CredentialMaterial, CredentialRecovery, CredentialResolutionError, CredentialResolver,
    CredentialTarget, ExecutionBackends, GatewayError, HttpAuthorization, RequestCapabilities,
    RequestProfile, ResolvedCredential, Runner,
};
pub use scheduling::{
    QuotaSnapshot, QuotaSource, QuotaWindow, SchedulingState, SessionIdentity, UnknownQuotaSource,
};

mod observation;
mod translation;
