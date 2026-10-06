//! Read-only metadata discovery through the Pi SDK source adapter.
use crate::model_projection::PiModelProjection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, process::Command};
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: String,
    pub harness_type_id: String,
    #[serde(default)]
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub contract_version: u64,
    pub sdk_version: String,
    pub models_path: String,
    pub auth_path: Option<String>,
    pub providers: Vec<ImportedProvider>,
    pub source_fingerprint: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedProvider {
    pub source_provider_id: String,
    pub name: String,
    pub endpoint: Option<String>,
    pub auth: ImportedAuth,
    pub models: Vec<ImportedModel>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAuth {
    pub kind: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub status_label: Option<String>,
    pub ready: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedModel {
    pub source_provider_id: String,
    pub id: String,
    pub name: String,
    pub api: String,
    pub protocol: Option<String>,
    pub input: Vec<String>,
    pub reasoning: bool,
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    pub thinking_level_map: Option<Value>,
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub cost: Option<Value>,
    pub prompt_cache: Option<Value>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    #[serde(default)]
    pub unsupported_features: Vec<String>,
    #[serde(default)]
    pub pi_projection: Option<PiModelProjection>,
}

/// Allowlisted discovery phases; subprocess output cannot introduce arbitrary labels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePhase {
    Input,
    Node,
    SourceDirectory,
    Sdk,
    Models,
    Auth,
    ModelRuntime,
    Projection,
    CredentialBinding,
    Spawn,
    Protocol,
    Unknown,
}
/// Safe diagnostic categories, distinct from SDK error messages and configuration values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDiagnosticCode {
    InvalidSource,
    StaleBinding,
    InvalidSourceDirectory,
    InvalidNodePath,
    InvalidModelsPath,
    InvalidAuthPath,
    SourceDirectoryMissing,
    SourceDirectoryPermission,
    SourceDirectoryUnavailable,
    SourceDirectoryNotDirectory,
    UnsupportedNode,
    UnsupportedSdk,
    HelperUnavailable,
    InvalidModels,
    InvalidModelRuntime,
    SpawnFailed,
    InvalidOutput,
    AdapterFailed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Diagnostic {
    code: SourceDiagnosticCode,
    phase: SourcePhase,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticEnvelope {
    contract_version: u64,
    diagnostic: Diagnostic,
    #[serde(rename = "detail", default)]
    _detail: String,
}
#[derive(Debug, Clone)]
pub struct SourceReadError {
    diagnostic: Diagnostic,
    exit_status: Option<i32>,
    detail: String,
}
impl SourceReadError {
    fn new(code: SourceDiagnosticCode, phase: SourcePhase) -> Self {
        Self {
            diagnostic: Diagnostic { code, phase },
            exit_status: None,
            detail: String::new(),
        }
    }
    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
    pub fn code(&self) -> &'static str {
        match self.diagnostic.code {
            SourceDiagnosticCode::InvalidSource => "invalid_source",
            SourceDiagnosticCode::StaleBinding => "stale_binding",
            SourceDiagnosticCode::InvalidSourceDirectory => "invalid_source_directory",
            SourceDiagnosticCode::InvalidNodePath => "invalid_node_path",
            SourceDiagnosticCode::InvalidModelsPath => "invalid_models_path",
            SourceDiagnosticCode::InvalidAuthPath => "invalid_auth_path",
            SourceDiagnosticCode::SourceDirectoryMissing => "source_directory_missing",
            SourceDiagnosticCode::SourceDirectoryPermission => "source_directory_permission",
            SourceDiagnosticCode::SourceDirectoryUnavailable => "source_directory_unavailable",
            SourceDiagnosticCode::SourceDirectoryNotDirectory => "source_directory_not_directory",
            SourceDiagnosticCode::UnsupportedNode => "unsupported_node",
            SourceDiagnosticCode::UnsupportedSdk => "unsupported_sdk",
            SourceDiagnosticCode::HelperUnavailable => "helper_unavailable",
            SourceDiagnosticCode::InvalidModels => "invalid_models",
            SourceDiagnosticCode::InvalidModelRuntime => "invalid_model_runtime",
            SourceDiagnosticCode::SpawnFailed => "spawn_failed",
            SourceDiagnosticCode::InvalidOutput => "invalid_output",
            SourceDiagnosticCode::AdapterFailed => "adapter_failed",
        }
    }
    pub fn phase(&self) -> &'static str {
        match self.diagnostic.phase {
            SourcePhase::Input => "input",
            SourcePhase::Node => "node",
            SourcePhase::SourceDirectory => "source_directory",
            SourcePhase::Sdk => "sdk",
            SourcePhase::Models => "models",
            SourcePhase::Auth => "auth",
            SourcePhase::ModelRuntime => "model_runtime",
            SourcePhase::Projection => "projection",
            SourcePhase::CredentialBinding => "credential_binding",
            SourcePhase::Spawn => "spawn",
            SourcePhase::Protocol => "protocol",
            SourcePhase::Unknown => "unknown",
        }
    }
    pub fn exit_status(&self) -> Option<i32> {
        self.exit_status
    }
    pub fn detail(&self) -> &str {
        &self.detail
    }
    pub fn message(&self) -> &'static str {
        match self.diagnostic.code {
            SourceDiagnosticCode::InvalidSource => "所选运行时的提供商来源配置无效。",
            SourceDiagnosticCode::StaleBinding => {
                "已导入的模型执行配置已失效；请重新预览并导入该提供商。"
            }
            SourceDiagnosticCode::InvalidSourceDirectory => {
                "运行时目录必须是绝对路径；请在运行时配置中修正。"
            }
            SourceDiagnosticCode::InvalidNodePath => {
                "Node 可执行文件必须是绝对路径；请在运行时配置中修正。"
            }
            SourceDiagnosticCode::InvalidModelsPath => "模型配置文件必须是绝对路径。",
            SourceDiagnosticCode::InvalidAuthPath => "认证来源文件必须是绝对路径。",
            SourceDiagnosticCode::SourceDirectoryMissing => {
                "运行时目录不存在；请检查运行时目录配置。"
            }
            SourceDiagnosticCode::SourceDirectoryPermission => "没有读取运行时目录的权限。",
            SourceDiagnosticCode::SourceDirectoryUnavailable => "无法读取运行时目录。",
            SourceDiagnosticCode::SourceDirectoryNotDirectory => {
                "运行时目录配置指向了普通文件；请选择目录。"
            }
            SourceDiagnosticCode::UnsupportedNode => "提供商导入需要 Node 22.19.0 或更高版本。",
            SourceDiagnosticCode::UnsupportedSdk => {
                "应用中的 Pi SDK 版本不受支持，请重新安装应用。"
            }
            SourceDiagnosticCode::HelperUnavailable => {
                "应用中的提供商读取组件不完整，请重新安装应用。"
            }
            SourceDiagnosticCode::InvalidModels => {
                "Pi 模型配置无法解析；请检查 models.json 的格式。"
            }
            SourceDiagnosticCode::InvalidModelRuntime => {
                "Pi 模型目录不能建立；请检查来源配置的协议及模型定义。"
            }
            SourceDiagnosticCode::SpawnFailed => {
                "无法启动 Node 提供商读取组件；请检查 Node 路径与执行权限。"
            }
            SourceDiagnosticCode::InvalidOutput => "提供商读取组件返回了无效结果，请检查应用安装。",
            SourceDiagnosticCode::AdapterFailed => match self.diagnostic.phase {
                SourcePhase::Sdk => "无法加载应用中的 Pi SDK，请检查应用安装。",
                SourcePhase::Models => "无法读取 Pi 模型配置；请检查文件权限及格式。",
                SourcePhase::Auth => "无法读取 Pi 认证来源的元数据；请检查文件权限及格式。",
                SourcePhase::ModelRuntime => "无法建立 Pi 模型目录，请检查来源配置。",
                _ => "Pi 提供商来源读取失败；请查看诊断记录中的阶段与错误类别。",
            },
        }
    }
}
impl std::fmt::Display for SourceReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())?;
        if !self.detail.is_empty() {
            write!(f, " 原始错误：{}", self.detail)?;
        }
        Ok(())
    }
}
impl std::error::Error for SourceReadError {}

