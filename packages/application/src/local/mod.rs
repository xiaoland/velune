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
    path::{Path, PathBuf},
    sync::Arc,
};
use velune_agent_runtime::history::{self, HistoryConfig};
use velune_agent_runtime::native::{GatewayInjection, NativeConfig, NativeKind, NativeSession};
use velune_agent_runtime::{self as pi, Config as PiConfig, PiProjection};
use velune_gateway::Runner;

const CONTRACT_VERSION: u64 = 3;
mod acp_composition;
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
        if variant.id=="acp-v1" {binary.help=Some("支持 ACP v1 的命令或 bridge 的绝对路径；启动参数与环境变量在下方设置，启动时协商协议。".into());}
        if variant.family_id=="pi" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"pi".into(),minimum_version:"1.0.2".into()});}
        if variant.id=="codex-0.159" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"codex".into(),minimum_version:"0.159.3".into()});}
        if variant.id=="dsh-acp-0.2" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"dsh".into(),minimum_version:"0.2.0".into()});}
        let mut runtime_node = node.clone();
        runtime_node.required = variant.family_id != "claude-code";
        if variant.family_id == "claude-code" { binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"claude".into(),minimum_version:"2.1.286".into()}); }
        let mut fields=vec![binary,runtime_node,field("agentDir","运行时目录","directoryPath",true,String::new(),match variant.id {"claude-code-2.1"=>"该实例的 CLAUDE_CONFIG_DIR；配置与原生会话目录，不是任务工作目录。","codex-0.159"=>"该实例的 CODEX_HOME；配置与会话根目录，不是任务工作目录。","dsh-acp-0.2"=>"该实例的 DeepSeek Harness 配置与会话根目录，不是任务工作目录。",_=>"该实例的 Pi 配置与状态根目录，不是任务工作目录。"})];
        if variant.id=="acp-v1" {fields.retain(|field| field.key=="binary");}
        if variant.id=="pi-1.0" {fields.push(field("sessionDir","会话存储目录","directoryPath",false,String::new(),"覆盖 Pi 默认的会话存储位置；留空采用运行时目录的 sessions，此项不是任务工作目录。"));}
        crate::config::RuntimeTypeDescriptor{id:variant.id.into(),family_id:variant.family_id.into(),compatible_versions:variant.versions.describe(),can_rename_conversations:variant.can_rename_conversations,can_delete_conversations:variant.can_delete_conversations,launch_options:variant.id=="acp-v1",agent_info:variant.id=="acp-v1",session_options:variant.id=="acp-v1",remote_resources:variant.id!="acp-v1",supported_protocols:crate::config::runtime_protocols(variant.id).expect("registered runtime adapter"),supported_provider_protocols:crate::config::runtime_provider_protocols(variant.id).expect("registered runtime adapter"),name:variant.name.into(),fields,permission_modes:variant.permission_modes.iter().map(|mode| crate::config::PermissionMode{id:mode.id.into(),label:mode.label.into(),detail:mode.detail.into()}).collect()}
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
}
#[derive(Default)]
enum ActiveState {
    #[default]
    Empty,
    History(Box<crate::conversation::ConversationSnapshot>),
    Pi,
    Native(Box<NativeSession>),
    Acp(Box<velune_agent_runtime::acp::Client>),
}

struct UnsubmittedDraft {
    public_id: String,
    original_runtime_id: String,
    runtime_id: String,
    native_id: String,
}

