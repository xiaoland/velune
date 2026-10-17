//! What a local Codex or Claude Code configuration reaches and how, read from
//! the harness's own files for importing as providers. Only literal keys are
//! carried, redacted; a Codex `auth.json` login is never opened.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A secret read from a harness file; it never prints or serializes.
#[derive(Clone)]
pub struct Secret(String);
impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Responses,
    Messages,
}

/// How the harness sends a literal key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyHeader {
    ApiKey,
    Bearer,
}

/// Where the harness takes an API's key from.
#[derive(Debug, Clone)]
pub enum KeySource {
    Literal {
        value: Secret,
        header: KeyHeader,
    },
    /// The named variable of the shell that starts the harness.
    Environment {
        variable: String,
    },
    /// A command the harness runs for the key (Claude Code `apiKeyHelper`).
    Helper,
    Missing,
}

#[derive(Debug, Clone)]
pub enum Access {
    Http {
        protocol: Protocol,
        endpoint: String,
        key: KeySource,
    },
    /// The harness's own Claude Code login, run by the official CLI.
    ClaudeLogin,
}

#[derive(Debug, Clone)]
pub struct InventoryProvider {
    pub source_provider_id: String,
    pub name: String,
    pub access: Access,
    /// Provider model IDs the configuration names for this provider.
    pub models: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    pub providers: Vec<InventoryProvider>,
    pub warnings: Vec<String>,
    /// Changes with the files read, to bind a preview to what it showed.
    pub fingerprint: String,
    pub source_label: String,
}

#[derive(Debug)]
pub struct InventoryError(pub String);
impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Model aliases the Claude Code CLI accepts for `--model`.
pub const CLAUDE_CODE_MODEL_ALIASES: [&str; 3] = ["sonnet", "opus", "haiku"];

fn read_optional(path: &Path) -> Result<Option<String>, InventoryError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(InventoryError(format!("无法读取 {}", path.display()))),
    }
}
fn fingerprint(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}
fn push_unique(models: &mut Vec<String>, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty())
        && !models.iter().any(|m| m == value)
    {
        models.push(value.to_owned());
    }
}

/// Reads Claude Code's user settings in `config_dir` (the instance's
/// `CLAUDE_CONFIG_DIR`, the user's `~/.claude` by default): its own login, and
/// the API its `env` block points at, if any.
pub fn claude_code(config_dir: &Path) -> Result<Inventory, InventoryError> {
    let path = config_dir.join("settings.json");
    let text = read_optional(&path)?;
    let settings: Value = match &text {
        Some(text) => serde_json::from_str(text)
            .map_err(|_| InventoryError(format!("{} 不是有效的 JSON", path.display())))?,
        None => Value::Null,
    };
    let env = &settings["env"];
    let mut providers = vec![InventoryProvider {
        source_provider_id: "claudeCodeLogin".into(),
        name: "Claude Code 订阅".into(),
        access: Access::ClaudeLogin,
        models: CLAUDE_CODE_MODEL_ALIASES.map(String::from).to_vec(),
        issues: Vec::new(),
    }];
    let base_url = env["ANTHROPIC_BASE_URL"].as_str().filter(|v| !v.is_empty());
    let key = if let Some(token) = env["ANTHROPIC_AUTH_TOKEN"]
        .as_str()
        .filter(|v| !v.is_empty())
    {
        KeySource::Literal {
            value: Secret(token.to_owned()),
            header: KeyHeader::Bearer,
        }
    } else if let Some(key) = env["ANTHROPIC_API_KEY"].as_str().filter(|v| !v.is_empty()) {
        KeySource::Literal {
            value: Secret(key.to_owned()),
            header: KeyHeader::ApiKey,
        }
    } else if settings["apiKeyHelper"]
        .as_str()
        .is_some_and(|v| !v.is_empty())
    {
        KeySource::Helper
    } else {
        KeySource::Missing
    };
    if base_url.is_some() || matches!(key, KeySource::Literal { .. } | KeySource::Helper) {
        let mut models = Vec::new();
        for value in [
            env["ANTHROPIC_MODEL"].as_str(),
            settings["model"].as_str(),
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"].as_str(),
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"].as_str(),
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_str(),
            env["ANTHROPIC_SMALL_FAST_MODEL"].as_str(),
        ] {
            // Aliases name Claude Code's own choice, not an upstream model ID.
            if value.is_some_and(|v| !CLAUDE_CODE_MODEL_ALIASES.contains(&v) && v != "default") {
                push_unique(&mut models, value);
            }
        }
        let mut issues = Vec::new();
        match &key {
            KeySource::Literal {
                header: KeyHeader::Bearer,
                ..
            } => issues.push("来源以 Authorization: Bearer 发送密钥；Velune 以 x-api-key 发送，部分服务可能不接受。".into()),
            KeySource::Helper => issues.push("来源通过 apiKeyHelper 取得密钥；导入后请在 Velune 中填写 API key。".into()),
            KeySource::Missing => issues.push("来源未在 settings.json 中保存密钥；导入后请在 Velune 中填写 API key。".into()),
            KeySource::Literal { .. } | KeySource::Environment { .. } => {}
        }
        if models.is_empty() {
            issues.push("来源未指定模型；导入后请为提供商添加模型。".into());
        }
        providers.push(InventoryProvider {
            source_provider_id: "claudeCodeSettingsApi".into(),
            name: "Claude Code API（settings.json）".into(),
            access: Access::Http {
                protocol: Protocol::Messages,
                endpoint: base_url.unwrap_or("https://api.anthropic.com").to_owned(),
                key,
            },
            models,
            issues,
        });
    }
    Ok(Inventory {
        providers,
        warnings: Vec::new(),
        fingerprint: fingerprint(&[
            "claude",
            &config_dir.to_string_lossy(),
            text.as_deref().unwrap_or(""),
        ]),
        source_label: path.display().to_string(),
    })
}

