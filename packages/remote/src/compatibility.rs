//! Keep v3 application traffic within the shared contract of the shipped
//! v3 peers. The ledger always keeps the original result.
use crate::{PROTOCOL_VERSION, RequestStatus, protocol::Reply};
use std::collections::HashSet;
use velune_application_api::{
    Command, ExecutionBackend, Failure, FailureKind, GatewaySummary, HandoffRequirements,
    NextTurnInput, Protocol, ProviderAuthenticationSource, ProviderQuotaSource, ProviderSummary,
    Response, RuntimeTypeDescriptor, SendQueueOperation,
};

fn upgrade_required() -> Failure {
    Failure::new(
        FailureKind::Unsupported,
        "此操作需要两端更新到支持它的 Velune 版本",
    )
}

/// Shape an observation for old hosts, or decline writes that a v3 host
/// could reject or interpret differently. A business command is never probed.
pub(crate) fn prepare_command(version: u16, command: &mut Command) -> Result<(), Failure> {
    if version != PROTOCOL_VERSION {
        return Ok(());
    }
    match command {
        Command::ReadBackgroundTasks { manual, .. } => *manual = false,
        Command::SetRuntimeModelAutoCompaction { .. }
        | Command::EditSendQueue {
            operation: SendQueueOperation::Withdraw { .. },
            ..
        } => return Err(upgrade_required()),
        Command::SteerTurn {
            target_runtime_id,
            turn,
            ..
        } if target_runtime_id.is_some() || turn.permission_mode.is_some() => {
            return Err(upgrade_required());
        }
        Command::SaveProvider { provider, .. }
            if provider.codex_client_version.is_some()
                || !legacy_provider(
                    provider.protocol,
                    &provider.execution_backend,
                    provider.authentication_source,
                    provider.quota_source,
                ) =>
        {
            return Err(upgrade_required());
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn reply_for_wire(version: u16, mut reply: Reply) -> Reply {
    if version == PROTOCOL_VERSION
        && let Reply::Status(RequestStatus::Completed(Ok(application))) = &mut reply
        && let Err(failure) = project_response(&mut application.response)
    {
        return Reply::Status(RequestStatus::Completed(Err(failure)));
    }
    reply
}

fn project_response(response: &mut Response) -> Result<(), Failure> {
    match response {
        Response::ReadConfiguration(value)
        | Response::SetConversationBrowserPreferences(value)
        | Response::OrganizeConversations(value)
        | Response::SetProject(value)
        | Response::LoadMoreConversations(value)
        | Response::ShowArchivedConversations(value)
        | Response::SelectRuntime(value)
        | Response::RenameConversation(value)
        | Response::DeleteConversation(value) => {
            value.protocols.retain(|item| item.id != Protocol::Gemini);
            value.runtime_types.iter_mut().for_each(project_runtime);
            value
                .provider_import_types
                .iter_mut()
                .for_each(project_runtime);
            project_gateways(&mut value.gateways);
        }
        Response::UpsertRuntime(value)
        | Response::UpsertRuntimeWithExecution(value)
        | Response::UpsertAcpRuntime(value)
        | Response::DeleteRuntime(value) => {
            value.runtime_types.iter_mut().for_each(project_runtime);
        }
        Response::SaveProvider(value)
        | Response::DeleteProvider(value)
        | Response::SaveModelRoute(value)
        | Response::DeleteModelRoute(value)
        | Response::LogoutAuthentication(value) => project_gateways(&mut value.gateways),
        Response::ApplyProviderImport(value) => project_gateways(&mut value.gateways),
        Response::StartAuthentication(value)
        | Response::PollAuthentication(value)
        | Response::ReplyAuthentication(value)
        | Response::CancelAuthentication(value) => {
            if let Some(gateways) = &mut value.gateways {
                project_gateways(gateways);
            }
        }
        Response::ReadComposerTargetBasis(value) => project_next_turn(&mut value.input),
        Response::ReadHandoffRequirements(HandoffRequirements::PortableSummaryRequired {
            summary_basis,
            ..
        }) => project_next_turn(summary_basis),
        Response::PreviewProviderImport(value) => {
            for provider in &mut value.providers {
                if provider.protocol == Some(Protocol::Gemini) {
                    provider.protocol = None;
                    provider.can_import = false;
                    provider.issues.push(upgrade_required().message);
                }
            }
        }
        Response::InspectAuthentication(value)
            if value.capabilities.protocol == Protocol::Gemini =>
        {
            return Err(upgrade_required());
        }
        Response::QueryAnalytics(value) => {
            value
                .requests
                .retain(|row| row.protocol != Protocol::Gemini);
        }
        _ => {}
    }
    Ok(())
}

fn project_runtime(runtime: &mut RuntimeTypeDescriptor) {
    runtime
        .supported_protocols
        .retain(|value| *value != Protocol::Gemini);
    runtime
        .supported_provider_protocols
        .retain(|value| *value != Protocol::Gemini);
}

fn project_next_turn(value: &mut NextTurnInput) {
    if let Some(runtime) = &mut value.runtime_type {
        project_runtime(runtime);
    }
    value.providers.retain(provider_visible);
}

fn project_gateways(gateways: &mut [GatewaySummary]) {
    for gateway in gateways {
        let provider_count = gateway.providers.len();
        gateway.providers.retain(provider_visible);
        if gateway.providers.len() == provider_count {
            continue;
        }
        let keys: HashSet<_> = gateway
            .providers
            .iter()
            .flat_map(|provider| provider.models.iter().map(|model| &model.record_key))
            .collect();
        // A partly projected route would invite an old editor to overwrite
        // hidden members. Keep the complete route or leave it off that editor.
        gateway
            .routes
            .retain(|route| route.members.iter().all(|key| keys.contains(key)));
    }
}

fn provider_visible(provider: &ProviderSummary) -> bool {
    provider.codex_client_version.is_none()
        && legacy_provider(
            provider.protocol,
            &provider.execution_backend,
            provider.authentication_source,
            provider.quota_source,
        )
}

fn legacy_provider(
    protocol: Protocol,
    backend: &ExecutionBackend,
    authentication: ProviderAuthenticationSource,
    quota: ProviderQuotaSource,
) -> bool {
    protocol != Protocol::Gemini
        && !matches!(backend, ExecutionBackend::AntigravityCli { .. })
        && authentication != ProviderAuthenticationSource::AntigravityCli
        && quota != ProviderQuotaSource::AntigravityCliUsage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::APPLICATION_WIRE_VERSION;
    use serde::Deserialize;
    use serde_json::json;
    use velune_application_api::{
        ApplicationReply, ConfigurationSnapshot, ConversationBrowserLimits, ProtocolDescriptor,
        Revision,
    };

    // The shipped alpha.1 tags. Keep the old decoder independent of today's
    // Protocol enum so adding another value cannot accidentally fix the test.
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    enum LegacyProtocol {
        ChatCompletionsV1,
        ResponsesV1,
        MessagesV1,
    }
    #[derive(Debug, Deserialize)]
    struct LegacyDescriptor {
        id: LegacyProtocol,
    }

    #[test]
    fn v3_configuration_hides_unknown_protocols_without_changing_modern_results() {
        let snapshot = ConfigurationSnapshot {
            conversations: Vec::new(),
            conversation_groups: Vec::new(),
            conversation_projects: Vec::new(),
            working_directory_projects: Default::default(),
            shows_archived: false,
            history_failures: Vec::new(),
            gateways: Vec::new(),
            runtime_instances: Vec::new(),
            runtime_types: Vec::new(),
            model_templates: Vec::new(),
            conversation_browser_limits: ConversationBrowserLimits {
                page: 20,
                total: 100,
            },
            outline_presentation: Default::default(),
            linked_conversation_opening: Default::default(),
            usage_display: Default::default(),
            context_usage_display: Default::default(),
            conversation_browser_preferences: Default::default(),
            status_line: Vec::new(),
            provider_import_types: Vec::new(),
            protocols: [Protocol::Responses, Protocol::Gemini]
                .into_iter()
                .map(|id| ProtocolDescriptor {
                    id,
                    name: id.id().into(),
                    supported: true,
                })
                .collect(),
            selected_runtime_instance_id: None,
        };
        let reply = || {
            Reply::Status(RequestStatus::Completed(Ok(Box::new(ApplicationReply {
                revision: Revision(42),
                response: Response::ReadConfiguration(snapshot.clone()),
            }))))
        };
        let encoded = |version| {
            let Reply::Status(RequestStatus::Completed(Ok(application))) =
                reply_for_wire(version, reply())
            else {
                panic!("expected a configuration result")
            };
            assert_eq!(application.revision, Revision(42));
            let Response::ReadConfiguration(configuration) = application.response else {
                panic!("expected a configuration snapshot")
            };
            serde_json::to_value(configuration.protocols).unwrap()
        };
        let modern = encoded(APPLICATION_WIRE_VERSION);
        assert!(serde_json::from_value::<Vec<LegacyDescriptor>>(modern.clone()).is_err());
        assert_eq!(modern[1]["id"], "geminiGenerateContent");
        let legacy: Vec<LegacyDescriptor> =
            serde_json::from_value(encoded(PROTOCOL_VERSION)).unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].id, LegacyProtocol::ResponsesV1);
    }

    #[test]
    fn v3_clients_use_the_old_background_read_shape_and_decline_new_writes() {
        let mut read = Command::ReadBackgroundTasks {
            runtime_id: "runtime".into(),
            conversation_id: "session".into(),
            manual: true,
        };
        prepare_command(PROTOCOL_VERSION, &mut read).unwrap();
        let frame = serde_json::to_value(read).unwrap();
        assert_eq!(
            frame,
            json!({
                "command": "readBackgroundTasks",
                "parameters": {"runtime_id": "runtime", "conversation_id": "session"}
            })
        );

        let mut write = Command::EditSendQueue {
            submission_id: None,
            runtime_id: "runtime".into(),
            conversation_id: "session".into(),
            operation: SendQueueOperation::Withdraw {
                id: "item".into(),
                expected_revision: 1,
            },
        };
        assert_eq!(
            prepare_command(PROTOCOL_VERSION, &mut write)
                .unwrap_err()
                .kind,
            FailureKind::Unsupported
        );
        prepare_command(APPLICATION_WIRE_VERSION, &mut write).unwrap();
    }
}