pub struct CoreRuntime {
    options: RuntimeOptions,
    client_baseline: Option<Box<ClientState>>,
    local_presentation_commit: std::cell::RefCell<Option<crate::repository::PresentationDefaults>>,
    // Fields drop in declaration order: stop the collaboration service and
    // execution owners before releasing the repository's exclusive
    // application-home lock.
    collaboration: collaboration::Broker,
    /// The conversation session requests currently address; see `session`.
    session: session::Session,
    /// Other open conversation sessions, most recently used last.
    parked: Vec<session::Session>,
    elsewhere: live::ElsewhereScan,
    finished_turns: session::FinishedTurns,
    gateways: Vec<GatewayConfig>,
    model_templates: Vec<crate::config::ModelTemplate>,
    conversation_links: Vec<crate::config::ConversationLink>,
    conversation_browser_limits: crate::api::ConversationBrowserLimits,
    organization: crate::config::ConversationOrganization,
    browser_window: organization::BrowserWindow,
    /// The latest listing's conversations by public identity, for organizing
    /// what the list showed.
    last_listed: std::cell::RefCell<BTreeMap<String, organization::ListedSource>>,
    /// The runtimes' own archives, listed while the archive view shows.
    archived_listing: std::cell::RefCell<Vec<ConversationSummary>>,
    /// Conversations no listing holds, reached by identity (a native
    /// sub-agent its runtime archived), as last described, for opening them.
    reached: std::cell::RefCell<BTreeMap<String, organization::ListedSource>>,
    outline_presentation: crate::config::OutlinePresentation,
    linked_conversation_opening: crate::config::LinkedConversationOpening,
    usage_display: crate::config::UsageDisplay,
    status_line: Vec<crate::config::StatusLineItem>,
    composer_drafts: BTreeMap<String, String>,
    composer_choices: crate::config::ComposerChoices,
    turn_requests: TurnRequests,
    resource_templates: Vec<crate::ResourceTemplate>,
    resource_bindings: Vec<crate::ResourceBinding>,
    /// A resource save could not reach its runtimes because one was busy; retried once idle.
    resource_sync_due: bool,
    conversation_browser_preferences: crate::config::ConversationBrowserPreferences,
    subscription_login: Option<crate::provider_login::Login>,
    authentication_provider: Option<(String, String)>,
    runtime_instances: Vec<RuntimeInstance>,
    next_turn_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    acp_capability_cache: std::cell::RefCell<BTreeMap<String, crate::AcpCapabilities>>,
    history_listings: std::cell::RefCell<BTreeMap<String, browsing::HistoryListing>>,
    delivery: delivery::TranscriptDelivery,
    prepared_remote_resources: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// HOME of each inherited-HOME SSH location, by its execution settings.
    remote_homes: std::cell::RefCell<BTreeMap<String, Option<PathBuf>>>,
    /// Whether each listed native sub-agent's latest turn failed.
    sub_agent_failures: std::cell::RefCell<BTreeMap<String, bool>>,
    /// Whether each session file's latest turn failed, as of its signature.
    /// The repository each folder on this Mac is a linked worktree of, if any.
    worktree_origins: std::cell::RefCell<BTreeMap<String, Option<String>>>,
    turn_failures:
        std::cell::RefCell<BTreeMap<PathBuf, (velune_agent_runtime::activity::Signature, bool)>>,
    analytics: Arc<crate::analytics::AnalyticsStore>,
    /// Shared admission, recovery and session affinity across gateway runners.
    scheduling_state: velune_gateway::SchedulingState,
    execution_backends: velune_gateway::ExecutionBackends,
    subscription_quota: Arc<crate::subscription_quota::SubscriptionQuota>,
    repository: crate::repository::Repository,
}

mod attachments;
mod client_state;
pub(crate) use client_state::ClientState;
mod authentication_coordination;
mod browsing;
mod collaboration;
mod composer_choices;
pub(crate) use composer_choices::{TurnKey, TurnRequests};
mod configuration;
mod continuation;
mod conversations;
mod delivery;
mod editing;
mod execution;
mod follow_up;
use crate::SendQueueOperation;
mod collaborators;
mod live;
mod model_selection;
mod native_composition;
mod organization;
mod pi_composition;
mod projects;
mod provider_management;
mod resource_library;
mod runtime_execution;
mod runtime_management;
mod session;
mod sub_agents;
mod working_changes;
pub(crate) use organization::unix_ms_now;
use pi_composition::*;

