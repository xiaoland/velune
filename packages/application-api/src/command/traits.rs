use super::{Command, DeliveryClass, Response};
use crate::{
    ComposerInputOperation, ConversationReference, ModelSelection, ScheduledTasksOperation,
    SnapshotResult,
};

/// Transport and owner policies for one command, borrowing only the needed parameters.
#[derive(Clone, Copy)]
pub struct CommandTraits<'a> {
    pub delivery: DeliveryClass,
    pub route: CommandRoute<'a>,
    /// The address recorded with an uncertain remote operation.
    pub operation_target: Option<CommandTarget<'a>>,
    pub touch_view: bool,
    pub trace_owner_queue: bool,
    shared_state: SharedState,
    history_invalidation: HistoryInvalidation<'a>,
}

#[derive(Clone, Copy)]
pub enum CommandRoute<'a> {
    Owner {
        submission_conversation_id: Option<&'a str>,
    },
    Live(LiveRoute<'a>),
    /// Concurrent readers share one in-flight configuration and history scan.
    ListRefresh,
    ReadHistory(ConversationAddress<'a>),
    HistoryWithSourceRefresh(HistoryLookup<'a>),
    Handoff(HandoffRoute<'a>),
}

#[derive(Clone, Copy)]
pub struct ConversationAddress<'a> {
    pub runtime_id: &'a str,
    pub conversation_id: &'a str,
}

#[derive(Clone, Copy)]
pub struct CommandTarget<'a> {
    pub runtime_id: &'a str,
    pub conversation_id: Option<&'a str>,
}

impl<'a> CommandTarget<'a> {
    fn conversation(runtime_id: &'a str, conversation_id: &'a str) -> Self {
        Self {
            runtime_id,
            conversation_id: Some(conversation_id),
        }
    }
}

#[derive(Clone, Copy)]
pub struct LiveRoute<'a> {
    pub kind: ExecutionCommandKind,
    pub address: CommandTarget<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionCommandKind {
    Create,
    Cancel,
    StopBackground,
    Reply,
    Send,
    Resume,
    Steer,
    Edit,
    Delete,
}

impl ExecutionCommandKind {
    pub fn is_submission(self) -> bool {
        match self {
            Self::Send | Self::Resume | Self::Steer | Self::Edit => true,
            Self::Create | Self::Cancel | Self::StopBackground | Self::Reply | Self::Delete => {
                false
            }
        }
    }

    pub fn response(self) -> fn(SnapshotResult) -> Response {
        match self {
            Self::Create => Response::CreateConversation,
            Self::Cancel => Response::Cancel,
            Self::StopBackground => Response::StopSubAgents,
            Self::Reply => Response::ReplyRuntimeInteraction,
            Self::Send => Response::SendTurn,
            Self::Resume => Response::ResumeTurn,
            Self::Steer => Response::SteerTurn,
            Self::Edit => Response::EditMessage,
            Self::Delete => Response::DeleteUserMessage,
        }
    }
}

#[derive(Clone, Copy)]
pub enum HistoryLookup<'a> {
    Refresh(ConversationAddress<'a>),
    /// Even without a history address, the owner checks whether to refresh the source projection.
    Retry(Option<ConversationAddress<'a>>),
}

#[derive(Clone, Copy)]
pub enum HandoffRoute<'a> {
    QueueSummary,
    BeginSummary {
        source: ConversationAddress<'a>,
        target_runtime_id: &'a str,
        summary_model: Option<&'a ModelSelection>,
        effort: Option<&'a str>,
    },
    ReadSummary(&'a str),
    CancelSummary(&'a str),
    SendPrepared,
    ReadRequirements {
        source: ConversationAddress<'a>,
        target_runtime_id: &'a str,
        request_id: Option<&'a str>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedState {
    Unchanged,
    AdvancesRevision,
    RequiresRevision,
}

#[derive(Clone, Copy)]
enum HistoryInvalidation<'a> {
    None,
    Conversation(&'a str),
    Conversations(&'a [ConversationReference]),
}

impl<'a> CommandTraits<'a> {
    const OBSERVATION: Self = Self {
        delivery: DeliveryClass::Observation,
        route: CommandRoute::Owner {
            submission_conversation_id: None,
        },
        operation_target: None,
        touch_view: true,
        trace_owner_queue: false,
        shared_state: SharedState::Unchanged,
        history_invalidation: HistoryInvalidation::None,
    };
    const EFFECT: Self = Self {
        delivery: DeliveryClass::Effect,
        ..Self::OBSERVATION
    };
    const MUTATION: Self = Self {
        shared_state: SharedState::AdvancesRevision,
        ..Self::EFFECT
    };
    const REVISION_CHECKED: Self = Self {
        shared_state: SharedState::RequiresRevision,
        ..Self::EFFECT
    };

    /// Whether a remote caller must carry the current revision.
    pub fn requires_revision(self) -> bool {
        self.shared_state == SharedState::RequiresRevision
    }

    pub fn changes_shared_state(self) -> bool {
        self.shared_state != SharedState::Unchanged
    }

    pub fn affects_history(self, ids: &[String]) -> bool {
        match self.history_invalidation {
            HistoryInvalidation::None => false,
            HistoryInvalidation::Conversation(id) => ids.iter().any(|candidate| candidate == id),
            HistoryInvalidation::Conversations(references) => references
                .iter()
                .any(|reference| ids.contains(&reference.conversation_id)),
        }
    }

    fn live(self, kind: ExecutionCommandKind, address: CommandTarget<'a>) -> Self {
        Self {
            route: CommandRoute::Live(LiveRoute { kind, address }),
            ..self
        }
    }
}

impl Command {
    /// Assign every command's delivery, state, and routing policies in one exhaustive match.
    pub fn traits(&self) -> CommandTraits<'_> {
        use CommandRoute::{Handoff, HistoryWithSourceRefresh, Owner, ReadHistory};
        use CommandTraits as Traits;
        use ExecutionCommandKind as Live;
        use HistoryInvalidation::{Conversation, Conversations};
        use HistoryLookup::{Refresh, Retry};

        match self {
            Self::ManageComposerInputs { operation } => match operation {
                ComposerInputOperation::Read { .. } => Traits::OBSERVATION,
                ComposerInputOperation::Ensure { .. }
                | ComposerInputOperation::Save { .. }
                | ComposerInputOperation::EnsureTarget { .. }
                | ComposerInputOperation::SaveTarget { .. }
                | ComposerInputOperation::Bind { .. }
                | ComposerInputOperation::Promote { .. }
                | ComposerInputOperation::Forget { .. } => Traits::EFFECT,
            },
            Self::ManageScheduledTasks { operation } => match operation {
                ScheduledTasksOperation::List
                | ScheduledTasksOperation::PreviewImport { .. }
                | ScheduledTasksOperation::PreviewSchedule { .. } => Traits::OBSERVATION,
                ScheduledTasksOperation::Save { .. }
                | ScheduledTasksOperation::SetEnabled { .. }
                | ScheduledTasksOperation::Delete { .. }
                | ScheduledTasksOperation::RunNow { .. }
                | ScheduledTasksOperation::Import { .. }
                | ScheduledTasksOperation::ConfirmHandover { .. } => Traits::MUTATION,
            },
            Self::ReleaseView => Traits {
                touch_view: false,
                ..Traits::EFFECT
            },
            Self::CreateConversation { runtime_id, .. } => {
                let address = CommandTarget {
                    runtime_id,
                    conversation_id: None,
                };
                Traits {
                    operation_target: Some(address),
                    ..Traits::MUTATION.live(Live::Create, address)
                }
            }
            Self::SendTurn {
                source_runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(source_runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    trace_owner_queue: true,
                    ..Traits::MUTATION.live(Live::Send, address)
                }
            }
            Self::SendCurrentTurn {
                source_runtime_id,
                conversation_id,
                ..
            } => Traits {
                history_invalidation: Conversation(conversation_id),
                trace_owner_queue: true,
                ..Traits::MUTATION.live(
                    Live::Send,
                    CommandTarget::conversation(source_runtime_id, conversation_id),
                )
            },
            Self::ReadConversation {
                runtime_id,
                conversation_id,
            } => Traits {
                route: ReadHistory(ConversationAddress {
                    runtime_id,
                    conversation_id,
                }),
                trace_owner_queue: true,
                ..Traits::OBSERVATION
            },
            Self::ReadSnapshot {
                runtime_id,
                conversation_id,
            }
            | Self::ReadSnapshotUpdate {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                route: HistoryWithSourceRefresh(Retry(Some(ConversationAddress {
                    runtime_id,
                    conversation_id,
                }))),
                ..Traits::OBSERVATION
            },
            Self::OpenConversation {
                runtime_id,
                conversation_id,
            } => Traits {
                route: HistoryWithSourceRefresh(Refresh(ConversationAddress {
                    runtime_id,
                    conversation_id,
                })),
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                trace_owner_queue: true,
                ..Traits::EFFECT
            },
            Self::ReadComposerTargetIdentity { request }
            | Self::ReadComposerTargetBasis { request, .. } => {
                let address = request
                    .conversation_id
                    .as_deref()
                    .filter(|id| !id.starts_with("draft:") && !id.starts_with("side-chat:"))
                    .map(|conversation_id| ConversationAddress {
                        runtime_id: &request.runtime_id,
                        conversation_id,
                    });
                Traits {
                    route: HistoryWithSourceRefresh(Retry(address)),
                    ..Traits::OBSERVATION
                }
            }
            Self::SetConversationHandoff {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                route: HistoryWithSourceRefresh(Retry(Some(ConversationAddress {
                    runtime_id,
                    conversation_id,
                }))),
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                history_invalidation: Conversation(conversation_id),
                trace_owner_queue: true,
                ..Traits::MUTATION
            },
            Self::OpenSideChat {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                route: HistoryWithSourceRefresh(Retry(Some(ConversationAddress {
                    runtime_id,
                    conversation_id,
                }))),
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                ..Traits::MUTATION
            },
            Self::SendSideChat { side_chat_id, .. } => Traits {
                route: Owner {
                    submission_conversation_id: Some(side_chat_id),
                },
                ..Traits::MUTATION
            },
            Self::QueueHandoffSummary { .. } | Self::RetryQueuedHandoffSummary { .. } => Traits {
                route: Handoff(HandoffRoute::QueueSummary),
                ..Traits::MUTATION
            },
            Self::BeginHandoffSummary {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                summary_model,
                effort,
            } => Traits {
                route: Handoff(HandoffRoute::BeginSummary {
                    source: ConversationAddress {
                        runtime_id: source_runtime_id,
                        conversation_id,
                    },
                    target_runtime_id,
                    summary_model: summary_model.as_ref(),
                    effort: effort.as_deref(),
                }),
                ..Traits::MUTATION
            },
            Self::ReadHandoffSummary { work_id } => Traits {
                route: Handoff(HandoffRoute::ReadSummary(work_id)),
                ..Traits::OBSERVATION
            },
            Self::CancelHandoffSummary { work_id } => Traits {
                route: Handoff(HandoffRoute::CancelSummary(work_id)),
                ..Traits::MUTATION
            },
            Self::SendPreparedHandoff { .. } => Traits {
                route: Handoff(HandoffRoute::SendPrepared),
                ..Traits::MUTATION
            },
            Self::ReadHandoffRequirements {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                request_id,
            } => Traits {
                route: Handoff(HandoffRoute::ReadRequirements {
                    source: ConversationAddress {
                        runtime_id: source_runtime_id,
                        conversation_id,
                    },
                    target_runtime_id,
                    request_id: request_id.as_deref(),
                }),
                ..Traits::OBSERVATION
            },
            Self::RenameConversation {
                runtime_id,
                conversation_id,
                ..
            }
            | Self::DeleteConversation {
                runtime_id,
                conversation_id,
            } => Traits {
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                history_invalidation: Conversation(conversation_id),
                ..Traits::REVISION_CHECKED
            },
            Self::SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                ..Traits::REVISION_CHECKED
            },
            Self::EditSendQueue {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                history_invalidation: Conversation(conversation_id),
                ..Traits::MUTATION
            },
            Self::ForkConversation {
                runtime_id,
                conversation_id,
                ..
            } => Traits {
                operation_target: Some(CommandTarget::conversation(runtime_id, conversation_id)),
                ..Traits::MUTATION
            },
            Self::CompactConversation {
                conversation_id, ..
            } => Traits {
                history_invalidation: Conversation(conversation_id),
                ..Traits::MUTATION
            },
            Self::AdoptRuntimeResource { runtime_id, .. }
            | Self::ResolveResourceBinding { runtime_id, .. } => Traits {
                operation_target: Some(CommandTarget {
                    runtime_id,
                    conversation_id: None,
                }),
                ..Traits::REVISION_CHECKED
            },
            Self::OrganizeConversations { conversations, .. } => Traits {
                history_invalidation: Conversations(conversations),
                ..Traits::REVISION_CHECKED
            },
            Self::Cancel {
                runtime_id,
                conversation_id,
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    trace_owner_queue: true,
                    ..Traits::MUTATION.live(Live::Cancel, address)
                }
            }
            Self::StopSubAgents {
                runtime_id,
                conversation_id,
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    ..Traits::MUTATION.live(Live::StopBackground, address)
                }
            }
            Self::ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    trace_owner_queue: true,
                    ..Traits::MUTATION.live(Live::Reply, address)
                }
            }
            Self::ResumeTurn {
                runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    ..Traits::MUTATION.live(Live::Resume, address)
                }
            }
            Self::SteerTurn {
                runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    ..Traits::MUTATION.live(Live::Steer, address)
                }
            }
            Self::EditMessage {
                runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    operation_target: Some(address),
                    history_invalidation: Conversation(conversation_id),
                    ..Traits::REVISION_CHECKED.live(Live::Edit, address)
                }
            }
            Self::DeleteUserMessage {
                runtime_id,
                conversation_id,
                ..
            } => {
                let address = CommandTarget::conversation(runtime_id, conversation_id);
                Traits {
                    history_invalidation: Conversation(conversation_id),
                    ..Traits::REVISION_CHECKED.live(Live::Delete, address)
                }
            }
            Self::ReadConfiguration => Traits {
                route: CommandRoute::ListRefresh,
                ..Traits::OBSERVATION
            },
            Self::ReadComposerOwner
            | Self::ReadComposerTarget { .. }
            | Self::ReadComposerAttachment { .. }
            | Self::SetObservedConversations { .. }
            | Self::ListResourceTemplates
            | Self::ReadResourceTemplate { .. }
            | Self::ImportRuntimeResource { .. }
            | Self::ReadResourceSyncStatus
            | Self::ReadResourceDrift { .. }
            | Self::ListUnmanagedRuntimeResources { .. }
            | Self::ReadResourceNativeImport { .. }
            | Self::QueryAnalytics { .. }
            | Self::ReadProviderAnalytics { .. }
            | Self::ReadRuntimeDiscoveryHints { .. }
            | Self::DiscoverRuntimes { .. }
            | Self::ReadComposerDrafts
            | Self::ReadComposerChoices
            | Self::ReadRuntimeStartChoice { .. }
            | Self::SearchConversations { .. }
            | Self::FindConversation { .. }
            | Self::ResolveAgentUri { .. }
            | Self::ReadAcpLaunch { .. }
            | Self::ReadAcpRuntimeInfo { .. }
            | Self::ReadAcpSessionConfig { .. }
            | Self::ReadRuntimeExecution { .. }
            | Self::ReadRuntimeHomeDirectory { .. }
            | Self::ReadRuntimeModels { .. }
            | Self::ReadComposerQuota { .. }
            | Self::ReadMessagePayloads { .. }
            | Self::ReadReasoningEfforts { .. }
            | Self::ReadBackgroundTasks { .. }
            | Self::ReadBackgroundTaskOutput { .. }
            | Self::ReadConversationActivity { .. }
            | Self::ReadWorkingChanges { .. }
            | Self::ReadWorkingBranch { .. }
            | Self::ReadFileDiff { .. }
            | Self::PreviewProviderImport { .. }
            | Self::ReadProviderBilling { .. }
            | Self::ReadProviderAccount { .. }
            | Self::ReadProviderQuota { .. }
            | Self::ReadProviderQuotaHistory { .. }
            | Self::ReadQuotaAlerts { .. }
            | Self::ReadProviderApiKey { .. }
            | Self::FetchPublicModelCatalog
            | Self::ReadAttachment { .. } => Traits::OBSERVATION,
            Self::UploadComposerAttachment { .. }
            | Self::SetOutlinePresentation { .. }
            | Self::SetLinkedConversationOpening { .. }
            | Self::SetUsageDisplay { .. }
            | Self::SetContextUsageDisplay { .. }
            | Self::SetStatusLine { .. }
            | Self::SetConversationBrowserPreferences { .. }
            | Self::SetComposerDraft { .. }
            | Self::SetNextTurnIntent { .. }
            | Self::SetRuntimeComposerChoices { .. }
            | Self::SetConversationBrowserLimits { .. }
            | Self::LoadMoreConversations { .. }
            | Self::ShowArchivedConversations { .. }
            | Self::SelectRuntime { .. }
            | Self::RefreshProviderQuota { .. }
            | Self::ClearProviderQuotaHistory { .. }
            | Self::InspectAuthentication { .. }
            | Self::PollAuthentication
            | Self::CancelAuthentication
            | Self::UploadAttachment { .. } => Traits::EFFECT,
            Self::SetRuntimeModelAutoCompaction { .. }
            | Self::StopBackgroundTask { .. }
            | Self::CloseSideChat { .. }
            | Self::StartAuthentication { .. }
            | Self::ReplyAuthentication { .. } => Traits::MUTATION,
            Self::SaveResourceTemplate { .. }
            | Self::DeleteResourceTemplate { .. }
            | Self::ApplyResourceTemplate { .. }
            | Self::SetProject { .. }
            | Self::UpsertRuntime { .. }
            | Self::UpsertRuntimeWithExecution { .. }
            | Self::UpsertAcpRuntime { .. }
            | Self::DeleteRuntime { .. }
            | Self::ApplyProviderImport { .. }
            | Self::SaveProviderBilling { .. }
            | Self::SaveProvider { .. }
            | Self::DeleteProvider { .. }
            | Self::SaveModelRoute { .. }
            | Self::DeleteModelRoute { .. }
            | Self::SaveModelTemplate { .. }
            | Self::DeleteModelTemplate { .. }
            | Self::LogoutAuthentication { .. } => Traits::REVISION_CHECKED,
        }
    }
}