/// Reads the custom model providers of Codex's `config.toml` in `codex_home`.
/// Codex's own OpenAI login stays in Codex: use the Codex subscription
/// template to sign that account in to Velune.
pub fn codex(codex_home: &Path) -> Result<Inventory, InventoryError> {
    let path = codex_home.join("config.toml");
    let text = read_optional(&path)?;
    let config: Value = match &text {
        Some(text) => toml_edit::de::from_str(text)
            .map_err(|_| InventoryError(format!("{} 不是有效的 TOML", path.display())))?,
        None => Value::Null,
    };
    let mut warnings = Vec::new();
    let selected = config["model_provider"].as_str().unwrap_or("openai");
    if selected == "openai" {
        warnings.push("Codex 的 OpenAI／ChatGPT 登录不会导入；如需使用该订阅，请用“Codex 订阅”模板在 Velune 中登录。".into());
    }
    let mut providers = Vec::new();
    if let Some(table) = config["model_providers"].as_object() {
        for (id, definition) in table {
            let mut issues = Vec::new();
            let Some(endpoint) = definition["base_url"].as_str().filter(|v| !v.is_empty()) else {
                warnings.push(format!("提供商 {id} 没有 base_url，未列出。"));
                continue;
            };
            match definition["wire_api"].as_str() {
                None | Some("responses") => {}
                Some(other) => {
                    warnings.push(format!(
                        "提供商 {id} 使用不受支持的 wire_api “{other}”，未列出。"
                    ));
                    continue;
                }
            }
            if definition["requires_openai_auth"].as_bool() == Some(true) {
                warnings.push(format!("提供商 {id} 使用 OpenAI 登录认证，未列出。"));
                continue;
            }
            let key = if let Some(token) = definition["experimental_bearer_token"]
                .as_str()
                .filter(|v| !v.is_empty())
            {
                KeySource::Literal {
                    value: Secret(token.to_owned()),
                    header: KeyHeader::Bearer,
                }
            } else if let Some(variable) = definition["env_key"].as_str().filter(|v| !v.is_empty())
            {
                issues.push(format!(
                    "来源从环境变量 {variable} 读取密钥；导入后请在 Velune 中填写 API key。"
                ));
                KeySource::Environment {
                    variable: variable.to_owned(),
                }
            } else {
                issues.push("来源未配置密钥；如服务需要，请在 Velune 中填写 API key。".into());
                KeySource::Missing
            };
            if ["http_headers", "env_http_headers", "query_params"]
                .iter()
                .any(|field| !definition[*field].is_null())
            {
                issues.push("来源附加的请求头或查询参数不会导入。".into());
            }
            let mut models = Vec::new();
            if selected == id.as_str() {
                push_unique(&mut models, config["model"].as_str());
            }
            if let Some(profiles) = config["profiles"].as_object() {
                for profile in profiles.values() {
                    if profile["model_provider"].as_str() == Some(id.as_str()) {
                        push_unique(&mut models, profile["model"].as_str());
                    }
                }
            }
            if models.is_empty() {
                issues.push("来源未为此提供商指定模型；导入后请为提供商添加模型。".into());
            }
            providers.push(InventoryProvider {
                source_provider_id: id.clone(),
                name: definition["name"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .unwrap_or(id)
                    .to_owned(),
                access: Access::Http {
                    protocol: Protocol::Responses,
                    endpoint: endpoint.to_owned(),
                    key,
                },
                models,
                issues,
            });
        }
    }
    if providers.is_empty() {
        warnings.push("config.toml 中没有可导入的自定义 model_providers。".into());
    }
    Ok(Inventory {
        providers,
        warnings,
        fingerprint: fingerprint(&[
            "codex",
            &codex_home.to_string_lossy(),
            text.as_deref().unwrap_or(""),
        ]),
        source_label: path.display().to_string(),
    })
}
