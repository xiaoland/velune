//! The resolver through which the gateway obtains each attempt's credential.
use crate::config::{ExecutionBackend, Protocol};
use velune_ai::OperationFuture;
pub use velune_ai_provider::config::{
    CredentialMaterial, CredentialRecovery, HttpAuthorization, RequestCapabilities, RequestProfile,
    ResolvedCredential,
};

#[derive(Debug, Clone)]
pub struct CredentialTarget {
    pub protocol: Protocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
}
#[derive(Debug, Clone, Copy)]
pub enum CredentialResolutionError {
    Unavailable,
    UnauthorizedTarget,
    Timeout,
    InvalidContract,
    StaleBinding,
    AuthenticationExpired,
}
pub trait CredentialResolver: Send + Sync {
    fn resolve(
        &self,
        reference: String,
        target: CredentialTarget,
    ) -> OperationFuture<Result<ResolvedCredential, CredentialResolutionError>>;
    /// Recover the rejected credential once. The token lets an implementation reuse
    /// a concurrently renewed session instead of rotating its refresh token twice.
    fn refresh(
        &self,
        _reference: String,
        _target: CredentialTarget,
        _rejected_token: String,
    ) -> OperationFuture<Result<ResolvedCredential, CredentialResolutionError>> {
        Box::pin(async { Err(CredentialResolutionError::Unavailable) })
    }
    /// Mark only the still-current rejected session as requiring login.
    fn invalidate(
        &self,
        _reference: String,
        _target: CredentialTarget,
        _rejected_token: String,
    ) -> OperationFuture<()> {
        Box::pin(async {})
    }
}