impl CoreRuntime {
    pub fn open(
        options: RuntimeOptions,
        analytics: Arc<crate::analytics::AnalyticsStore>,
    ) -> Result<Self, RuntimeError> {
        options.validate()?;
        let (repository, persisted) = crate::repository::Repository::open(&options.home_directory)?;
        let conversation_links = repository.load_conversation_links()?;
        let composer_drafts = repository.load_composer_drafts()?;
        let composer_choices = repository.load_composer_choices()?;
        let turn_requests = repository.load_turn_requests()?;
        let organization = repository.load_conversation_organization()?;
        let (resource_templates, resource_bindings) = repository.load_resource_library()?;
        let scheduling_state = velune_gateway::SchedulingState::default();
        let subscription_quota = crate::subscription_quota::SubscriptionQuota::new(
            options.home_directory.clone(),
            &persisted.gateways,
            scheduling_state.clone(),
        );
        Ok(Self {
            options,
            collaboration: Default::default(),
            session: Default::default(),
            parked: Vec::new(),
            elsewhere: live::ElsewhereScan::default(),
            finished_turns: session::FinishedTurns::default(),
            repository,
            gateways: persisted.gateways,
            model_templates: persisted.model_templates,
            conversation_links,
            conversation_browser_limits: crate::api::ConversationBrowserLimits {
                page: persisted.conversation_browser_group_limit,
                total: persisted.conversation_browser_total_limit,
            },
            organization,
            browser_window: Default::default(),
            client_baseline: None,
            local_presentation_commit: std::cell::RefCell::new(None),
            last_listed: Default::default(),
            archived_listing: Default::default(),
            reached: Default::default(),
            outline_presentation: persisted.outline_presentation,
            linked_conversation_opening: persisted.linked_conversation_opening,
            usage_display: persisted.usage_display,
            status_line: crate::config::canonical_status_line(persisted.status_line),
            composer_drafts,
            composer_choices,
            turn_requests,
            resource_templates,
            resource_sync_due: !resource_bindings.is_empty(),
            resource_bindings,
            conversation_browser_preferences: persisted.conversation_browser_preferences,
            subscription_login: None,
            authentication_provider: None,
            runtime_instances: persisted.runtime_instances,
            next_turn_runtime_id: None,
            authentication: None,
            acp_capability_cache: Default::default(),
            history_listings: Default::default(),
            delivery: delivery::TranscriptDelivery::new(),
            prepared_remote_resources: Default::default(),
            remote_homes: Default::default(),
            sub_agent_failures: Default::default(),
            worktree_origins: Default::default(),
            turn_failures: Default::default(),
            analytics,
            scheduling_state,
            execution_backends: Default::default(),
            subscription_quota,
        })
    }

