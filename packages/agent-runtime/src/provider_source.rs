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

pub fn validate_source(source: &Source) -> Result<(), String> {
    if source.kind != "harness" || source.harness_type_id != "pi" {
        return Err(String::from("unsupported provider import source"));
    }
    if !source.settings.contains_key("agentDir") && !source.settings.contains_key("sourceDir") {
        return Err(String::from("provider import source path"));
    }
    let value = source
        .settings
        .get("nodeBinary")
        .ok_or_else(|| String::from("provider import source path"))?;
    if !Path::new(value).is_absolute() || value.contains('\0') {
        return Err(String::from("provider import source path"));
    }
    if let Some(path) = source.settings.get("modelsPath")
        && (!Path::new(path).is_absolute() || path.contains('\0'))
    {
        return Err(String::from("provider import models path"));
    }
    if let Some(path) = source.settings.get("authPath")
        && (!Path::new(path).is_absolute() || path.contains('\0'))
    {
        return Err(String::from("provider import auth path"));
    }
    Ok(())
}
pub fn read(source: &Source, resources_directory: &Path) -> Result<Snapshot, String> {
    validate_source(source)?;
    let node = source.settings.get("nodeBinary").expect("validated");
    let helper = resources_directory.join("pi_provider_import.mjs");
    let output = Command::new(node)
        .arg(&helper)
        .arg("--source-json")
        .arg(serde_json::to_string(source).map_err(|_| "invalid Pi source metadata")?)
        .env_clear()
        .output()
        .map_err(|_| "Pi provider source could not start")?;
    if !output.status.success() {
        return Err(String::from("Pi provider import failed"));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| String::from("invalid Pi provider import snapshot"))
}
