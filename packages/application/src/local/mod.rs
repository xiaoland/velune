//! Cross-platform application runtime.
//!
//! This module owns ordinary application configuration and the active
//! Pi/gateway projection. It has no filesystem socket, SQLite, environment, or
//! platform credential access beyond explicit paths supplied at open.

use crate::{Error as RuntimeError, Options as RuntimeOptions};
use crate::{
    config::{GatewayConfig, Protocol, ProviderModel, RuntimeInstance, RuntimeKind},
    conversation::{ConversationSummary, RunState},
    provider_import,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use velune_agent_runtime::history::{self, HistoryConfig};
use velune_agent_runtime::native::{GatewayInjection, NativeConfig, NativeKind, NativeSession};
use velune_agent_runtime::{self as pi, Config as PiConfig, LiveConversation, PiProjection};
use velune_gateway::Runner;

/// The runtime setting holding ACP launch arguments and environment.
pub(crate) const ACP_LAUNCH_SETTING: &str = "acpLaunch";
mod acp_composition;
mod composer_quota;
mod discovery;
mod naming;
mod working_directories;

fn runtime_types() -> Vec<crate::config::RuntimeTypeDescriptor> {
    let field = |key: &str,
                 label: &str,
                 kind: crate::conversation::SettingKind,
                 required: bool,
                 value: String,
                 help: &str| {
        crate::conversation::SettingField {
            key: key.into(),
            label: label.into(),
            kind,
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
        crate::conversation::SettingKind::FilePath,
        true,
        String::new(),
        "Node 22.19+ 的绝对路径；用于运行时接入与会话历史读取。",
    );
    node.executable_discovery = Some(crate::conversation::ExecutableDiscovery {
        command: "node".into(),
        minimum_version: "22.19.0".into(),
    });
    velune_agent_runtime::version::variants().iter().map(|variant| {
        let mut binary=field("binary","运行时入口",crate::conversation::SettingKind::FilePath,true,String::new(),"当前运行时版本的绝对可执行路径。执行准备时会核对实际版本。");
        if variant.kind==RuntimeKind::Acp {binary.help=Some("支持 ACP v1 的命令或 bridge 的绝对路径；启动参数与环境变量在下方设置，启动时协商协议。".into());}
        if variant.family_id=="pi" {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"pi".into(),minimum_version:"1.0.2".into()});}
        if matches!(variant.kind, RuntimeKind::Codex166) {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"codex".into(),minimum_version:"0.166.0".into()});}
        if variant.kind==RuntimeKind::DeepSeek {binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"dsh".into(),minimum_version:"0.2.0".into()});}
        let mut runtime_node = node.clone();
        runtime_node.required = variant.family_id != "claude-code";
        if variant.family_id == "claude-code" { binary.executable_discovery=Some(crate::conversation::ExecutableDiscovery{command:"claude".into(),minimum_version:"2.1.286".into()}); }
        let mut fields=vec![binary,runtime_node,field("agentDir","运行时目录",crate::conversation::SettingKind::DirectoryPath,true,String::new(),match variant.kind {RuntimeKind::ClaudeCode=>"该实例的 CLAUDE_CONFIG_DIR；配置与原生会话目录，不是任务工作目录。",RuntimeKind::Codex166 =>"该实例的 CODEX_HOME；配置与会话根目录，不是任务工作目录。",RuntimeKind::DeepSeek=>"该实例的 DeepSeek Harness 配置与会话根目录，不是任务工作目录。",_=>"该实例的 Pi 配置与状态根目录，不是任务工作目录。"})];
        if variant.kind==RuntimeKind::Acp {fields.retain(|field| field.key=="binary");}
        if variant.kind==RuntimeKind::Pi {fields.push(field("sessionDir","会话存储目录",crate::conversation::SettingKind::DirectoryPath,false,String::new(),"覆盖 Pi 默认的会话存储位置；留空采用运行时目录的 sessions，此项不是任务工作目录。"));}
        if !matches!(variant.kind, RuntimeKind::Codex166) {
            let mut naming = field("automaticNaming", "自动命名兜底", crate::conversation::SettingKind::Choice, false, "false".into(), "原生名称优先；由 Velune 使用所选模型为尚未命名的会话取名。");
            naming.options = vec![crate::conversation::SettingOption { id: "false".into(), label: "关闭".into() }, crate::conversation::SettingOption { id: "true".into(), label: "开启".into() }];
            fields.push(naming);
            fields.push(field("automaticNamingModel", "命名模型", crate::conversation::SettingKind::Text, false, String::new(), "Velune 中配置的模型记录键。"));
            fields.push(field("automaticNamingPrompt", "命名提示词", crate::conversation::SettingKind::Text, false, String::new(), "留空使用默认命名提示词。"));
        }
        crate::config::RuntimeTypeDescriptor {
            id: variant.kind,
            family_id: variant.family_id.into(),
            compatible_versions: variant.versions.describe(),
            can_rename_conversations: variant.can_rename_conversations,
            can_delete_conversations: variant.can_delete_conversations,
            can_side_chat: variant.side_chat.is_supported(),
            launch_options: variant.kind == RuntimeKind::Acp,
            agent_info: variant.kind == RuntimeKind::Acp,
            session_options: variant.kind == RuntimeKind::Acp,
            native_account_quota: crate::native_quota_source::Capture::supported(variant.kind),
            remote_resources: variant.kind != RuntimeKind::Acp,
            supported_protocols: crate::config::runtime_protocols(variant.kind),
            supported_provider_protocols: crate::config::runtime_provider_protocols(variant.kind),
            name: variant.name.into(),
            fields,
            permission_modes: variant.permission_modes.iter().map(|mode| {
                crate::config::PermissionMode {
                    id: mode.id.into(),
                    label: mode.label.into(),
                    detail: mode.detail.into(),
                }
            }).collect(),
        }
    }).collect()
}