    fn sync_scheduling(&self, previous: &[GatewayConfig]) {
        for gateway in previous {
            for provider in &gateway.providers {
                let current = self
                    .gateways
                    .iter()
                    .find(|g| g.id == gateway.id)
                    .and_then(|g| g.providers.iter().find(|p| p.id == provider.id));
                if current.is_some_and(|p| {
                    matches!(
                        p.request_profile,
                        crate::config::RequestProfile::KimiCoding
                            | crate::config::RequestProfile::ArkCoding
                    ) && provider.request_profile == p.request_profile
                        && p.endpoint == provider.endpoint
                        && p.protocol == provider.protocol
                        && p.execution_backend == provider.execution_backend
                        && p.quota_source == provider.quota_source
                        && matches!(
                            p.authentication.material,
                            crate::provider_authentication::AuthenticationMaterial::ApiKey { .. }
                        )
                        && matches!(
                            provider.authentication.material,
                            crate::provider_authentication::AuthenticationMaterial::ApiKey { .. }
                        )
                }) {
                    if current.is_some_and(|p| {
                        p.authentication != provider.authentication
                            || p.quota_authentication != provider.quota_authentication
                    }) {
                        self.scheduling_state
                            .invalidate_credentials(&gateway.id, &provider.id);
                    }
                    continue;
                }
                if current.is_none_or(|p| {
                    p.authentication != provider.authentication
                        || p.endpoint != provider.endpoint
                        || p.protocol != provider.protocol
                        || p.execution_backend != provider.execution_backend
                        || p.request_profile != provider.request_profile
                }) {
                    self.execution_backends
                        .invalidate_provider(&gateway.id, &provider.id);
                    self.scheduling_state
                        .invalidate_provider(&gateway.id, &provider.id);
                }
            }
        }
        self.subscription_quota.configure(&self.gateways);
    }
    fn turn_running(&self) -> bool {
        self.session.turn_running()
    }
    fn native_snapshot(&self) -> Option<crate::conversation::ConversationSnapshot> {
        self.session.projected().cloned().map(|mut snapshot| {
            snapshot.model_record_key = self.session.model_record_key.clone();
            snapshot
        })
    }
    fn current_snapshot(&mut self) -> Option<crate::conversation::ConversationSnapshot> {
        use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
        let Some(native) = self.native_snapshot() else {
            self.session.transcript_projection = TranscriptProjection::default();
            self.session.transcript_conversation_id = None;
            return None;
        };
        let mut snapshot = self.decorate_snapshot(native);
        continuation::stamp_turn_runtime(&mut snapshot.messages, &snapshot.context_runtime_id);
        if self.session.transcript_conversation_id.as_deref() != Some(&snapshot.conversation.id) {
            self.session.transcript_projection = TranscriptProjection::default();
            self.session.transcript_conversation_id = Some(snapshot.conversation.id.clone());
        }
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ConfirmIdentities(
                &snapshot.message_identity_confirmations,
            ));
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ReplaceMessages(&snapshot.messages));
        composer_choices::recall_turn_requests(
            &self.turn_requests,
            &mut self.session.transcript_projection,
            &snapshot,
        );
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ExecutionState(snapshot.run_state.clone()));
        self.keep_turn_requests(&snapshot);
        snapshot.transcript_turns = self.session.transcript_projection.turns();
        self.name_turn_models(&mut snapshot.transcript_turns);
        snapshot.transcript_items = self.session.transcript_projection.items();
        snapshot.transcript_outline = self.session.transcript_projection.outline();
        snapshot.transcript_message_identities =
            self.session.transcript_projection.message_identities();
        snapshot.follow_up = self.follow_up(&snapshot);
        snapshot.sub_agents = self.sub_agents(&snapshot);
        snapshot.collaborators = self.collaborators(&snapshot);
        Some(snapshot)
    }
    fn snapshot_result(&mut self) -> Result<Value, RuntimeError> {
        let snapshot = self.current_snapshot();
        Ok(json!({"snapshot": snapshot.map(|snapshot| self.delivery.snapshot(snapshot))}))
    }
    /// The projection a snapshot query names: only the loaded conversation, in
    /// its own runtime context. An enabled instance with nothing loaded has none.
    fn queried_snapshot(
        &mut self,
        request: &Value,
    ) -> Result<Option<crate::conversation::ConversationSnapshot>, RuntimeError> {
        let conversation_id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        if matches!(self.session.active_state, ActiveState::Empty) {
            if !self.runtime_instance(runtime_id)?.enabled {
                return Err(RuntimeError::invalid("此运行时已停用"));
            }
            // A conversation shown read-only has no session yet; its first
            // poll opens one so the view can follow native changes.
            match self.read_conversation_source(runtime_id, conversation_id) {
                Ok((snapshot, projection)) => self.present_history(snapshot, projection),
                Err(error) => {
                    tracing::warn!(target: "velune_application", event = "followed_conversation_read_failed", detail = %error);
                    return Ok(None);
                }
            }
        }
        self.sync_projection()?;
        self.follow_source();
        Ok(self.current_snapshot().filter(|snapshot| {
            snapshot.context_runtime_id == runtime_id && snapshot.conversation.id == conversation_id
        }))
    }
    fn drain_runtime(&mut self) -> Result<(), RuntimeError> {
        let was_busy = self.turn_running();
        let was_executing = self.session.executing();
        if was_executing {
            self.session.last_execution = Some(std::time::Instant::now());
        }
        let result = match &mut self.session.active_state {
            ActiveState::Acp(session) => session
                .poll()
                .map_err(|error| RuntimeError::context("ACP 事件读取失败", error)),
            ActiveState::Native(session) => session
                .poll()
                .map_err(|error| RuntimeError::context("运行时事件读取失败", error)),
            ActiveState::Pi => {
                self.drain_pi();
                Ok(())
            }
            ActiveState::Empty | ActiveState::History(_) => Ok(()),
        };
        let outcome = if was_busy && (!self.turn_running() || result.is_err()) {
            self.record_finished_turn(result.is_err())
        } else {
            None
        };
        // A turn's gateway lasts until the turn and the work it started have ended.
        if self.session.gateway_runner.is_some()
            && ((was_executing && !self.session.executing()) || result.is_err())
        {
            self.finish_managed_turn()?;
        }
        if let Some(outcome) = outcome {
            self.follow_finished_turn(outcome);
        }
        if !self.session.executing() {
            self.follow_ended_work();
        }
        result
    }

    fn finish_managed_turn(&mut self) -> Result<(), RuntimeError> {
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.execution_backends
                .finish_user_turn(&gateway_id, &session);
        }
        // Pi confirms durable message identities only during the settled RPC resync.
        let synchronized =
            if matches!(self.session.active_state, ActiveState::Pi) && !self.session.pi.busy {
                self.sync_projection()
            } else {
                Ok(())
            };
        if let ActiveState::Native(session) = &mut self.session.active_state
            && let Err(error) = session.release_override()
        {
            tracing::warn!(target: "velune_application", event = "gateway_release_restore_failed", detail = %error);
        }
        let snapshot = self.native_snapshot();
        let selection = self.session.model_selection.clone();
        let result = self.shutdown_active();
        self.session.model_selection = selection;
        if let Some(mut snapshot) = snapshot {
            self.session.model_record_key = snapshot.model_record_key.clone();
            snapshot.pending_interactions.clear();
            snapshot.actions.can_cancel = false;
            self.session.active_state = ActiveState::History(Box::new(snapshot));
        }
        synchronized.and(result)
    }
    fn public_gateways(&self) -> Vec<crate::GatewaySummary> {
        let mut summaries = crate::provider_configuration::summaries(&self.gateways);
        for gateway in &mut summaries {
            for provider in &mut gateway.providers {
                if provider.request_profile == crate::config::RequestProfile::KimiCoding {
                    (
                        provider.authentication.configured,
                        provider.authentication.account_label,
                    ) = self
                        .subscription_quota
                        .identity_status(&gateway.id, &provider.id);
                }

                if let Some(source) = self.gateways.iter().find(|g| g.id == gateway.id).and_then(|g| g.providers.iter().find(|p| p.id == provider.id))
                    && let crate::provider_authentication::AuthenticationMaterial::CodexSubscription { credential_id, .. } = &source.authentication.material
                { (provider.authentication.configured, provider.authentication.account_label) = crate::subscription_auth::status(&self.options.home_directory, credential_id, source.authentication.generation); }
            }
        }
        summaries
    }
    /// `stop_work` ends executing sessions too; otherwise any refuses.
    pub fn close_if_idle(&mut self, stop_work: bool) -> Result<(), RuntimeError> {
        self.drain_sessions()?;
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            || self
                .subscription_login
                .as_ref()
                .is_some_and(crate::provider_login::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if !stop_work
            && std::iter::once(&self.session)
                .chain(&self.parked)
                .any(session::Session::executing)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.subscription_quota.stop();
        self.execution_backends.shutdown();
        self.collaboration.shutdown();
        let result = self.for_each_session(Self::shutdown_active);
        self.parked.clear();
        result?;
        // Release the lock explicitly; a concurrent child spawn can briefly
        // inherit the open file description before exec closes its descriptors.
        self.repository.unlock()?;
        Ok(())
    }

    pub(crate) fn request_inner(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let result = self.route(request);
        self.trim_sessions();
        result
    }

    /// Address the session a conversation request names, then advance every
    /// session. Only the addressed session's failure fails the request; others
    /// settle into their failed state and are reported when addressed.
    fn route(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let action = request["action"].as_str().unwrap_or_default();
        let conversation_id = request["payload"]["conversationID"].as_str();
        match (action, conversation_id) {
            ("create" | "openAcpConversation", _) => self.focus_new(),
            (
                "open"
                | "sendTurn"
                | "cancel"
                | "stopSubAgents"
                | "replyRuntimeInteraction"
                | "renameConversation"
                | "deleteConversation"
                | "getSnapshot"
                | "getSnapshotUpdate"
                | "acpSessionConfig"
                | "setAcpSessionConfig"
                | "sendQueue"
                | "steerTurn"
                | "editMessage",
                Some(id),
            ) => self.focus_conversation(id),
            ("readConversation", Some(id)) => {
                self.focus_existing(id);
            }
            _ => {}
        }
        let focused = self.drain_runtime();
        for index in 0..self.parked.len() {
            std::mem::swap(&mut self.session, &mut self.parked[index]);
            if let Err(error) = self.drain_runtime() {
                tracing::warn!(target: "velune_application", event = "session_drain_failed", detail = %error);
            }
            std::mem::swap(&mut self.session, &mut self.parked[index]);
        }
        self.serve_collaboration();
        focused?;
        self.dispatch(request)
    }

    fn dispatch(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.sync_due_resources();
        if request["version"] != CONTRACT_VERSION {
            return Err(RuntimeError::Unsupported(
                "projection contract version".into(),
            ));
        }
        let action = request["action"].as_str().unwrap_or_default();
        if (self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            || self
                .subscription_login
                .as_ref()
                .is_some_and(crate::provider_login::Login::is_running))
            && !matches!(
                action,
                "authentication"
                    | "list"
                    | "searchConversations"
                    | "findConversation"
                    | "getSnapshot"
                    | "getSnapshotUpdate"
                    | "messagePayloads"
                    | "composerDrafts"
            )
        {
            return Err(RuntimeError::invalid("authentication is running"));
        }
        match action {
            "list" => self.list(),
            "outlinePresentation" => self.set_outline_presentation(request),
            "linkedConversationOpening" => self.set_linked_conversation_opening(request),
            "usageDisplay" => self.set_usage_display(request),
            "statusLine" => self.set_status_line(request),
            "composerDrafts" => self.composer_drafts_action(request),
            "conversationBrowserPreferences" => self.set_conversation_browser_preferences(request),
            "conversationBrowserSettings" => self.set_conversation_browser_limits(request),
            "conversationBrowserWindow" => self.browser_window_action(request),
            "conversationOrganization" => self.organization_action(request),
            "setProject" => self.set_project_action(request),
            "searchConversations" => self.search_conversations(request),
            "findConversation" => self.find_conversation(request),
            "discoverRuntimes" => self.discover_runtimes(request),
            "providerImport" => self.provider_import_action(request),
            "providers" => self.provider_action(&request["payload"]),
            "runtimeResourceCatalog"
            | "readRuntimeResource"
            | "writeRuntimeResource"
            | "deleteRuntimeResource" => self.runtime_management_action(request),
            "resourceTemplates" => self.resource_templates_action(request),
            "resourceSync" => self.resource_sync_action(request),
            "modelTemplates" => self.template_action(&request["payload"]),
            "runtimeInstances" => self.runtime_action(request),
            "selectRuntime" => self.select_runtime_action(request),
            "runtimeExecution" => self.runtime_execution_action(request, false),
            "configureRuntimeExecution" => self.runtime_execution_action(request, true),
            "runtimeHomeDirectory" => {
                let runtime = self.runtime_instance(
                    request["payload"]["runtimeInstanceID"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
                )?;
                Ok(serde_json::to_value(self.runtime_location(runtime)?.1)?)
            }
            "runtimeModels" => self.runtime_models_action(request),
            "reasoningEfforts" => {
                let runtime = self.runtime_instance(
                    request["payload"]["runtimeInstanceID"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
                )?;
                let key = request["payload"]["modelRecordKey"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("model record key"))?;
                Ok(serde_json::to_value(self.reasoning_efforts(runtime, key)?)?)
            }
            "acpLaunch" => Ok(serde_json::to_value(
                self.runtime_instance(
                    request["payload"]["runtimeInstanceID"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
                )?
                .acp_launch()?,
            )?),
            "acpRuntimeInfo" => self.acp_runtime_info_action(request),
            "openAcpConversation" => self.open_acp_conversation_action(request),
            "acpSessionConfig" => self.acp_session_config_action(request, false),
            "setAcpSessionConfig" => self.acp_session_config_action(request, true),
            "nextTurnModelSelection" => {
                Ok(serde_json::to_value(if self.native_snapshot().is_some() {
                    &self.session.model_selection
                } else {
                    &None
                })?)
            }
            "composerChoices" => self.composer_choices_action(request),
            "create" => self.create_conversation(request),
            "open" => self.open_conversation(request),
            "readConversation" => self.read_conversation(request),
            "renameConversation" => self.manage_conversation(request, false),
            "deleteConversation" => {
                let result = self.manage_conversation(request, true)?;
                if let Some(id) = request["payload"]["conversationID"].as_str() {
                    self.discard_composer_draft(id);
                    self.discard_conversation_choices(id);
                }
                Ok(result)
            }
            "workingChanges" => Ok(serde_json::to_value(self.working_changes(request)?)?),
            "workingBranch" => Ok(serde_json::to_value(self.working_branch(request)?)?),
            "fileDiff" => Ok(serde_json::to_value(self.file_diff(request)?)?),
            "conversationActivity" => {
                let since = request["payload"]["since"]
                    .as_u64()
                    .ok_or_else(|| RuntimeError::invalid("activity cursor"))?;
                Ok(serde_json::to_value(self.conversation_activity(since))?)
            }
            "getSnapshot" => Ok(json!({"snapshot": self
                .queried_snapshot(request)?
                .map(|snapshot| self.delivery.snapshot(snapshot))})),
            "getSnapshotUpdate" => {
                let since = serde_json::from_value(request["payload"]["since"].clone())?;
                let snapshot = self.queried_snapshot(request)?;
                Ok(serde_json::to_value(self.delivery.update(snapshot, since))?)
            }
            "messagePayloads" => {
                let ids: Vec<String> =
                    serde_json::from_value(request["payload"]["messageIDs"].clone())?;
                Ok(serde_json::to_value(
                    self.delivery.payloads(
                        request["payload"]["runtimeInstanceID"]
                            .as_str()
                            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
                        request["payload"]["conversationID"]
                            .as_str()
                            .ok_or_else(|| RuntimeError::invalid("conversation id"))?,
                        &ids,
                    ),
                )?)
            }
            "sendTurn" => self.send_turn_action(request),
            "sendQueue" => self.send_queue_action(request),
            "steerTurn" => self.steer_turn_action(request),
            "editMessage" => self.edit_message_action(request),
            "cancel" => self.cancel_action(request),
            "stopSubAgents" => self.stop_sub_agents_action(request),
            "replyRuntimeInteraction" => self.reply_runtime_interaction(request),
            "authentication" => self.authentication_action(&request["payload"]),
            action => Err(RuntimeError::Unsupported(action.into())),
        }
    }
}

use velune_agent_runtime::authentication;

impl Drop for CoreRuntime {
    fn drop(&mut self) {
        self.subscription_quota.stop();
        self.execution_backends.shutdown();
    }
}
