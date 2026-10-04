use std::{collections::HashSet, fmt, net::IpAddr, num::NonZeroU16};
use velune_ai::{
    InvalidContract,
    ids::{ConfigRevision, ModelId, ProviderId},
};

/// Reference only, not a token/key or a resolver. Never fetched from the environment by this crate.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRef(String);
impl CredentialRef {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidContract> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        {
            return Err(InvalidContract("invalid credential reference"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialRef([redacted])")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Https,
    Http,
}
/// Structured endpoint avoids accepting embedded credentials, arbitrary schemes, query or fragment.
/// Http is explicit (e.g. a local fixture); production destination policy belongs to the app.
#[derive(Clone)]
pub struct HttpEndpoint {
    transport: Transport,
    host: String,
    port: NonZeroU16,
    base_path: String,
}
impl HttpEndpoint {
    pub fn new(
        transport: Transport,
        host: impl Into<String>,
        port: u16,
        base_path: impl Into<String>,
    ) -> Result<Self, InvalidContract> {
        let host = host.into();
        let base_path = base_path.into();
        let valid_dns = host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        if host.parse::<IpAddr>().is_err() && !valid_dns {
            return Err(InvalidContract("invalid endpoint host"));
        }
        let port =
            NonZeroU16::new(port).ok_or(InvalidContract("endpoint port must be positive"))?;
        if !base_path.starts_with('/')
            || base_path.len() > 1024
            || !base_path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
            || base_path.split('/').any(|s| s == ".." || s == ".")
        {
            return Err(InvalidContract("invalid endpoint base path"));
        }
        Ok(Self {
            transport,
            host,
            port,
            base_path,
        })
    }
    pub fn transport(&self) -> Transport {
        self.transport
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn port(&self) -> NonZeroU16 {
        self.port
    }
    pub fn base_path(&self) -> &str {
        &self.base_path
    }
    pub fn url(&self, suffix: &str) -> String {
        let scheme = match self.transport {
            Transport::Https => "https",
            Transport::Http => "http",
        };
        format!(
            "{scheme}://{}:{}{}{}",
            self.host, self.port, self.base_path, suffix
        )
    }
}
impl fmt::Debug for HttpEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HttpEndpoint([redacted])")
    }
}
/// Protocol-specific configuration is provider-owned, not part of the service's sampling input.
#[derive(Debug, Clone)]
pub struct ResponsesConfig {
    pub endpoint: HttpEndpoint,
}
#[derive(Debug, Clone)]
pub struct MessagesConfig {
    pub endpoint: HttpEndpoint,
}
#[derive(Debug, Clone)]
pub struct ChatCompletionsConfig {
    pub endpoint: HttpEndpoint,
}
#[derive(Debug, Clone)]
pub enum ProtocolConfig {
    Responses(ResponsesConfig),
    Messages(MessagesConfig),
    ChatCompletions(ChatCompletionsConfig),
}
#[derive(Debug, Clone)]
pub struct ModelMapping {
    id: ModelId,
    external_name: String,
}
impl ModelMapping {
    pub fn new(id: ModelId, external_name: impl Into<String>) -> Result<Self, InvalidContract> {
        let external_name = external_name.into();
        if external_name.trim().is_empty()
            || external_name.len() > 256
            || external_name.chars().any(char::is_control)
        {
            return Err(InvalidContract("invalid external model name"));
        }
        Ok(Self { id, external_name })
    }
    pub fn id(&self) -> &ModelId {
        &self.id
    }
    pub fn external_name(&self) -> &str {
        &self.external_name
    }
}
/// Unified config center owns persistence and deserialization; app main validates and assembles.
/// Private fields prevent mutating an existing running configuration in place.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    id: ProviderId,
    revision: ConfigRevision,
    protocol: ProtocolConfig,
    credential: CredentialRef,
    credential_source: Option<String>,
    models: Vec<ModelMapping>,
}

/// The platform helper returns this transient material per dispatch. It is not
/// persisted in provider configuration and its token has no Debug impl.
pub struct ResolvedCredential {
    pub token: String,
    pub explicit_output_cap: Option<bool>,
    /// The platform helper's authentication contract, when it is a managed
    /// subscription rather than an ordinary API credential.
    pub subscription: bool,
}

pub fn parse_resolved_credential(
    value: &str,
    source: bool,
    expected_protocol: &str,
) -> Option<ResolvedCredential> {
    if !source {
        return (!value.is_empty()).then(|| ResolvedCredential {
            token: value.to_owned(),
            explicit_output_cap: None,
            subscription: false,
        });
    }
    let value: serde_json::Value = serde_json::from_str(value).ok()?;
    if value["contractVersion"] != 1 || value["capabilities"]["protocol"] != expected_protocol {
        return None;
    }
    let token = value["bearer"].as_str()?.trim();
    if token.is_empty() {
        return None;
    }
    Some(ResolvedCredential {
        token: token.to_owned(),
        explicit_output_cap: value["capabilities"]["explicitOutputCap"].as_bool(),
        subscription: value["capabilities"]["authentication"] == "subscription",
    })
}
impl ProviderConfig {
    pub fn new(
        id: ProviderId,
        revision: ConfigRevision,
        protocol: ProtocolConfig,
        credential: CredentialRef,
        models: Vec<ModelMapping>,
    ) -> Result<Self, InvalidContract> {
        if models.is_empty() {
            return Err(InvalidContract("provider model list must not be empty"));
        }
        let mut seen = HashSet::new();
        if models.iter().any(|model| !seen.insert(model.id())) {
            return Err(InvalidContract(
                "duplicate model ID in provider configuration",
            ));
        }
        Ok(Self {
            id,
            revision,
            protocol,
            credential,
            credential_source: None,
            models,
        })
    }
    pub fn id(&self) -> &ProviderId {
        &self.id
    }
    pub fn revision(&self) -> ConfigRevision {
        self.revision
    }
    pub fn protocol(&self) -> &ProtocolConfig {
        &self.protocol
    }
    pub fn credential(&self) -> &CredentialRef {
        &self.credential
    }
    pub fn with_credential_source(mut self, source_json: Option<String>) -> Self {
        self.credential_source = source_json;
        self
    }
    pub fn credential_source(&self) -> Option<&str> {
        self.credential_source.as_deref()
    }
    pub fn models(&self) -> &[ModelMapping] {
        &self.models
    }
    pub fn model(&self, id: &ModelId) -> Option<&ModelMapping> {
        self.models.iter().find(|model| model.id() == id)
    }
}