/// The conversation session a use case addresses before it runs.
pub(crate) enum Focus {
    None,
    /// A new empty session; the current one stays open.
    New,
    /// The conversation's open session, or a new one the use case loads.
    Conversation(String),
    /// The conversation's open session if any; otherwise the current one.
    Existing(String),
}

/// Whether a use case may run while a provider login is in progress.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Login {
    Allows,
    Blocks,
}

#[derive(Default)]
enum ActiveState {
    #[default]
    Empty,
    History(Box<browsing::HistorySnapshot>),
    Pi(Box<pi::PiSession>),
    Native(Box<NativeSession>),
    Acp(Box<velune_agent_runtime::acp::Client>),
    Pending(Box<execution_workers::PendingExecution>),
}

impl ActiveState {
    /// The running conversation, whatever its runtime kind.
    fn live(&self) -> Option<&dyn LiveConversation> {
        match self {
            Self::Pi(session) => Some(session.as_ref()),
            Self::Native(session) => Some(session.as_ref()),
            Self::Acp(session) => Some(session.as_ref()),
            Self::Pending(pending) => Some(pending.live()),
            Self::Empty | Self::History(_) => None,
        }
    }
    fn live_mut(&mut self) -> Option<&mut dyn LiveConversation> {
        match self {
            Self::Pi(session) => Some(session.as_mut()),
            Self::Native(session) => Some(session.as_mut()),
            Self::Acp(session) => Some(session.as_mut()),
            Self::Pending(_) => None,
            Self::Empty | Self::History(_) => None,
        }
    }
}

struct UnsubmittedDraft {
    public_id: String,
    original_runtime_id: String,
    runtime_id: String,
    native_id: String,
    creation_token: Option<native_control::CreationToken>,
}

pub struct CoreRuntime {
    pub(crate) submission_cancellation: Option<(String, Arc<std::sync::atomic::AtomicBool>)>,
    telemetry: Option<Arc<velune_telemetry::ClientTelemetry>>,
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
    /// Current visible logical views supplied by all client views. Both the
    /// session and delivery caches use this same set for protected retention.
    observed_conversations: BTreeSet<(String, String)>,
    /// One bounded observation lifetime per physical source, independent of history views.
    control_observations: control::ControlObservations,
    execution_workers: execution_workers::ExecutionWorkers,
    execution_command: Option<execution_workers::ExecutionCommand>,
    execution_results: BTreeMap<
        execution_workers::ExecutionTicket,
        Result<execution_workers::ExecutionSuccess, RuntimeError>,
    >,
    /// Recoverable ownership records for temporary portable-summary forks.
    /// Malformed records disable only this feature, never ordinary startup.
    pub(crate) handoff_forks: handoff_summary::ForkRegistry,
    handoff_compaction_events: Vec<handoff_events::HandoffCompactionEvent>,
    /// Scoped provenance for the one prepared context currently being imported.
    prepared_handoff_work_id: Option<String>,
    handoffs: handoff::Handoffs,
    elsewhere: live::ElsewhereScan,
    finished_turns: session::FinishedTurns,
    scheduled_tasks: crate::scheduled_tasks::Store,
    schedule_next_check: std::time::Instant,
    gateways: Vec<GatewayConfig>,
    model_templates: Vec<crate::config::ModelTemplate>,
    conversation_links: Vec<crate::config::ConversationLink>,
    pending_history_reads: BTreeMap<String, usize>,
    conversation_browser_limits: crate::ConversationBrowserLimits,
    organization: crate::config::ConversationOrganization,
    async_question_marks_retry_at: Option<std::time::Instant>,
    browser_window: organization::BrowserWindow,
    /// The latest listing's conversations by public identity, for organizing
    /// what the list showed.
    last_listed: std::cell::RefCell<organization::ListedProjection>,
    /// The runtimes' own archives, listed while the archive view shows.
    archived_listing: std::cell::RefCell<BTreeMap<String, browsing::HistoryArchive>>,
    /// Failures from the last history scan, retained when only organization changes.
    history_failures: std::cell::RefCell<Vec<crate::HistoryFailure>>,
    /// Conversations no listing holds, reached by identity (a native
    /// sub-agent its runtime archived), as last described, for opening them.
    reached: std::cell::RefCell<BTreeMap<String, browsing::CachedSource>>,
    outline_presentation: crate::config::OutlinePresentation,
    linked_conversation_opening: crate::config::LinkedConversationOpening,
    usage_display: crate::config::UsageDisplay,
    context_usage_display: crate::config::UsageDisplay,
    status_line: Vec<crate::config::StatusLineItem>,
    composer_choices: crate::config::ComposerChoices,
    turn_requests: TurnRequests,
    resource_templates: Vec<crate::ResourceTemplate>,
    resource_bindings: Vec<crate::ResourceBinding>,
    /// Targets whose resource writes need retrying once their own work ends.
    resource_sync_due: std::collections::BTreeSet<String>,
    conversation_browser_preferences: crate::config::ConversationBrowserPreferences,
    subscription_login: Option<crate::provider_login::Login>,
    authentication_provider: Option<(String, String)>,
    runtime_instances: Vec<RuntimeInstance>,
    next_turn_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    acp_capability_cache: std::cell::RefCell<BTreeMap<String, crate::AcpCapabilities>>,
    native_projects: std::cell::RefCell<BTreeMap<String, projects::NativeProjects>>,
    history_listings: std::cell::RefCell<BTreeMap<String, browsing::HistoryListing>>,
    history_index_generation: std::cell::Cell<u64>,
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
    /// Saved provider versions whose dispatch resources are still leased by a turn.
    retired_gateway_providers: Vec<(String, crate::config::ProviderDefinition)>,
    pending_provider_identity_changes: std::collections::BTreeSet<(String, String)>,
    provider_account: Arc<crate::provider_account::ProviderAccountService>,
    subscription_quota: Arc<crate::subscription_quota::SubscriptionQuota>,
    billing: Arc<crate::billing::Billing>,
    runtime_quota: Arc<crate::runtime_quota::RuntimeQuota>,
    repository: crate::repository::Repository,
}

