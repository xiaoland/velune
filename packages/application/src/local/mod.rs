//! Cross-platform application runtime.
//!
//! This module owns ordinary application configuration and the active
//! Pi/gateway projection. It has no filesystem socket, SQLite, environment, or
//! platform credential access beyond explicit paths supplied at open.

use crate::{Error as RuntimeError, Options as RuntimeOptions};
use crate::{
    config::{GatewayConfig, GatewayProtocol, ProviderModel, RuntimeInstance},
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

fn runtime_types(resources_directory: &Path) -> Vec<crate::config::RuntimeTypeDescriptor> {
    let bundled_binary =
        resources_directory.join("node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js");
    let binary_value = if bundled_binary.is_file() {
        bundled_binary.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    let field = |key: &str, label: &str, kind: &str, required: bool, value: String, help: &str| {
        crate::conversation::SettingField {
            key: key.into(),
            label: label.into(),
            kind: kind.into(),
            required,
            value,
            options: Vec::new(),
            help: Some(help.into()),
            executable_discovery: None,
        }
    };
    let mut node = field(
        "nodeBinary",
        "Node 可执行文件",
        "filePath",
        true,
        String::new(),
        "Node 22.19+ 的绝对路径；Pi SDK 接入必填。",
    );
    node.executable_discovery = Some(crate::conversation::ExecutableDiscovery {
        command: "node".into(),
        minimum_version: "22.19.0".into(),
    });
    vec![crate::config::RuntimeTypeDescriptor {
        id: "pi".into(),
        name: "Pi Agent 运行时".into(),
        fields: vec![
            field(
                "binary",
                "运行时入口",
                "filePath",
                true,
                binary_value,
                "Pi CLI 的绝对路径；应用不会使用全局 PATH。",
            ),
            node,
            field(
                "agentDir",
                "运行时目录",
                "directoryPath",
                true,
                String::new(),
                "配置与状态根目录，不是会话项目目录。",
            ),
            field(
                "sessionDir",
                "会话存储目录",
                "directoryPath",
                false,
                String::new(),
                "覆盖 Pi 默认的会话文件存储目录。留空时保存到运行时目录的 sessions 下，并按会话工作目录分组；此项不是工作目录。",
            ),
        ],
        actions: vec![crate::conversation::SettingAction {
            id: "connect".into(),
            label: "连接运行时".into(),
        }],
    }]
}

pub struct CoreRuntime {
    options: RuntimeOptions,
    repository: crate::repository::Repository,
    gateways: Vec<GatewayConfig>,
    model_templates: Vec<crate::config::ModelTemplate>,
    authentication_provider: Option<(String, String)>,
    runtime_instances: Vec<RuntimeInstance>,
    pi: Option<pi::Client>,
    pi_config: Option<PiConfig>,
    pi_busy: bool,
    projection: Option<PiProjection>,
    gateway_runner: Option<Runner>,
    active_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    physical_model_id: Option<String>,
    model_record_key: Option<String>,
    subscription_capability: bool,
    pi_turn_started: bool,
}

mod authentication_coordination;
mod configuration;
mod connection;
mod conversations;
mod pi_composition;
mod provider_management;
use pi_composition::*;

impl CoreRuntime {
    pub fn open(options: RuntimeOptions) -> Result<Self, RuntimeError> {
        options.validate()?;
        let (repository, persisted) = crate::repository::Repository::open(&options.home_directory)?;
        Ok(Self {
            options,
            repository,
            gateways: persisted.gateways,
            model_templates: persisted.model_templates,
            authentication_provider: None,
            runtime_instances: persisted.runtime_instances,
            pi: None,
            pi_config: None,
            pi_busy: false,
            projection: None,
            gateway_runner: None,
            active_runtime_id: None,
            authentication: None,
            physical_model_id: None,
            model_record_key: None,
            subscription_capability: false,
            pi_turn_started: false,
        })
    }

    fn public_gateways(&self) -> Vec<crate::GatewaySummary> {
        crate::provider_configuration::summaries(&self.gateways)
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
            "providers" => self.provider_action(&request["payload"]),
            "modelTemplates" => self.template_action(&request["payload"]),
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
            "authentication" => self.authentication_action(&request["payload"]),
            action => Err(RuntimeError::Unsupported(action.into())),
        }
    }
}

use velune_agent_runtime::authentication;
