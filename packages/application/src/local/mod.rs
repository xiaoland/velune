//! Cross-platform application runtime.
//!
//! This module owns ordinary application configuration and the active
//! Pi/gateway projection. It has no filesystem socket, SQLite, environment, or
//! platform credential access beyond explicit paths supplied at open.

use crate::{Error as RuntimeError, Options as RuntimeOptions};
use crate::{
    config::{GatewayConfig, GatewayProtocol, ModelDefinition, RuntimeInstance},
    conversation::{ConversationSummary, RunState},
    provider_import,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use velune_agent_runtime::{self as pi, Config as PiConfig, PiProjection};
use velune_gateway::Runner;

const CONTRACT_VERSION: u64 = 3;

fn runtime_types(resources_directory: &Path) -> Value {
    let bundled_binary =
        resources_directory.join("node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js");
    let binary_value = if bundled_binary.is_file() {
        bundled_binary.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    json!([{
        "id":"pi",
        "name":"Pi Agent 运行时",
        "fields":[
            {"key":"binary","label":"运行时入口","kind":"filePath","required":true,"value":binary_value,"options":[],"help":"Pi CLI 的绝对路径；应用不会使用全局 PATH。"},
            {"key":"nodeBinary","label":"Node 可执行文件","kind":"filePath","required":true,"value":"","options":[],"executableDiscovery":{"command":"node","minimumVersion":"22.19.0"},"help":"Node 22.19+ 的绝对路径；Pi SDK 接入必填。"},
            {"key":"agentDir","label":"运行时目录","kind":"directoryPath","required":true,"value":"","options":[],"help":"配置与状态根目录，不是会话项目目录。"},
            {"key":"sessionDir","label":"会话存储目录","kind":"directoryPath","required":false,"value":"","options":[],"help":"覆盖 Pi 默认的会话文件存储目录。留空时保存到运行时目录的 sessions 下，并按会话工作目录分组；此项不是工作目录。"}
        ],
        "actions":[{"id":"connect","label":"连接运行时"}]
    }])
}

pub struct CoreRuntime {
    options: RuntimeOptions,
    repository: crate::repository::Repository,
    gateways: Vec<GatewayConfig>,
    authentication_resources: crate::authentication_resources::AuthenticationManager,
    authentication_binding_id: Option<String>,
    runtime_instances: Vec<RuntimeInstance>,
    pi: Option<pi::Client>,
    pi_config: Option<PiConfig>,
    pi_busy: bool,
    projection: Option<PiProjection>,
    gateway_runner: Option<Runner>,
    active_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    physical_model_id: Option<String>,
    logical_model_id: Option<String>,
    subscription_capability: bool,
    pi_turn_started: bool,
}

mod authentication_coordination;
mod authentication_management;
mod configuration;
mod connection;
mod conversations;
mod pi_composition;
use pi_composition::*;

impl CoreRuntime {
    pub fn open(options: RuntimeOptions) -> Result<Self, RuntimeError> {
        options.validate()?;
        let (repository, persisted) = crate::repository::Repository::open(&options.home_directory)?;
        Ok(Self {
            options,
            repository,
            gateways: persisted.gateways,
            authentication_resources: crate::authentication_resources::AuthenticationManager {
                resources: persisted.authentication_bindings,
            },
            authentication_binding_id: None,
            runtime_instances: persisted.runtime_instances,
            pi: None,
            pi_config: None,
            pi_busy: false,
            projection: None,
            gateway_runner: None,
            active_runtime_id: None,
            authentication: None,
            physical_model_id: None,
            logical_model_id: None,
            subscription_capability: false,
            pi_turn_started: false,
        })
    }

    pub fn close_if_idle(&mut self) -> Result<(), RuntimeError> {
        self.drain_pi();
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.shutdown_active()?;
        // Release the lock explicitly; a concurrent child spawn can briefly
        // inherit the open file description before exec closes its descriptors.
        self.repository.unlock()?;
        Ok(())
    }

    pub(crate) fn request_inner(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.drain_pi();
        if request["version"] != CONTRACT_VERSION {
            return Err(RuntimeError::Unsupported(
                "projection contract version".into(),
            ));
        }
        let action = request["action"].as_str().unwrap_or_default();
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            && !matches!(action, "authentication" | "list" | "getSnapshot")
        {
            return Err(RuntimeError::invalid("authentication is running"));
        }
        match action {
            "list" => self.list(),
            "providerImport" => self.provider_import_action(request),
            "gateways" => self.gateway_action(request),
            "runtimeInstances" => self.runtime_action(request),
            "runtimeAction" | "connect" => self.connect_action(request),
            "create" => self.create_conversation(request),
            "open" => self.open_conversation(request),
            "getSnapshot" => {
                self.ensure_active(request)?;
                self.sync_projection()?;
                Ok(
                    json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}),
                )
            }
            "send" => self.send_action(request),
            "cancel" => self.cancel_action(request),
            "selectModel" => self.select_model(request),
            "authenticationResources" => self.authentication_resource_action(&request["payload"]),
            "authentication" => self.authentication_action(&request["payload"]),
            "resources" | "settings" | "beginAuthorization" => {
                Err(RuntimeError::Unsupported("legacy action".into()))
            }
            action => Err(RuntimeError::Unsupported(action.into())),
        }
    }
}

use velune_agent_runtime::authentication;