mod attachments;
mod client_state;
pub(crate) use client_state::ClientState;
mod agent_uri;
mod authentication_coordination;
mod browsing;
mod collaboration;
mod composer_choices;
mod composer_target;
pub(crate) mod conversation_history;
pub(crate) use composer_choices::{TurnKey, TurnRequests};
mod compaction;
mod configuration;
mod continuation;
mod conversation_address;
mod conversations;
pub(crate) use conversations::TurnRequest;
mod delivery;
mod editing;
mod execution;
pub(crate) mod execution_workers;
mod follow_up;
pub(crate) use follow_up::PreparationState;
mod handoff;
pub(crate) mod handoff_events;
pub(crate) mod handoff_summary;
mod submission;
use crate::SendQueueOperation;
mod background_tasks;
mod collaborators;
mod control;
pub(crate) mod list_refresh;
pub(crate) mod live;
mod model_selection;
mod native_composition;
mod native_control;
mod native_initialization;
mod organization;
mod pi_composition;
mod projects;
mod provider_management;
mod resource_library;
mod runtime_execution;
mod runtime_management;
mod runtime_source;
mod scheduled_tasks;
mod session;
mod side_chat;
mod sub_agents;
mod telemetry;
mod working_changes;
pub(crate) use organization::unix_ms_now;
use pi_composition::*;

