//! API services whose own account metadata Velune reads with the provider's
//! key. Only an exact official HTTPS origin qualifies, so the key reaches no
//! host beyond the one its model requests already go to.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiAccountService {
    DeepSeek,
    /// The mainland open platform; balances in CNY.
    MoonshotCn,
    /// The international open platform; balances in USD.
    MoonshotGlobal,
    OpenRouter,
}
impl ApiAccountService {
    pub fn name(self) -> &'static str {
        match self {
            Self::DeepSeek => "DeepSeek",
            Self::MoonshotCn | Self::MoonshotGlobal => "Moonshot",
            Self::OpenRouter => "OpenRouter",
        }
    }
    pub fn host(self) -> &'static str {
        match self {
            Self::DeepSeek => "api.deepseek.com",
            Self::MoonshotCn => "api.moonshot.cn",
            Self::MoonshotGlobal => "api.moonshot.ai",
            Self::OpenRouter => "openrouter.ai",
        }
    }
}
/// The service a provider address belongs to. The authority must be the
/// official host itself: no user info, port or other host.
pub fn api_account_service(endpoint: &str) -> Option<ApiAccountService> {
    let rest = endpoint.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    [
        ApiAccountService::DeepSeek,
        ApiAccountService::MoonshotCn,
        ApiAccountService::MoonshotGlobal,
        ApiAccountService::OpenRouter,
    ]
    .into_iter()
    .find(|service| service.host() == authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The key reaches only the official host itself; any other authority,
    /// scheme or lookalike name is not an account source.
    #[test]
    fn only_exact_official_origins() {
        assert_eq!(
            api_account_service("https://API.DeepSeek.com/v1"),
            Some(ApiAccountService::DeepSeek)
        );
        assert_eq!(
            api_account_service("https://openrouter.ai/api/v1"),
            Some(ApiAccountService::OpenRouter)
        );
        for other in [
            "http://api.deepseek.com",
            "https://api.deepseek.com.example.com",
            "https://user@api.deepseek.com",
            "https://api.deepseek.com:8443",
            "https://relay.example.com/api.deepseek.com",
            "https://api.kimi.com/coding/",
        ] {
            assert_eq!(api_account_service(other), None, "{other}");
        }
    }
}