pub fn validate_source(source: &Source) -> Result<(), SourceReadError> {
    if source.kind != "harness" || source.harness_type_id != "pi" {
        return Err(SourceReadError::new(
            SourceDiagnosticCode::InvalidSource,
            SourcePhase::Input,
        ));
    }
    let directory = source
        .settings
        .get("sourceDir")
        .or_else(|| source.settings.get("agentDir"));
    for (path, code) in [
        (directory, SourceDiagnosticCode::InvalidSourceDirectory),
        (
            source.settings.get("nodeBinary"),
            SourceDiagnosticCode::InvalidNodePath,
        ),
    ] {
        if path.is_none_or(|path| !Path::new(path).is_absolute() || path.contains('\0')) {
            return Err(SourceReadError::new(code, SourcePhase::Input));
        }
    }
    for (key, code) in [
        ("modelsPath", SourceDiagnosticCode::InvalidModelsPath),
        ("authPath", SourceDiagnosticCode::InvalidAuthPath),
    ] {
        if source
            .settings
            .get(key)
            .is_some_and(|path| !Path::new(path).is_absolute() || path.contains('\0'))
        {
            return Err(SourceReadError::new(code, SourcePhase::Input));
        }
    }
    Ok(())
}
pub fn read(source: &Source, resources_directory: &Path) -> Result<Snapshot, SourceReadError> {
    let span = tracing::info_span!("provider_source.read", source_type = "pi");
    let _entered = span.enter();
    let result = read_inner(source, resources_directory);
    match &result {
        Ok(snapshot) => tracing::info!(
            provider_count = snapshot.providers.len(),
            "provider source metadata read"
        ),
        Err(error) => tracing::warn!(
            code = error.code(),
            phase = error.phase(),
            exit_status = error.exit_status(),
            "provider source discovery failed"
        ),
    }
    result
}
fn read_inner(source: &Source, resources_directory: &Path) -> Result<Snapshot, SourceReadError> {
    validate_source(source)?;
    let node = source.settings.get("nodeBinary").expect("validated");
    let helper = resources_directory.join("pi_provider_import.mjs");
    if !helper.is_file() {
        return Err(SourceReadError::new(
            SourceDiagnosticCode::HelperUnavailable,
            SourcePhase::Sdk,
        )
        .with_detail(format!("helper 路径：{}", helper.display())));
    }
    let output = Command::new(node)
        .arg(&helper)
        .arg("--source-json")
        .arg(serde_json::to_string(source).map_err(|error| {
            SourceReadError::new(SourceDiagnosticCode::InvalidSource, SourcePhase::Input)
                .with_detail(error.to_string())
        })?)
        .env_clear()
        .output()
        .map_err(|error| {
            SourceReadError::new(SourceDiagnosticCode::SpawnFailed, SourcePhase::Spawn).with_detail(
                format!("{}；Node：{}；helper：{}", error, node, helper.display()),
            )
        })?;
    if !output.status.success() {
        // The envelope supplies stable classification; the complete stderr is
        // retained as the local user's actionable underlying cause.
        let diagnostic = (output.stderr.len() <= 8192)
            .then(|| serde_json::from_slice::<DiagnosticEnvelope>(&output.stderr).ok())
            .flatten()
            .filter(|value| value.contract_version == 1)
            .map(|value| value.diagnostic)
            .unwrap_or(Diagnostic {
                code: SourceDiagnosticCode::AdapterFailed,
                phase: SourcePhase::Unknown,
            });
        return Err(SourceReadError {
            diagnostic,
            exit_status: output.status.code(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let snapshot: Snapshot = serde_json::from_slice(&output.stdout).map_err(|error| {
        SourceReadError::new(SourceDiagnosticCode::InvalidOutput, SourcePhase::Protocol)
            .with_detail(error.to_string())
    })?;
    if snapshot.contract_version != 1 || snapshot.sdk_version != "1.0.2" {
        return Err(SourceReadError::new(
            SourceDiagnosticCode::InvalidOutput,
            SourcePhase::Protocol,
        )
        .with_detail(format!(
            "contractVersion={}，sdkVersion={}",
            snapshot.contract_version, snapshot.sdk_version
        )));
    }
    Ok(snapshot)
}