impl CoreRuntime {
    pub fn open(
        options: RuntimeOptions,
        analytics: Arc<crate::analytics::AnalyticsStore>,
        home_lock: crate::repository::HomeLock,
    ) -> Result<Self, RuntimeError> {
        options.validate()?;
        let (repository, persisted) =
            crate::repository::Repository::open_with_lock(&options.home_directory, home_lock)?;
        let conversation_links = repository.load_conversation_links()?;
        let composer_choices = repository.load_composer_choices()?;
        let turn_requests = repository.load_turn_requests()?;
        let organization = repository.load_conversation_organization()?;
        let (resource_templates, resource_bindings) = repository.load_resource_library()?;
        let handoffs = handoff::Handoffs::open(&options.home_directory);
        let handoff_forks = handoff_summary::ForkRegistry::open(&options.home_directory);
        let scheduled_tasks = crate::scheduled_tasks::Store::open(&options.home_directory)?;
        let scheduling_state = velune_gateway::SchedulingState::default();
        let billing = Arc::new(crate::billing::Billing::default());
        billing.configure(&persisted.gateways);
        let subscription_quota = crate::subscription_quota::SubscriptionQuota::new(
            options.home_directory.clone(),
            &persisted.gateways,
            scheduling_state.clone(),
        );
        let provider_account = crate::provider_account::ProviderAccountService::new(
            options.home_directory.clone(),
            &persisted.gateways,
        );
        Ok(Self {
            submission_cancellation: None,
            telemetry: None,
            options,
            collaboration: Default::default(),
            session: Default::default(),
            parked: Vec::new(),
            observed_conversations: BTreeSet::new(),
            control_observations: control::ControlObservations::new()
                .map_err(|error| RuntimeError::context("控制权观察服务无法启动", error))?,
            handoff_forks,
            execution_workers: execution_workers::ExecutionWorkers::start()?,
            execution_command: None,
            execution_results: BTreeMap::new(),
            handoff_compaction_events: Vec::new(),
            prepared_handoff_work_id: None,
            elsewhere: live::ElsewhereScan::default(),
            finished_turns: session::FinishedTurns::default(),
            scheduled_tasks,
            schedule_next_check: std::time::Instant::now(),
            handoffs,
            repository,
            gateways: persisted.gateways,
            model_templates: persisted.model_templates,
            conversation_links,
            pending_history_reads: BTreeMap::new(),
            conversation_browser_limits: crate::ConversationBrowserLimits {
                page: persisted.conversation_browser_group_limit,
                total: persisted.conversation_browser_total_limit,
            },
            organization,
            async_question_marks_retry_at: None,
            browser_window: Default::default(),
            client_baseline: None,
            local_presentation_commit: std::cell::RefCell::new(None),
            last_listed: Default::default(),
            archived_listing: Default::default(),
            history_failures: Default::default(),
            reached: Default::default(),
            outline_presentation: persisted.outline_presentation,
            linked_conversation_opening: persisted.linked_conversation_opening,
            usage_display: persisted.usage_display,
            context_usage_display: persisted.context_usage_display,
            status_line: crate::config::canonical_status_line(persisted.status_line),
            composer_choices,
            turn_requests,
            resource_templates,
            resource_sync_due: resource_bindings
                .iter()
                .map(|binding| binding.runtime_id.clone())
                .collect(),
            resource_bindings,
            conversation_browser_preferences: persisted.conversation_browser_preferences,
            subscription_login: None,
            authentication_provider: None,
            runtime_instances: persisted.runtime_instances,
            next_turn_runtime_id: None,
            authentication: None,
            acp_capability_cache: Default::default(),
            native_projects: Default::default(),
            history_listings: Default::default(),
            history_index_generation: Default::default(),
            delivery: delivery::TranscriptDelivery::new(),
            prepared_remote_resources: Default::default(),
            remote_homes: Default::default(),
            sub_agent_failures: Default::default(),
            worktree_origins: Default::default(),
            turn_failures: Default::default(),
            analytics,
            scheduling_state,
            execution_backends: Default::default(),
            retired_gateway_providers: Vec::new(),
            pending_provider_identity_changes: Default::default(),
            provider_account,
            subscription_quota,
            billing,
            runtime_quota: Arc::default(),
        })
    }

