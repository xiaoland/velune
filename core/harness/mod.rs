//! Harness adapter boundary. Native clients consume the generic projection;
//! runtime-specific Pi transport stays behind this module.

pub use crate::conversation::{
    Connection, ConversationActions, ConversationSnapshot, ConversationSummary, Message,
    MessageBlock, ModelOption, ModelResource, PiProjection, RunState, SettingAction,
    SettingCategory, SettingField, SettingOption,
};
pub use crate::pi::{Client as PiClient, Config as PiConfig};

use std::{collections::BTreeMap, path::PathBuf};

/// Convert generic app-owned settings into the Pi adapter's launch config.
/// Provider/model identifiers stay opaque to the generic client and are only
/// interpreted here, at the native adapter boundary.
pub fn config_from_resource(
    resource: &ModelResource,
    values: &BTreeMap<String, String>,
) -> Result<PiConfig, &'static str> {
    let binary = values.get("binary").cloned().unwrap_or_default();
    if binary.is_empty() {
        return Err("runtime entry is required");
    }
    let path = |key: &str| {
        values
            .get(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let model = resource
        .models
        .first()
        .ok_or("resource model is required")?;
    let service_id = service_id_for_resource(resource);
    Ok(PiConfig {
        binary: PathBuf::from(binary),
        node_binary: path("nodeBinary").or_else(|| path("node_binary")),
        sdk_helper: path("sdkHelper").or_else(|| path("sdk_helper")),
        extension: None,
        agent_dir: path("agentDir").or_else(|| path("agent_dir")),
        working_dir: path("workingDir").or_else(|| path("working_dir")),
        provider: Some(service_id),
        model: Some(model.id.clone()),
        protocol: resource.protocol_id.clone(),
        endpoint: resource.endpoint.clone(),
        credential_ref: resource.credential_ref.clone(),
        credential_resolver: path("credentialResolver").or_else(|| path("credential_resolver")),
        gateway_token: None,
        session_dir: path("sessionDir").or_else(|| path("session_dir")),
        session: path("session"),
        name: None,
    })
}

pub fn service_id_for_resource(resource: &ModelResource) -> String {
    resource
        .service_id
        .clone()
        .unwrap_or_else(|| format!("velune-{}", stable_namespace(&resource.id)))
}

fn stable_namespace(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
