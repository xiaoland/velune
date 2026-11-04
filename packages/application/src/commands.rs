//! The complete public command surface dispatches to the same application use cases.
use crate::local::{Focus, Login, TurnRequest};
use crate::*;
use base64::Engine;
use std::io::Read;
use velune_application_api::{ComposerContextKey, ComposerInputOperation};

const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
impl Application {
    pub(crate) fn execute_command(
        &mut self,
        mut command: Command,
        remote: bool,
        view: &ClientViewId,
    ) -> Result<Response, Failure> {
        self.resolve_input_attachments(&mut command, view, remote)?;
        if let Command::ReplyRuntimeInteraction {
            runtime_id,
            conversation_id,
            interaction_id,
            ..
        } = &command
        {
            let snapshot = self
                .runtime
                .serve(
                    Focus::Conversation(conversation_id.clone()),
                    Login::Allows,
                    |core| core.snapshot(runtime_id, conversation_id),
                )
                .map_err(Failure::from)?;
            if !snapshot.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .pending_interactions
                    .iter()
                    .any(|interaction| interaction.id == *interaction_id)
            }) {
                return Err(Failure::new(
                    FailureKind::StaleInteraction,
                    "此审批已被处理或已失效",
                ));
            }
        }
        let payload_runtime = match &command {
            Command::ReadMessagePayloads { runtime_id, .. } => Some(runtime_id.clone()),
            _ => None,
        };
        let response = self
            .dispatch_command(command, view)
            .map_err(Failure::from)?;
        if remote {
            self.remote_response(response, payload_runtime)
        } else {
            Ok(response)
        }
    }

    fn dispatch_command(
        &mut self,
        command: Command,
        view: &ClientViewId,
    ) -> Result<Response, Error> {
        let application = self;
        Ok(match command {
            Command::ReadComposerOwner => {
                Response::ReadComposerOwner(application.composer.owner_id())
            }
            Command::ManageComposerInputs { operation } => {
                Response::ManageComposerInputs(application.composer.update(operation)?)
            }
            Command::UploadComposerAttachment { name, bytes } => {
                Response::UploadComposerAttachment(application.composer.upload(name, bytes)?)
            }
            Command::ReadComposerAttachment { attachment_id } => Response::ReadComposerAttachment(
                application.composer.read_attachment(&attachment_id)?,
            ),
            Command::ManageScheduledTasks { operation } => {
                Response::ManageScheduledTasks(application.runtime.scheduled_tasks(operation)?)
            }
            Command::ReleaseView => Response::ReleaseView,
            Command::ListResourceTemplates => {
                Response::ListResourceTemplates(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.list_resource_templates(),
                )?)
            }
            Command::ReadResourceTemplate { template_id } => {
                Response::ReadResourceTemplate(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.read_resource_template(template_id),
                )?)
            }
            Command::SaveResourceTemplate { template } => {
                Response::SaveResourceTemplate(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.save_resource_template(template),
                )?)
            }
            Command::DeleteResourceTemplate { template_id } => {
                application
                    .runtime
                    .serve(Focus::None, Login::Blocks, |core| {
                        core.delete_resource_template(template_id)
                    })?;
                Response::DeleteResourceTemplate
            }
            Command::ImportRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Response::ImportRuntimeResource(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.import_native_resource(runtime_id, kind, path),
            )?),
            Command::AdoptRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Response::AdoptRuntimeResource(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.adopt_native_resource(runtime_id, kind, path),
            )?),
            Command::ReadResourceSyncStatus => {
                Response::ReadResourceSyncStatus(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.resource_sync_status(),
                )?)
            }
            Command::ReadResourceDrift {
                runtime_id,
                template_id,
            } => Response::ReadResourceDrift(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.resource_drift(runtime_id, template_id),
            )?),
            Command::ListUnmanagedRuntimeResources { kind } => {
                Response::ListUnmanagedRuntimeResources(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.unmanaged_runtime_resources(kind),
                )?)
            }
            Command::ApplyResourceTemplate {
                template_id,
                targets,
            } => Response::ApplyResourceTemplate(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.apply_resource_template(template_id, targets),
            )?),
            Command::ReadResourceNativeImport {
                runtime_id,
                template_id,
            } => Response::ReadResourceNativeImport(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.resource_native_import(runtime_id, template_id),
            )?),
            Command::ResolveResourceBinding {
                runtime_id,
                template_id,
                resolution,
            } => {
                application
                    .runtime
                    .serve(Focus::None, Login::Blocks, |core| {
                        core.resolve_resource_binding(runtime_id, template_id, resolution)
                    })?;
                Response::ResolveResourceBinding
            }
            Command::QueryAnalytics { query } => {
                Response::QueryAnalytics(application.analytics_query(query)?)
            }
            Command::ReadRuntimeDiscoveryHints {
                user_home,
                overrides,
            } => Response::ReadRuntimeDiscoveryHints(crate::runtime_discovery::hints(
                &user_home, overrides,
            )?),
            Command::DiscoverRuntimes { probes } => {
                Response::DiscoverRuntimes(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.discover_runtimes(probes),
                )?)
            }
            Command::ReadConfiguration => Response::ReadConfiguration(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| core.list(),
            )?),
            Command::SetOutlinePresentation { presentation } => {
                Response::SetOutlinePresentation(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_outline_presentation(presentation),
                )?)
            }
            Command::SetLinkedConversationOpening { opening } => {
                Response::SetLinkedConversationOpening(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_linked_conversation_opening(opening),
                )?)
            }
            Command::SetUsageDisplay { display } => {
                Response::SetUsageDisplay(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_usage_display(display),
                )?)
            }
            Command::SetContextUsageDisplay { display } => {
                Response::SetContextUsageDisplay(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_context_usage_display(display),
                )?)
            }
            Command::SetStatusLine { items } => {
                Response::SetStatusLine(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_status_line(items),
                )?)
            }
            Command::SetConversationBrowserPreferences { preferences } => {
                Response::SetConversationBrowserPreferences(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_conversation_browser_preferences(preferences),
                )?)
            }
            Command::ReadComposerDrafts => {
                Response::ReadComposerDrafts(application.composer.legacy_drafts())
            }
            Command::SetComposerDraft {
                conversation_id,
                text,
            } => {
                application
                    .composer
                    .legacy_edit(conversation_id, Some(text), None)?;
                Response::SetComposerDraft
            }
            Command::ReadComposerChoices => {
                let mut choices =
                    application
                        .runtime
                        .serve(Focus::None, Login::Blocks, |core| core.composer_choices())?;
                choices.conversations = application.composer.legacy_intents();
                Response::ReadComposerChoices(choices)
            }
            Command::SetNextTurnIntent { intent } => {
                application.composer.legacy_edit(
                    intent.conversation_id.clone(),
                    None,
                    Some(intent),
                )?;
                Response::SetNextTurnIntent
            }
            Command::SetRuntimeComposerChoices { choices } => {
                application
                    .runtime
                    .serve(Focus::None, Login::Blocks, |runtime| {
                        runtime.set_runtime_composer_choices(choices)
                    })?;
                Response::SetRuntimeComposerChoices
            }
            Command::SetConversationBrowserLimits { limits } => {
                Response::SetConversationBrowserLimits(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.set_conversation_browser_limits(limits),
                )?)
            }
            Command::OrganizeConversations {
                conversations,
                change,
            } => Response::OrganizeConversations(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.organize_conversations(conversations, change),
            )?),
            Command::SetProject {
                path,
                primary_folder,
                name,
                folders,
            } => Response::SetProject(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.set_project(path, primary_folder, name, folders),
            )?),
            Command::LoadMoreConversations { group } => {
                Response::LoadMoreConversations(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.load_more_conversations(group),
                )?)
            }
            Command::ShowArchivedConversations { archived } => {
                Response::ShowArchivedConversations(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.show_archived_conversations(archived),
                )?)
            }
            Command::SearchConversations { text } => {
                Response::SearchConversations(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.search_conversations(text),
                )?)
            }
            Command::ResolveAgentUri { uri, runtime_id } => {
                Response::ResolveAgentUri(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.resolve_agent_uri(&uri, runtime_id.as_deref()),
                )?)
            }
            Command::FindConversation { conversation_id } => {
                Response::FindConversation(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.find_conversation(conversation_id),
                )?)
            }
            Command::UpsertRuntime { runtime } => {
                Response::UpsertRuntime(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.upsert_runtime(runtime),
                )?)
            }
            Command::UpsertRuntimeWithExecution { runtime, execution } => {
                Response::UpsertRuntimeWithExecution(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.upsert_runtime_with_execution(runtime, execution),
                )?)
            }
            Command::UpsertAcpRuntime {
                runtime,
                execution,
                launch,
            } => Response::UpsertAcpRuntime(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.upsert_acp_runtime(runtime, execution, launch),
            )?),
            Command::ReadAcpLaunch { runtime_id } => {
                Response::ReadAcpLaunch(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.acp_launch(runtime_id),
                )?)
            }
            Command::ReadAcpRuntimeInfo { runtime_id } => {
                Response::ReadAcpRuntimeInfo(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.acp_runtime_info(runtime_id),
                )?)
            }
            Command::ReadAcpSessionConfig {
                runtime_id,
                conversation_id,
            } => Response::ReadAcpSessionConfig(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.acp_session_config(&runtime_id, &conversation_id, None),
            )?),
            Command::SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                config_id,
                value,
            } => Response::SetAcpSessionConfig(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| {
                    core.acp_session_config(
                        &runtime_id,
                        &conversation_id,
                        Some((&config_id, value)),
                    )
                },
            )?),
            Command::DeleteRuntime { id } => Response::DeleteRuntime(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.delete_runtime(id),
            )?),
            Command::SelectRuntime { id } => Response::SelectRuntime(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.select_runtime(id),
            )?),
            Command::ReadRuntimeExecution { runtime_id } => {
                Response::ReadRuntimeExecution(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.runtime_execution(runtime_id),
                )?)
            }
            Command::ReadRuntimeHomeDirectory { runtime_id } => {
                Response::ReadRuntimeHomeDirectory(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.runtime_home_directory(runtime_id),
                )?)
            }
            Command::CreateConversation {
                runtime_id,
                cwd,
                selection,
            } => Response::CreateConversation(application.runtime.serve(
                Focus::New,
                Login::Blocks,
                |core| core.create_conversation(&runtime_id, cwd, selection),
            )?),
            Command::ReadComposerQuota { context, refresh } => {
                Response::ReadComposerQuota(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.composer_quota(context, refresh),
                )?)
            }
            Command::ReadRuntimeModels { runtime_id } => {
                Response::ReadRuntimeModels(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.runtime_models(runtime_id),
                )?)
            }
            Command::OpenConversation {
                runtime_id,
                conversation_id,
            } => Response::OpenConversation(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.open_conversation(&runtime_id, &conversation_id),
            )?),
            Command::ReadConversation {
                runtime_id,
                conversation_id,
            } => Response::ReadConversation(application.runtime.serve(
                Focus::Existing(conversation_id.clone()),
                Login::Blocks,
                |core| core.read_conversation(&runtime_id, &conversation_id),
            )?),
            Command::RenameConversation {
                runtime_id,
                conversation_id,
                title,
            } => Response::RenameConversation(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.rename_conversation(&runtime_id, &conversation_id, &title),
            )?),
            Command::DeleteConversation {
                runtime_id,
                conversation_id,
            } => {
                let result = application.runtime.serve(
                    Focus::Conversation(conversation_id.clone()),
                    Login::Blocks,
                    |core| core.delete_conversation(&runtime_id, &conversation_id),
                )?;
                application
                    .composer
                    .operate(ComposerInputOperation::Forget {
                        context: ComposerContextKey::Conversation {
                            owner_id: application.composer.owner_id(),
                            conversation_id,
                        },
                    })?;
                Response::DeleteConversation(result)
            }
            Command::ReadSnapshot {
                runtime_id,
                conversation_id,
            } => Response::ReadSnapshot(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Allows,
                |core| core.snapshot(&runtime_id, &conversation_id),
            )?),
            Command::ReadSnapshotUpdate {
                runtime_id,
                conversation_id,
                since,
            } => Response::ReadSnapshotUpdate(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Allows,
                |core| core.snapshot_update(&runtime_id, &conversation_id, since),
            )?),
            Command::ReadMessagePayloads {
                runtime_id,
                conversation_id,
                message_ids,
            } => Response::ReadMessagePayloads(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| Ok(core.message_payloads(&runtime_id, &conversation_id, &message_ids)),
            )?),
            Command::SendTurn {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
            } => Response::SendTurn(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| {
                    core.send_turn(TurnRequest {
                        source_runtime_id,
                        conversation_id,
                        runtime_id: target_runtime_id,
                        selection,
                        turn,
                    })
                },
            )?),
            Command::ReadReasoningEfforts {
                runtime_id,
                model_record_key,
            } => Response::ReadReasoningEfforts(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.runtime_reasoning_efforts(runtime_id, model_record_key),
            )?),
            Command::EditSendQueue {
                runtime_id: _,
                conversation_id,
                operation,
            } => Response::EditSendQueue(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.send_queue(&conversation_id, operation),
            )?),
            Command::SteerTurn {
                runtime_id: _,
                conversation_id,
                turn,
            } => Response::SteerTurn(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.steer_turn(&turn),
            )?),
            Command::CompactConversation {
                runtime_id,
                conversation_id,
                selection,
            } => Response::CompactConversation(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.compact_conversation(&runtime_id, selection),
            )?),
            Command::ResumeTurn {
                runtime_id,
                conversation_id,
                selection,
                effort,
                permission_mode,
            } => Response::ResumeTurn(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.resume_turn(&runtime_id, selection, effort, permission_mode),
            )?),
            Command::EditMessage {
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn,
            } => Response::EditMessage(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| {
                    core.edit_message(
                        &runtime_id,
                        &conversation_id,
                        &message_id,
                        placement,
                        selection,
                        turn,
                    )
                },
            )?),
            Command::ForkConversation {
                runtime_id,
                conversation_id,
                message_id,
            } => Response::ForkConversation(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.fork_message(&runtime_id, &conversation_id, &message_id),
            )?),
            Command::Cancel {
                runtime_id,
                conversation_id,
            } => Response::Cancel(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.cancel(&runtime_id),
            )?),
            Command::OpenSideChat {
                runtime_id,
                conversation_id,
                selection,
                effort,
                permission_mode,
            } => Response::OpenSideChat(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| {
                    core.open_side_chat(
                        &runtime_id,
                        &conversation_id,
                        selection,
                        effort,
                        permission_mode,
                    )
                },
            )?),
            Command::SendSideChat {
                side_chat_id,
                target_runtime_id,
                selection,
                turn,
            } => Response::SendSideChat(application.runtime.serve(
                Focus::Existing(side_chat_id.clone()),
                Login::Blocks,
                |core| core.send_side_chat(&side_chat_id, &target_runtime_id, selection, turn),
            )?),
            Command::CloseSideChat { side_chat_id } => {
                application.runtime.serve(
                    Focus::Existing(side_chat_id.clone()),
                    Login::Allows,
                    |core| core.close_side_chat(&side_chat_id),
                )?;
                Response::CloseSideChat
            }
            Command::StopSubAgents {
                runtime_id,
                conversation_id,
            } => Response::StopSubAgents(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.stop_sub_agents(&runtime_id),
            )?),
            Command::ReadConversationActivity { since } => {
                Response::ReadConversationActivity(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| Ok(core.conversation_activity(since)),
                )?)
            }
            Command::ReadWorkingChanges {
                runtime_id,
                conversation_id,
            } => Response::ReadWorkingChanges(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.working_changes(runtime_id, conversation_id),
            )?),
            Command::ReadWorkingBranch {
                runtime_id,
                conversation_id,
            } => Response::ReadWorkingBranch(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.working_branch(runtime_id, conversation_id),
            )?),
            Command::ReadFileDiff {
                runtime_id,
                conversation_id,
                path,
            } => Response::ReadFileDiff(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.file_diff(runtime_id, conversation_id, path),
            )?),
            Command::PreviewProviderImport {
                gateway_id,
                runtime_id,
            } => Response::PreviewProviderImport(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.preview_provider_import(gateway_id, runtime_id),
            )?),
            Command::ApplyProviderImport {
                gateway_id,
                runtime_id,
                preview_token,
                selections,
                replace_existing,
            } => Response::ApplyProviderImport(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| {
                    core.apply_provider_import(
                        gateway_id,
                        runtime_id,
                        preview_token,
                        selections,
                        replace_existing,
                    )
                },
            )?),
            Command::ReadProviderBilling {
                gateway_id,
                provider_id,
                refresh,
            } => Response::ReadProviderBilling(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| core.provider_billing(gateway_id, provider_id, refresh),
            )?),
            Command::SaveProviderBilling {
                gateway_id,
                provider_id,
                configuration,
                authentication_edit,
                confirmed,
            } => Response::SaveProviderBilling(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| {
                    core.save_provider_billing(
                        gateway_id,
                        provider_id,
                        configuration,
                        authentication_edit,
                        confirmed,
                    )
                },
            )?),
            Command::ReadProviderQuota {
                gateway_id,
                provider_id,
            } => Response::ReadProviderQuota(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.provider_quota(gateway_id, provider_id, false),
            )?),
            Command::RefreshProviderQuota {
                gateway_id,
                provider_id,
            } => Response::RefreshProviderQuota(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.provider_quota(gateway_id, provider_id, true),
            )?),
            Command::ReadProviderQuotaHistory {
                gateway_id,
                provider_id,
                since_ms,
            } => Response::ReadProviderQuotaHistory(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.provider_quota_history(gateway_id, provider_id, since_ms),
            )?),
            Command::ClearProviderQuotaHistory {
                gateway_id,
                provider_id,
            } => Response::ClearProviderQuotaHistory(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.clear_provider_quota_history(gateway_id, provider_id),
            )?),
            Command::ReadQuotaAlerts { since } => Response::ReadQuotaAlerts(
                application
                    .runtime
                    .serve(Focus::None, Login::Blocks, |core| core.quota_alerts(since))?,
            ),
            Command::SaveProvider {
                gateway_id,
                provider,
                authentication_edit,
                quota_authentication_edit,
            } => Response::SaveProvider(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| {
                    core.save_provider(
                        gateway_id,
                        provider,
                        authentication_edit,
                        quota_authentication_edit,
                    )
                },
            )?),
            Command::DeleteProvider {
                gateway_id,
                provider_id,
            } => Response::DeleteProvider(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.delete_provider(gateway_id, provider_id),
            )?),
            Command::SaveModelRoute { gateway_id, route } => {
                Response::SaveModelRoute(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.save_model_route(gateway_id, route),
                )?)
            }
            Command::DeleteModelRoute {
                gateway_id,
                route_record_key,
            } => Response::DeleteModelRoute(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.delete_model_route(gateway_id, route_record_key),
            )?),
            Command::ReadProviderApiKey {
                gateway_id,
                provider_id,
            } => Response::ReadProviderApiKey(application.runtime.serve(
                Focus::None,
                Login::Blocks,
                |core| core.read_provider_api_key(gateway_id, provider_id),
            )?),
            Command::SaveModelTemplate { template } => {
                Response::SaveModelTemplate(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.save_model_template(template),
                )?)
            }
            Command::DeleteModelTemplate { id } => {
                Response::DeleteModelTemplate(application.runtime.serve(
                    Focus::None,
                    Login::Blocks,
                    |core| core.delete_model_template(id),
                )?)
            }
            Command::ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                interaction_id,
                reply,
            } => Response::ReplyRuntimeInteraction(application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Blocks,
                |core| core.reply_runtime_interaction(&runtime_id, &interaction_id, reply),
            )?),
            Command::InspectAuthentication {
                gateway_id,
                provider_id,
            } => Response::InspectAuthentication(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| core.authentication_inspect(gateway_id, provider_id),
            )?),
            Command::StartAuthentication {
                gateway_id,
                provider_id,
            } => Response::StartAuthentication(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| core.authentication_start(gateway_id, provider_id),
            )?),
            Command::LogoutAuthentication {
                gateway_id,
                provider_id,
            } => Response::LogoutAuthentication(application.runtime.serve(
                Focus::None,
                Login::Allows,
                |core| core.authentication_logout(gateway_id, provider_id),
            )?),
            Command::PollAuthentication => {
                Response::PollAuthentication(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.authentication_poll(),
                )?)
            }
            Command::ReplyAuthentication { prompt_id, value } => {
                Response::ReplyAuthentication(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.authentication_reply(prompt_id, value),
                )?)
            }
            Command::CancelAuthentication => {
                Response::CancelAuthentication(application.runtime.serve(
                    Focus::None,
                    Login::Allows,
                    |core| core.authentication_cancel(),
                )?)
            }
            Command::FetchPublicModelCatalog => {
                Response::FetchPublicModelCatalog(crate::model_catalog::fetch_models_dev()?)
            }
            Command::UploadAttachment { name, bytes } => {
                Response::UploadAttachment(application.store_attachment(view, name, bytes)?)
            }
            Command::ReadAttachment {
                runtime_id,
                conversation_id,
                message_id,
                block_index,
            } => Response::ReadAttachment(application.attachment_content(
                runtime_id,
                conversation_id,
                message_id,
                block_index,
            )?),
        })
    }

    fn store_attachment(
        &mut self,
        view: &ClientViewId,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<AttachmentReference, Error> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.len() > 240
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
            || bytes.len() > MAX_ATTACHMENT_BYTES
        {
            return Err(Error::invalid("附件名称或大小无效；单个附件最多 20 MiB"));
        }
        let reference = format!("attachment:{}", new_record_key());
        let root = self
            .attachment_directory
            .join(reference.trim_start_matches("attachment:"));
        std::fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.attachment_directory,
                std::fs::Permissions::from_mode(0o700),
            )?;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = root.join(&name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let pending = root.join(".upload");
        let publish = (|| -> Result<(), std::io::Error> {
            let mut file = options.open(&pending)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.sync_all()?;
            std::fs::rename(&pending, &path)
        })();
        if let Err(error) = publish {
            let _ = std::fs::remove_file(&pending);
            let _ = std::fs::remove_dir(&root);
            return Err(error.into());
        }

        self.uploaded_attachments
            .insert(reference.clone(), (view.clone(), path));
        Ok(AttachmentReference {
            reference,
            name,
            bytes: bytes.len() as u64,
        })
    }

    fn resolve_input_attachments(
        &self,
        command: &mut Command,
        view: &ClientViewId,
        remote: bool,
    ) -> Result<(), Failure> {
        let resolve = |turn: &mut ComposedTurn| -> Result<(), Failure> {
            for reference in &mut turn.attachments {
                if let Some(id) = reference.strip_prefix("composer:") {
                    *reference = self
                        .composer
                        .attachment_path(id)
                        .map_err(Failure::from)?
                        .to_string_lossy()
                        .into_owned();
                    continue;
                }
                if !remote && !reference.starts_with("attachment:") {
                    continue;
                }
                let Some((owner, path)) = self
                    .uploaded_attachments
                    .get(reference)
                    .filter(|(owner, _)| owner == view)
                else {
                    return Err(Failure::invalid(
                        "远程附件必须使用当前视图上传取得的内容引用",
                    ));
                };
                let _ = owner;
                *reference = path.to_string_lossy().into_owned();
            }
            Ok(())
        };
        if let Some(turn) = command.composed_turn_mut() {
            resolve(turn)?;
        }
        Ok(())
    }

    fn runs_locally(&mut self, runtime_id: String) -> Result<bool, Error> {
        let execution = self.runtime.serve(Focus::None, Login::Blocks, |core| {
            core.runtime_execution(runtime_id)
        })?;
        Ok(matches!(execution.transport, RuntimeTransport::Local))
    }

    fn attachment_content(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    ) -> Result<AttachmentContent, Error> {
        if !self.runs_locally(runtime_id.clone())? {
            return Err(Error::Unsupported(
                "SSH 执行位置的附件内容尚不支持读取".into(),
            ));
        }
        let messages = self.runtime.serve(Focus::None, Login::Allows, |core| {
            Ok(core.message_payloads(
                &runtime_id,
                &conversation_id,
                std::slice::from_ref(&message_id),
            ))
        })?;
        let block = messages
            .iter()
            .find(|message| message.id == message_id)
            .and_then(|message| message.blocks.get(block_index as usize))
            .ok_or_else(|| Error::invalid("附件不属于该会话消息"))?;
        match block {
            DeliveredBlock::Image {
                media_type,
                source: DeliveredImageSource::Path { path },
            } => Ok(AttachmentContent {
                media_type: media_type.clone(),
                bytes: read_content(path)?,
            }),
            DeliveredBlock::Image {
                media_type,
                source:
                    DeliveredImageSource::Data {
                        data: MessagePayload::Inline { text },
                    },
            } => Ok(AttachmentContent {
                media_type: media_type.clone(),
                bytes: base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(|_| Error::invalid("图片内容编码无效"))?,
            }),
            DeliveredBlock::File { path } => Ok(AttachmentContent {
                media_type: None,
                bytes: read_content(path)?,
            }),
            _ => Err(Error::invalid("此消息块不包含可读取的附件")),
        }
    }

    fn remote_response(
        &mut self,
        mut response: Response,
        payload_runtime: Option<String>,
    ) -> Result<Response, Failure> {
        let snapshots = match &mut response {
            Response::CreateConversation(result)
            | Response::OpenConversation(result)
            | Response::ReadConversation(result)
            | Response::ReadSnapshot(result)
            | Response::SendTurn(result)
            | Response::EditSendQueue(result)
            | Response::SteerTurn(result)
            | Response::ResumeTurn(result)
            | Response::CompactConversation(result)
            | Response::EditMessage(result)
            | Response::ForkConversation(result)
            | Response::Cancel(result)
            | Response::StopSubAgents(result)
            | Response::OpenSideChat(result)
            | Response::SendSideChat(result)
            | Response::ReplyRuntimeInteraction(result) => result.snapshot.as_mut(),
            Response::ReadSnapshotUpdate(SnapshotUpdate::Changed { snapshot }) => Some(snapshot),
            _ => None,
        };
        if let Some(snapshot) = snapshots {
            let local = self.runs_locally(snapshot.context_runtime_id.clone())?;
            for message in &mut snapshot.messages {
                encode_images(message, false, local);
            }
        }
        if let Response::ReadMessagePayloads(messages) = &mut response {
            let local = match payload_runtime {
                Some(runtime) => self.runs_locally(runtime)?,
                None => false,
            };
            for message in messages {
                encode_images(message, true, local);
            }
        }
        Ok(response)
    }
}
fn read_content(path: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_ATTACHMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(Error::invalid("附件超过 20 MiB 内容传递上限"));
    }
    Ok(bytes)
}
fn encode_images(message: &mut DeliveredMessage, full: bool, local: bool) {
    for block in &mut message.blocks {
        if let DeliveredBlock::Image {
            source: DeliveredImageSource::Path { path },
            ..
        } = block
        {
            if !local {
                *block = DeliveredBlock::Notice {
                    text: "此图片属于 SSH 执行位置，当前无法读取内容".into(),
                };
                continue;
            }
            let bytes = match read_content(path) {
                Ok(bytes) => bytes,
                Err(_) => {
                    *block = DeliveredBlock::Notice {
                        text: "图片已不可读取或超过 20 MiB 内容传递上限".into(),
                    };
                    continue;
                }
            };
            *block = match block {
                DeliveredBlock::Image { media_type, .. } => DeliveredBlock::Image {
                    media_type: media_type.clone(),
                    source: DeliveredImageSource::Data {
                        data: if full || bytes.len() <= 32 * 1024 {
                            MessagePayload::Inline {
                                text: base64::engine::general_purpose::STANDARD.encode(bytes),
                            }
                        } else {
                            MessagePayload::Deferred {
                                bytes: bytes.len() as u64,
                            }
                        },
                    },
                },
                _ => unreachable!(),
            };
        }
    }
}