    fn sync_scheduling(&mut self, previous: &[GatewayConfig]) {
        // Runners already capture their registry and resolver. Keep those services
        // alive for their turn; configure a new binding service/backend pool only
        // when the old one is leased. Admission/health stays shared by stable IDs.
        let leased = std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| {
                session.executing()
                    && session
                        .execution_configuration
                        .as_ref()
                        .and_then(|configuration| configuration.gateway.as_ref())
                        .is_some_and(|captured| {
                            previous.iter().any(|old| {
                                old.id == captured.id
                                    && self
                                        .gateways
                                        .iter()
                                        .find(|current| current.id == captured.id)
                                        != Some(old)
                            })
                        })
            });
        if leased {
            self.subscription_quota = self.subscription_quota.fork_configuration(&self.gateways);
            self.execution_backends = Default::default();
        }
        for gateway in previous {
            for provider in &gateway.providers {
                if self
                    .gateways
                    .iter()
                    .find(|current| current.id == gateway.id)
                    .and_then(|current| {
                        current
                            .providers
                            .iter()
                            .find(|current| current.id == provider.id)
                    })
                    != Some(provider)
                    && !self
                        .retired_gateway_providers
                        .iter()
                        .any(|(id, old)| id == &gateway.id && old == provider)
                {
                    self.retired_gateway_providers
                        .push((gateway.id.clone(), provider.clone()));
                }
            }
        }
        self.retire_gateway_configurations();
        self.subscription_quota.configure(&self.gateways);
        self.billing.configure(&self.gateways);
        self.provider_account.configure(&self.gateways);
    }

    fn retire_gateway_configurations(&mut self) {
        self.pending_provider_identity_changes
            .extend(self.subscription_quota.take_identity_changes());
        for (gateway, provider) in std::mem::take(&mut self.pending_provider_identity_changes) {
            if self.provider_in_use(&gateway, &provider) {
                self.pending_provider_identity_changes
                    .insert((gateway, provider));
            } else {
                self.scheduling_state
                    .invalidate_provider(&gateway, &provider);
            }
        }
        let retired = std::mem::take(&mut self.retired_gateway_providers);
        for (gateway_id, provider) in retired {
            // Retiring an epoch would also reject a still-running captured
            // runner. Wait until this stable provider is idle, across versions.
            if self.provider_in_use(&gateway_id, &provider.id) {
                self.retired_gateway_providers.push((gateway_id, provider));
                continue;
            }
            self.retire_provider_configuration(&gateway_id, &provider);
        }
    }

    fn retire_provider_configuration(
        &self,
        gateway_id: &str,
        provider: &crate::config::ProviderDefinition,
    ) {
        let current = self
            .gateways
            .iter()
            .find(|gateway| gateway.id == gateway_id)
            .and_then(|gateway| {
                gateway
                    .providers
                    .iter()
                    .find(|current| current.id == provider.id)
            });
        let same_account_source = current.is_some_and(|current| {
            matches!(
                current.request_profile,
                crate::config::RequestProfile::KimiCoding
                    | crate::config::RequestProfile::ArkCoding
            ) && provider.request_profile == current.request_profile
                && current.endpoint == provider.endpoint
                && current.protocol == provider.protocol
                && current.execution_backend == provider.execution_backend
                && current.quota_source == provider.quota_source
                && matches!(
                    current.authentication.material,
                    crate::provider_authentication::AuthenticationMaterial::ApiKey { .. }
                )
                && matches!(
                    provider.authentication.material,
                    crate::provider_authentication::AuthenticationMaterial::ApiKey { .. }
                )
        });
        if same_account_source {
            if current.is_some_and(|current| {
                current.authentication != provider.authentication
                    || current.quota_authentication != provider.quota_authentication
            }) {
                self.scheduling_state
                    .invalidate_credentials(gateway_id, &provider.id);
            }
        } else if current.is_none_or(|current| {
            current.authentication != provider.authentication
                || current.endpoint != provider.endpoint
                || current.protocol != provider.protocol
                || current.execution_backend != provider.execution_backend
                || current.request_profile != provider.request_profile
        }) {
            self.execution_backends
                .invalidate_provider(gateway_id, &provider.id);
            self.scheduling_state
                .invalidate_provider(gateway_id, &provider.id);
        }
        self.cleanup_retired_provider(provider);
    }
    fn turn_running(&self) -> bool {
        self.session.turn_running()
    }
    /// Owned read for cache/rollback, distinct from disposable wire projection.
    /// Never turn a bare display clone back into a history cache owner.
    fn owned_native_snapshot(&self) -> Result<Option<browsing::HistorySnapshot>, RuntimeError> {
        if self.session.projected().is_none() {
            return Ok(None);
        }
        let mut snapshot = match &self.session.active_state {
            ActiveState::History(snapshot) => snapshot.try_clone()?,
            ActiveState::Native(native) => browsing::HistorySnapshot::from_retained(
                native
                    .owned_snapshot()
                    .map_err(|error| RuntimeError::context("native retained snapshot", error))?,
            ),
            ActiveState::Acp(acp) => browsing::HistorySnapshot::from_retained(
                acp.owned_snapshot()
                    .map_err(|error| RuntimeError::context("ACP retained snapshot", error))?,
            ),
            // Pi's existing receiver is not the newline RpcClient ledger.
            ActiveState::Pi(_) => self
                .session
                .projected()
                .expect("present Pi snapshot")
                .clone()
                .into(),
            ActiveState::Empty | ActiveState::Pending(_) => return Ok(None),
        };
        snapshot.model_record_key = self.session.model_record_key.clone();
        Ok(Some(snapshot))
    }
    fn native_snapshot(&self) -> Option<crate::conversation::ConversationSnapshot> {
        self.session.projected().cloned().map(|mut snapshot| {
            snapshot.model_record_key = self.session.model_record_key.clone();
            snapshot
        })
    }
    /// Advance message identities and turn settings before a command or a delivery.
    fn refresh_transcript(&mut self) -> Option<crate::conversation::ConversationSnapshot> {
        use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
        self.retry_async_question_marks();
        let external_turn = self.observe_current_external_turn();
        let Some(native) = self.native_snapshot() else {
            self.session.transcript_projection = TranscriptProjection::default();
            self.session.transcript_conversation_id = None;
            return None;
        };
        let physical_id = native.conversation.id.clone();
        let physical_anchor = native.messages.last().map(|message| message.id.clone());
        let mut snapshot = match &self.session.side {
            Some(side) => side_chat::shown(side, native),
            None => self.decorate_snapshot(native),
        };
        if self.session.side.is_none() {
            self.apply_handoff_compaction_events(
                &mut snapshot,
                &physical_id,
                physical_anchor.as_deref(),
            );
        }
        snapshot.pending_interactions.retain(|interaction| {
            self.interaction_is_visible(&snapshot.conversation.id, interaction)
        });
        // Optional async questions must never conceal an approval that blocks execution.
        snapshot
            .pending_interactions
            .sort_by_key(|interaction| !interaction.blocks_turn());
        self.apply_generated_title(&mut snapshot.conversation);
        let child_run_state = self.child_run_state(&snapshot);
        if let Some(state) = &child_run_state {
            snapshot.run_state = state.clone();
        } else if external_turn
            && matches!(snapshot.run_state, RunState::Idle | RunState::Elsewhere | RunState::Failed)
        {
            snapshot.run_state = RunState::Elsewhere;
        }
        continuation::stamp_turn_runtime(&mut snapshot.messages, &snapshot.context_runtime_id);
        if self.session.transcript_conversation_id.as_deref() != Some(&snapshot.conversation.id) {
            self.session.transcript_projection = TranscriptProjection::default();
            self.session.transcript_conversation_id = Some(snapshot.conversation.id.clone());
        }
        self.refresh_native_accepted_settings();
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ConfirmIdentities(
                &snapshot.message_identity_confirmations,
            ));
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ReplaceSnapshot(&snapshot));
        self.session
            .transcript_projection
            .retain_reply_boundaries(&snapshot.retained_reply_ids);
        composer_choices::recall_turn_requests(
            &self.turn_requests,
            &mut self.session.transcript_projection,
            &snapshot,
        );
        self.session
            .transcript_projection
            .apply(TranscriptEvent::ExecutionState(
                if self.session.compacting {
                    RunState::Idle
                } else if child_run_state.is_some() {
                    // The owning runtime measures the child turn, not this
                    // history viewer, including while a stop is in progress.
                    RunState::Elsewhere
                } else {
                    snapshot.run_state.clone()
                },
            ));
        self.keep_turn_requests(&snapshot);
        Some(snapshot)
    }

    /// Both SendTurn and snapshot delivery derive admission from this logical context.
    fn submission_context(&mut self) -> Option<crate::conversation::ConversationSnapshot> {
        let mut snapshot = self.refresh_transcript()?;
        self.apply_input_access(&mut snapshot);
        if self.session.side.is_none() {
            snapshot.follow_up.control_handoff = self.conversation_handoff(&snapshot);
        }
        Some(snapshot)
    }

    fn current_snapshot(&mut self) -> Option<crate::conversation::ConversationSnapshot> {
        let mut snapshot = self.submission_context()?;
        snapshot.transcript_turns = self.session.transcript_projection.turns();
        self.name_turn_models(&mut snapshot.transcript_turns);
        self.stamp_forkable_turns(&mut snapshot);
        snapshot.transcript_items = self.session.transcript_projection.items();
        snapshot.transcript_outline = self.session.transcript_projection.outline();
        snapshot.transcript_message_identities =
            self.session.transcript_projection.message_identities();
        if self.session.side.is_none() {
            snapshot.follow_up = self.follow_up(&snapshot);
            snapshot.sub_agents = self.sub_agents(&snapshot);
            snapshot.collaborators = self.collaborators(&snapshot);
        }
        self.apply_submission_admission(&mut snapshot);
        self.apply_child_control(&mut snapshot);
        Some(snapshot)
    }
    fn snapshot_result(&mut self) -> Result<crate::SnapshotResult, RuntimeError> {
        let snapshot = self.current_snapshot();
        Ok(crate::SnapshotResult {
            submission_receipt: None,
            snapshot: snapshot.map(|snapshot| {
                self.delivery
                    .snapshot(snapshot, &self.observed_conversations)
            }),
            submitted_message_id: None,
        })
    }
    /// Queue ownership has already changed. Presentation failure cannot revoke
    /// acceptance or invite the client to submit the same input a second time.
    fn accepted_submission_result(
        &mut self,
        receipt: crate::SubmissionReceipt,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let mut result = self.snapshot_result().unwrap_or_else(|error| {
            tracing::warn!(target: "velune_application", event = "accepted_submission_projection_failed", detail = %error);
            crate::SnapshotResult {
                snapshot: None,
                submitted_message_id: None,
                submission_receipt: None,
            }
        });
        result.submission_receipt = Some(receipt);
        Ok(result)
    }
    /// The loaded conversation in its runtime context. An enabled known
    /// instance returns no snapshot when no conversation is loaded; a
    /// different instance cannot query another instance's conversation.
    pub(crate) fn snapshot(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let snapshot = self.queried_snapshot(runtime_id, conversation_id)?;
        Ok(crate::SnapshotResult {
            submission_receipt: None,
            snapshot: snapshot.map(|snapshot| {
                self.delivery
                    .snapshot(snapshot, &self.observed_conversations)
            }),
            submitted_message_id: None,
        })
    }
    /// Like `snapshot`, relative to a cursor from an earlier delivery of the
    /// same view: unchanged content is not transferred again.
    pub(crate) fn snapshot_update(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        since: crate::TranscriptCursor,
    ) -> Result<crate::SnapshotUpdate, RuntimeError> {
        let snapshot = self.queried_snapshot(runtime_id, conversation_id)?;
        Ok(self
            .delivery
            .update(snapshot, since, &self.observed_conversations))
    }
    /// Complete payloads of messages in a delivered view.
    pub(crate) fn message_payloads(
        &self,
        runtime_id: &str,
        conversation_id: &str,
        message_ids: &[String],
    ) -> Vec<crate::DeliveredMessage> {
        self.delivery
            .payloads(runtime_id, conversation_id, message_ids)
    }
    /// The projection a snapshot query names: only the loaded conversation, in
    /// its own runtime context. An enabled instance with nothing loaded has none.
    fn queried_snapshot(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
    ) -> Result<Option<crate::conversation::ConversationSnapshot>, RuntimeError> {
        let address = self.conversation_read_address(runtime_id, conversation_id)?;
        if matches!(self.session.active_state, ActiveState::Empty) {
            if !self.runtime_instance(runtime_id)?.enabled {
                return Err(RuntimeError::invalid("此运行时已停用"));
            }
            // Cold IO is prepared by ApplicationService and loaded by its
            // bounded HistoryWorkers. A query never falls back to a sync read.
            return Ok(None);
        }
        self.sync_projection()?;
        self.follow_source();
        Ok(self.current_snapshot().filter(|snapshot| {
            snapshot.context_runtime_id == address.context_runtime_id
                && snapshot.conversation.id == conversation_id
        }))
    }
    fn drain_runtime(&mut self) -> Result<(), RuntimeError> {
        if self
            .session
            .pi_io_cleanup
            .as_ref()
            .is_some_and(|pending| pending.abandoned_cleanup_complete())
        {
            self.session.pi_io_cleanup = None;
        }
        if (self.session.native_controls.cleanup_required
            && matches!(self.session.active_state, ActiveState::Native(_)))
            || (self.session.pi_cleanup_required
                && matches!(self.session.active_state, ActiveState::Pi(_)))
        {
            // Saturation retains the exact failed resource for a later drain.
            if self.session.pi_cleanup_required {
                if let ActiveState::Pi(pi) = &mut self.session.active_state {
                    pi.poll();
                }
                self.advance_pi_control();
                self.advance_pi_edit();
                if matches!(self.session.active_state, ActiveState::Pi(_)) {
                    let _ = self.start_native_cleanup_worker();
                }
            } else {
                let _ = self.schedule_native_cleanup();
            }
            return Ok(());
        }
        let compacting = self.session.compacting;
        let was_executing = self.session.executing();
        let changed_initialization = match &self.session.active_state {
            ActiveState::Pending(pending) => pending
                .initialization_source()
                .is_some_and(|source| !source.is_current(self)),
            _ => false,
        };
        if changed_initialization {
            let ActiveState::Pending(pending) = &self.session.active_state else {
                unreachable!()
            };
            let (ticket, error) = pending.discard_changed_initialization();
            // Abandonment leaves adapter destruction and capacity on its worker.
            self.session.active_state = ActiveState::Empty;
            self.session.execution_runtime_id = None;
            self.session.execution_configuration = None;
            self.session.model_selection = None;
            self.session.configuration_changed = false;
            self.execution_results.insert(ticket, Err(error));
            return Ok(());
        }
        if let ActiveState::Pending(pending) = &mut self.session.active_state {
            let Some((ticket, state, outcome)) = pending.complete() else {
                return Ok(());
            };
            self.session.active_state = state;
            let result = self.apply_execution_outcome(outcome);
            self.execution_results
                .insert(ticket, result.map(|()| Default::default()));
        }
        if was_executing {
            self.session.last_execution = Some(std::time::Instant::now());
        }
        let result = match self.session.active_state.live_mut() {
            Some(live) => live
                .poll()
                .map_err(|error| RuntimeError::context("运行时事件读取失败", error)),
            None => Ok(()),
        };
        self.harvest_native_controls();
        self.advance_native_submissions();
        self.advance_pi_control();
        self.advance_pi_edit();
        self.refresh_native_accepted_settings();
        let pi_failure = matches!(&self.session.active_state,
            ActiveState::Pi(pi) if pi.requires_failure_cleanup());
        if pi_failure {
            self.session.pi_cleanup_required = true;
            self.session.send_queue.pause_after_stop();
            if let Some(outcome) = self.record_finished_turn() {
                self.follow_finished_turn(outcome);
            }
            let _ = self.start_native_cleanup_worker();
            return result;
        }
        // A compatibility mutation or failed first admission may switch the
        // native lifecycle before this drain. Cleanup remains owed even when
        // no application ticket was returned and subsequent polls are Ok.
        let native_failure = matches!(&self.session.active_state,
            ActiveState::Native(native) if native.requires_failure_cleanup());
        if native_failure || (self.session.native_controls.enabled && result.is_err()) {
            self.session.native_controls.cleanup_required = true;
            self.session.send_queue.pause_after_stop();
            if let Some(outcome) = self.record_finished_turn() {
                self.follow_finished_turn(outcome);
            }
            let _ = self.schedule_native_cleanup();
            return result;
        }
        // A successful maintenance reply can race the user's stop request.
        // record_finished_turn clears cancel_requested; retain the stop in
        // the queue first, even if background metadata has not settled yet.
        if compacting && self.session.cancel_requested {
            self.session.send_queue.pause_after_stop();
        }
        let managed_pi = self.session.gateway_runner.is_some()
            && self.session.side.is_none()
            && matches!(self.session.active_state, ActiveState::Pi(_));
        // Pi startup also consumes a work lease while discovering background
        // tasks. Only a terminal activity in this connection can finish a turn.
        if managed_pi
            && self.session.projected().is_some_and(|view| {
                view.executions
                    .current
                    .as_ref()
                    .is_some_and(|activity| activity.ended)
            })
        {
            if !self.session.executing() {
                self.finish_managed_backend_turn();
                self.session.pi_cleanup_required = true;
                let _ = self.start_native_cleanup_worker();
                // Consume the outcome when the worker returns the projection.
                return result;
            }
            // Only actual background work allows the next turn to share this
            // connection before the current managed execution is collected.
            if self.session.work_status() != velune_agent_runtime::work::WorkStatus::Active {
                return result;
            }
        }
        let outcome = if !self.turn_running() || result.is_err() {
            self.record_finished_turn()
        } else {
            None
        };
        // A turn's gateway lasts until the turn and the work it started have ended.
        if !managed_pi
            && self.session.gateway_runner.is_some()
            && ((was_executing && !self.session.executing()) || result.is_err())
        {
            self.finish_managed_turn()?;
        }
        // A queued next turn also prepares from the saved configuration.
        if !self.session.executing() && self.session.configuration_changed {
            if self.preserves_connection_configuration() {
                self.session.configuration_changed = false;
            } else {
                self.invalidate_execution()?;
            }
        }
        self.poll_automatic_naming(outcome == Some(crate::TurnOutcome::Completed));
        if let Some(outcome) = outcome {
            self.follow_finished_turn(outcome);
        }
        if !self.session.executing() {
            if compacting {
                self.session.compacting = false;
                self.follow_finished_compaction(result.is_err());
            } else {
                self.follow_ended_work();
            }
        }
        result
    }

    fn finish_managed_backend_turn(&self) {
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.session_backends()
                .finish_user_turn(&gateway_id, &session);
        }
    }

    fn finish_managed_turn(&mut self) -> Result<(), RuntimeError> {
        self.finish_managed_backend_turn();
        // A side chat's copy lives only in its runtime process; it ends when
        // the side chat closes.
        if self.session.side.is_some() {
            return Ok(());
        }
        // Pi confirms durable message identities only during the settled RPC resync.
        let synchronized = if matches!(&self.session.active_state, ActiveState::Pi(session) if !session.turn_running())
        {
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
            snapshot
                .pending_interactions
                .retain(|interaction| !interaction.requires_live_reply());
            snapshot.actions.can_cancel = false;
            self.session.set_history(snapshot);
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
        if self.has_pending_execution() {
            return Err(RuntimeError::Busy);
        }
        if stop_work
            && std::iter::once(&self.session)
                .chain(&self.parked)
                .any(|session| matches!(session.active_state, ActiveState::Pi(_)))
        {
            self.for_each_session(|core| {
                if matches!(core.session.active_state, ActiveState::Pi(_)) {
                    core.session.pi_cleanup_required = true;
                    // Saturation retains this exact adapter for owner drain to
                    // retry. Explicit stopping close authorizes teardown, not
                    // an owner-thread kill/join or early release of HOME.
                    let _ = core.start_native_cleanup_worker();
                }
                Ok(())
            })?;
            return Err(RuntimeError::Busy);
        }
        self.subscription_quota.stop();
        self.billing.stop();
        self.provider_account.stop();
        self.runtime_quota.stop();
        self.execution_backends.shutdown();
        self.collaboration.shutdown();
        let result = self.for_each_session(Self::shutdown_active);
        self.parked.clear();
        result?;
        self.execution_workers.shutdown();
        // Stopping work releases the last captured credential/backend leases too.
        self.retire_gateway_configurations();
        // Release the lock explicitly; a concurrent child spawn can briefly
        // inherit the open file description before exec closes its descriptors.
        self.repository.unlock()?;
        Ok(())
    }

    /// Run one use case: address the session it names, advance every session,
    /// then run it. Only the addressed session's failure fails the use case;
    /// others settle into their failed state and are reported when addressed.
    pub(crate) fn serve<T>(
        &mut self,
        focus: Focus,
        login: Login,
        run: impl FnOnce(&mut Self) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let result = self.prepare(focus, login).and_then(|()| run(self));
        self.trim_sessions();
        result
    }

    fn prepare(&mut self, focus: Focus, login: Login) -> Result<(), RuntimeError> {
        match focus {
            Focus::None => {}
            Focus::New => self.focus_new(),
            Focus::Conversation(id) => self.focus_conversation(&id),
            Focus::Existing(id) => {
                self.focus_existing(&id);
            }
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
        self.sync_due_resources();
        self.retire_gateway_configurations();
        if login == Login::Blocks && self.login_running() {
            return Err(RuntimeError::invalid("authentication is running"));
        }
        Ok(())
    }

    fn login_running(&self) -> bool {
        self.authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            || self
                .subscription_login
                .as_ref()
                .is_some_and(crate::provider_login::Login::is_running)
    }
}

use velune_agent_runtime::authentication;

impl Drop for CoreRuntime {
    fn drop(&mut self) {
        self.shutdown_execution_workers();
        if let Some(owner) = &self.telemetry {
            owner.execution_owner_unavailable();
        }
        self.subscription_quota.stop();
        self.billing.stop();
        self.provider_account.stop();
        self.runtime_quota.stop();
        self.execution_backends.shutdown();
    }
}
