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
    sync::Arc,
};
use velune_agent_runtime::history::{self, HistoryConfig};
use velune_agent_runtime::native::{GatewayInjection, NativeConfig, NativeKind, NativeSession};
use velune_agent_runtime::{self as pi, Config as PiConfig, PiProjection};
use velune_gateway::Runner;

const CONTRACT_VERSION: u64 = 3;
mod discovery;

fn runtime_types() -> Vec<crate::config::RuntimeTypeDescriptor> {
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
        "Node 22.19+ 的绝对路径；用于运行时接入与会话历史读取。",
    );
    node.executable_discovery = Some(crate::conversation::ExecutableDiscovery {
        command: "node".into(),
        minimum_version: "22.19.0".into(),
    });
    velune_agent_runtime::version::variants().iter().map(|variant| {
        let mut binary=field("binary","运行时入口","filePath",true,String::new(),"当前运行时版本的绝对可执行路径。执行准备时会核对实际版本。");
        if variant.family_id=="pi" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"pi".into(),minimum_version:"1.0.2".into()});}
        if variant.id=="codex-0.159.3" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"codex".into(),minimum_version:"0.159.3".into()});}
        if variant.id=="dsh-acp-0.2.0-rc.2" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"dsh".into(),minimum_version:"0.2.0".into()});}
        let mut fields=vec![binary,node.clone(),field("agentDir","运行时目录","directoryPath",true,String::new(),match variant.id {"codex-0.159.3"=>"该实例的 CODEX_HOME；配置与会话根目录，不是任务工作目录。","dsh-acp-0.2.0-rc.2"=>"该实例的 DeepSeek Harness 配置与会话根目录，不是任务工作目录。",_=>"该实例的 Pi 配置与状态根目录，不是任务工作目录。"})];
        if variant.id=="pi-1.0.2" {fields.push(field("sessionDir","会话存储目录","directoryPath",false,String::new(),"覆盖 Pi 默认的会话存储位置；留空采用运行时目录的 sessions，此项不是任务工作目录。"));}
        crate::config::RuntimeTypeDescriptor{id:variant.id.into(),family_id:variant.family_id.into(),version_regex:variant.version_regex.into(),can_rename_conversations:variant.can_rename_conversations,can_delete_conversations:variant.can_delete_conversations,supported_protocols:GatewayProtocol::runtime_protocols(variant.id).expect("registered runtime adapter"),supported_provider_protocols:GatewayProtocol::runtime_provider_protocols(variant.id).expect("registered runtime adapter"),name:variant.name.into(),fields}
    }).collect()
}

#[derive(Default)]
struct PiState {
    client: Option<pi::Client>,
    config: Option<PiConfig>,
    projection: Option<PiProjection>,
    busy: bool,
    turn_started: bool,
    physical_model_id: Option<String>,
    subscription_capability: bool,
}
enum ActiveState {
    Empty,
    History(Box<crate::conversation::ConversationSnapshot>),
    Pi,
    Native(Box<NativeSession>),
}

pub struct CoreRuntime {
    options: RuntimeOptions,
    repository: crate::repository::Repository,
    gateways: Vec<GatewayConfig>,
    model_templates: Vec<crate::config::ModelTemplate>,
    conversation_links: Vec<crate::config::ConversationLink>,
    logical_projection: Option<continuation::LogicalProjection>,
    conversation_browser_group_limit: u32,
    transcript_presentation: crate::config::TranscriptPresentation,
    conversation_browser_preferences: crate::config::ConversationBrowserPreferences,
    transcript_projection: velune_agent_runtime::transcript::TranscriptProjection,
    transcript_conversation_id: Option<String>,
    authentication_provider: Option<(String, String)>,
    runtime_instances: Vec<RuntimeInstance>,
    pi: PiState,
    active_state: ActiveState,
    gateway_runner: Option<Runner>,
    next_turn_runtime_id: Option<String>,
    execution_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    model_record_key: Option<String>,
    analytics: Arc<crate::analytics::AnalyticsStore>,
}

mod authentication_coordination;
mod browsing;
mod configuration;
mod continuation;
mod conversations;
mod execution;
mod native_composition;
mod pi_composition;
mod provider_management;
use pi_composition::*;

impl CoreRuntime {
    pub fn open(
        options: RuntimeOptions,
        analytics: Arc<crate::analytics::AnalyticsStore>,
    ) -> Result<Self, RuntimeError> {
        options.validate()?;
        let (repository, persisted) = crate::repository::Repository::open(&options.home_directory)?;
        let conversation_links = repository.load_conversation_links()?;
        Ok(Self {
            options,
            repository,
            gateways: persisted.gateways,
            model_templates: persisted.model_templates,
            conversation_links,
            logical_projection: None,
            conversation_browser_group_limit: persisted.conversation_browser_group_limit,
            transcript_presentation: persisted.transcript_presentation,
            conversation_browser_preferences: persisted.conversation_browser_preferences,
            transcript_projection: Default::default(),
            transcript_conversation_id: None,
            authentication_provider: None,
            runtime_instances: persisted.runtime_instances,
            pi: PiState::default(),
            active_state: ActiveState::Empty,
            gateway_runner: None,
            next_turn_runtime_id: None,
            execution_runtime_id: None,
            authentication: None,
            model_record_key: None,
            analytics,
        })
    }

