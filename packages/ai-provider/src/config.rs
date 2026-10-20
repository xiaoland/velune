use std::{fmt, net::IpAddr, num::NonZeroU16};
use velune_ai::{
    InvalidContract,
    ids::{ConfigRevision, ProviderId},
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
            self.host,
            self.port,
            self.base_path.trim_end_matches('/'),
            suffix
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
/// Native Messages SDK base URL: the adapter appends /v1/messages.
/// This differs from an OpenAI base URL which commonly already ends in /v1.
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
/// Unified config center owns persistence and deserialization; app main validates and assembles.
/// Private fields prevent mutating an existing running configuration in place.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    id: ProviderId,
    revision: ConfigRevision,
    protocol: ProtocolConfig,
    credential: CredentialRef,
    request_profile: RequestProfile,
}

/// Request behavior selected independently of the authentication source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestProfile {
    Native,
    CodexResponses,
    KimiCoding,
    ArkCoding,
}

#[derive(Clone)]
pub enum HttpAuthorization {
    Bearer(String),
    ApiKey(String),
}
impl HttpAuthorization {
    pub fn secret(&self) -> &str {
        match self {
            Self::Bearer(value) | Self::ApiKey(value) => value,
        }
    }
}
#[derive(Clone)]
pub enum CredentialMaterial {
    Http {
        authorization: HttpAuthorization,
        account_id: Option<String>,
    },
    /// A local CLI's own login; `None` is the user's default login.
    LocalSession { config_dir: Option<String> },
}
/// A stable opaque identity for refresh recovery, not a vendor or a token.
#[derive(Clone, PartialEq, Eq)]
pub enum CredentialRecovery {
    None,
    Refreshable { identity: String },
}
#[derive(Clone, Default)]
pub struct RequestCapabilities {
    pub explicit_output_cap: Option<bool>,
}
/// Short-lived material supplied per dispatch; no Debug implementation exposes secrets.
#[derive(Clone)]
pub struct ResolvedCredential {
    pub material: CredentialMaterial,
    pub recovery: CredentialRecovery,
    pub request_capabilities: RequestCapabilities,
}
impl ResolvedCredential {
    pub fn token(&self) -> Option<&str> {
        match &self.material {
            CredentialMaterial::Http { authorization, .. } => Some(authorization.secret()),
            _ => None,
        }
    }
}

impl ProviderConfig {
    pub fn new(
        id: ProviderId,
        revision: ConfigRevision,
        protocol: ProtocolConfig,
        credential: CredentialRef,
        request_profile: RequestProfile,
    ) -> Result<Self, InvalidContract> {
        if request_profile == RequestProfile::CodexResponses {
            let ProtocolConfig::Responses(config) = &protocol else {
                return Err(InvalidContract("Codex request profile requires Responses"));
            };
            if config.endpoint.transport() != Transport::Https
                || config.endpoint.host() != "chatgpt.com"
                || config.endpoint.port().get() != 443
                || config.endpoint.base_path().trim_end_matches('/') != "/backend-api/codex"
            {
                return Err(InvalidContract("invalid Codex request profile destination"));
            }
        }
        if request_profile == RequestProfile::ArkCoding {
            let endpoint = match &protocol {
                ProtocolConfig::ChatCompletions(p) => &p.endpoint,
                ProtocolConfig::Responses(p) => &p.endpoint,
                ProtocolConfig::Messages(p) => &p.endpoint,
            };
            let path = if matches!(protocol, ProtocolConfig::Messages(_)) {
                "/api/coding"
            } else {
                "/api/coding/v3"
            };
            if endpoint.transport() != Transport::Https
                || endpoint.host() != "ark.cn-beijing.volces.com"
                || endpoint.port().get() != 443
                || endpoint.base_path().trim_end_matches('/') != path
            {
                return Err(InvalidContract(
                    "ARK Coding requires its official Coding endpoint",
                ));
            }
        }
        if request_profile == RequestProfile::KimiCoding {
            let (endpoint, path) = match &protocol {
                ProtocolConfig::Messages(config) => (&config.endpoint, "/coding"),
                ProtocolConfig::ChatCompletions(config) => (&config.endpoint, "/coding/v1"),
                ProtocolConfig::Responses(config) => (&config.endpoint, "/coding/v1"),
            };
            if endpoint.transport() != Transport::Https
                || !matches!(endpoint.host(), "api.kimi.com" | "api.kimi.ai")
                || endpoint.port().get() != 443
                || endpoint.base_path().trim_end_matches('/') != path
            {
                return Err(InvalidContract("invalid Kimi Coding destination"));
            }
        }
        Ok(Self {
            id,
            revision,
            protocol,
            credential,
            request_profile,
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
    pub fn request_profile(&self) -> RequestProfile {
        self.request_profile
    }
    pub fn credential(&self) -> &CredentialRef {
        &self.credential
    }
}
