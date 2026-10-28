use std::{fmt, net::IpAddr, num::NonZeroU16};
use velune_ai::{InvalidContract, Protocol};

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
    /// An `http` or `https` URL without credentials, query or fragment.
    pub fn parse(value: &str) -> Result<Self, InvalidContract> {
        let url = reqwest::Url::parse(value).map_err(|_| InvalidContract("provider endpoint"))?;
        let transport = match url.scheme() {
            "http" => Transport::Http,
            "https" => Transport::Https,
            _ => return Err(InvalidContract("provider endpoint scheme")),
        };
        if url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(InvalidContract("provider endpoint query"));
        }
        let host = url
            .host_str()
            .ok_or(InvalidContract("provider endpoint host"))?;
        let port = url
            .port_or_known_default()
            .ok_or(InvalidContract("provider endpoint port"))?;
        Self::new(transport, host, port, url.path())
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
/// Where one provider's HTTP requests go and how they are shaped. Private
/// fields keep a running target from changing in place.
#[derive(Debug, Clone)]
pub struct HttpTarget {
    protocol: Protocol,
    /// Messages appends `/v1/messages`; OpenAI base URLs commonly already end in `/v1`.
    endpoint: HttpEndpoint,
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

impl HttpTarget {
    pub fn new(
        protocol: Protocol,
        endpoint: HttpEndpoint,
        request_profile: RequestProfile,
    ) -> Result<Self, InvalidContract> {
        request_profile.check_destination(protocol, &endpoint)?;
        Ok(Self {
            protocol,
            endpoint,
            request_profile,
        })
    }
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }
    pub fn endpoint(&self) -> &HttpEndpoint {
        &self.endpoint
    }
    pub fn request_profile(&self) -> RequestProfile {
        self.request_profile
    }
}

impl RequestProfile {
    /// A profile bound to one service accepts only that service's official
    /// destination for the protocol; its credentials go nowhere else.
    pub fn check_destination(
        self,
        protocol: Protocol,
        endpoint: &HttpEndpoint,
    ) -> Result<(), InvalidContract> {
        let (hosts, path, error): (&[&str], _, _) = match self {
            Self::Native => return Ok(()),
            Self::CodexResponses if protocol != Protocol::Responses => {
                return Err(InvalidContract("Codex request profile requires Responses"));
            }
            Self::CodexResponses => (
                &["chatgpt.com"],
                "/backend-api/codex",
                "invalid Codex request profile destination",
            ),
            Self::ArkCoding => (
                &["ark.cn-beijing.volces.com"],
                if protocol == Protocol::Messages {
                    "/api/coding"
                } else {
                    "/api/coding/v3"
                },
                "ARK Coding requires its official Coding endpoint",
            ),
            Self::KimiCoding => (
                &["api.kimi.com", "api.kimi.ai"],
                if protocol == Protocol::Messages {
                    "/coding"
                } else {
                    "/coding/v1"
                },
                "invalid Kimi Coding destination",
            ),
        };
        if endpoint.transport() != Transport::Https
            || !hosts.contains(&endpoint.host())
            || endpoint.port().get() != 443
            || endpoint.base_path().trim_end_matches('/') != path
        {
            return Err(InvalidContract(error));
        }
        Ok(())
    }
}