    fn busy(&self) -> bool {
        match &self.active_state {
            ActiveState::Native(session) => session.busy(),
            ActiveState::Pi => self.pi.busy,
            ActiveState::Empty | ActiveState::History(_) => false,
        }
    }
    fn native_snapshot(&self) -> Option<crate::conversation::ConversationSnapshot> {
        let snapshot = match &self.active_state {
            ActiveState::Native(session) => session.snapshot(),
            ActiveState::Pi => self
                .pi
                .projection
                .as_ref()
                .and_then(|p| p.snapshot.as_ref()),
            ActiveState::History(snapshot) => Some(snapshot.as_ref()),
            ActiveState::Empty => None,
        };
        snapshot.cloned().map(|mut snapshot| {
            snapshot.model_record_key = self.model_record_key.clone();
            snapshot
        })
    }
    fn current_snapshot(&mut self) -> Option<crate::conversation::ConversationSnapshot> {
        use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
        let Some(native) = self.native_snapshot() else {
            self.transcript_projection = TranscriptProjection::default();
            self.transcript_conversation_id = None;
            return None;
        };
        let mut snapshot = self.decorate_snapshot(native);
        if self.transcript_conversation_id.as_deref() != Some(&snapshot.conversation.id) {
            self.transcript_projection = TranscriptProjection::default();
            self.transcript_conversation_id = Some(snapshot.conversation.id.clone());
        }
        self.transcript_projection
            .apply(TranscriptEvent::ConfirmIdentities(
                &snapshot.message_identity_confirmations,
            ));
        self.transcript_projection
            .apply(TranscriptEvent::ReplaceMessages(&snapshot.messages));
        self.transcript_projection
            .apply(TranscriptEvent::ExecutionState(snapshot.run_state.clone()));
        snapshot.transcript_turns = self.transcript_projection.turns();
        Some(snapshot)
    }
    fn drain_runtime(&mut self) -> Result<(), RuntimeError> {
        match &mut self.active_state {
            ActiveState::Native(session) => session
                .poll()
                .map_err(|error| RuntimeError::Invalid(format!("运行时事件读取失败：{error}"))),
            ActiveState::Pi => {
                self.drain_pi();
                Ok(())
            }
            ActiveState::Empty | ActiveState::History(_) => Ok(()),
        }
    }
    fn public_gateways(&self) -> Vec<crate::GatewaySummary> {
        crate::provider_configuration::summaries(&self.gateways)
    }
    pub fn close_if_idle(&mut self) -> Result<(), RuntimeError> {
        self.drain_runtime()?;
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.shutdown_active()?;
        // Release the lock explicitly; a concurrent child spawn can briefly
        // inherit the open file description before exec closes its descriptors.
        self.repository.unlock()?;
        Ok(())
    }

    pub(crate) fn request_inner(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.drain_runtime()?;
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
            "transcriptPresentation" => self.set_transcript_presentation(request),
            "conversationBrowserPreferences" => self.set_conversation_browser_preferences(request),
            "conversationBrowserSettings" => self.set_conversation_browser_group_limit(request),
            "discoverRuntimes" => self.discover_runtimes(request),
            "providerImport" => self.provider_import_action(request),
            "providers" => self.provider_action(&request["payload"]),
            "modelTemplates" => self.template_action(&request["payload"]),
            "runtimeInstances" => self.runtime_action(request),
            "selectRuntime" => self.select_runtime_action(request),
            "create" => self.create_conversation(request),
            "open" => self.open_conversation(request),
            "renameConversation" => self.manage_conversation(request, false),
            "deleteConversation" => self.manage_conversation(request, true),
            "getSnapshot" => {
                if matches!(self.active_state, ActiveState::Empty) {
                    let id = request["payload"]["runtimeInstanceID"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
                    if !self.runtime_instance(id)?.enabled {
                        return Err(RuntimeError::invalid("此运行时已停用"));
                    }
                    return Ok(json!({"snapshot":null}));
                }
                self.ensure_active(request)?;
                self.sync_projection()?;
                Ok(json!({"snapshot":self.current_snapshot()}))
            }
            "sendTurn" => self.send_turn_action(request),
            "cancel" => self.cancel_action(request),
            "replyRuntimeInteraction" => self.reply_runtime_interaction(request),
            "authentication" => self.authentication_action(&request["payload"]),
            action => Err(RuntimeError::Unsupported(action.into())),
        }
    }
}

use velune_agent_runtime::authentication;
