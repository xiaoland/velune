//! The complete public command surface dispatches to the same application use cases.
use crate::*;
use base64::Engine;
use std::io::Read;

const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
impl Application {
    pub(crate) fn execute_command(
        &mut self,
        mut command: Command,
        remote: bool,
        view: &ClientViewId,
    ) -> Result<Response, Failure> {
        if remote {
            self.resolve_remote_attachments(&mut command, view)?;
        }
        if let Command::ReplyRuntimeInteraction {
            runtime_id,
            conversation_id,
            interaction_id,
            ..
        } = &command
        {
            let snapshot = self
                .snapshot(runtime_id.clone(), conversation_id.clone())
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
            Command::MessagePayloads { runtime_id, .. } => Some(runtime_id.clone()),
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
            Command::ReleaseView => Response::ReleaseView,
            Command::RuntimeResourceCatalog {
                runtime_id,
                cwd,
                kind,
                scope,
            } => Response::RuntimeResourceCatalog(
                application.runtime_resource_catalog(runtime_id, cwd, kind, scope)?,
            ),
            Command::ReadRuntimeResource {
                runtime_id,
                cwd,
                key,
            } => Response::ReadRuntimeResource(
                application.read_runtime_resource(runtime_id, cwd, key)?,
            ),
            Command::WriteRuntimeResource {
                runtime_id,
                cwd,
                key,
                expected,
                bytes,
                executable,
            } => Response::WriteRuntimeResource(
                application
                    .write_runtime_resource(runtime_id, cwd, key, expected, bytes, executable)?,
            ),
            Command::DeleteRuntimeResource {
                runtime_id,
                cwd,
                key,
                expected,
            } => {
                application.delete_runtime_resource(runtime_id, cwd, key, expected)?;
                Response::DeleteRuntimeResource
            }
            Command::ListResourceTemplates => {
                Response::ListResourceTemplates(application.list_resource_templates()?)
            }
            Command::ReadResourceTemplate { template_id } => {
                Response::ReadResourceTemplate(application.read_resource_template(template_id)?)
            }
            Command::SaveResourceTemplate { template } => {
                Response::SaveResourceTemplate(application.save_resource_template(template)?)
            }
            Command::DeleteResourceTemplate { template_id } => {
                application.delete_resource_template(template_id)?;
                Response::DeleteResourceTemplate
            }
            Command::ImportRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Response::ImportRuntimeResource(
                application.import_runtime_resource(runtime_id, kind, path)?,
            ),
            Command::AdoptRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Response::AdoptRuntimeResource(
                application.adopt_runtime_resource(runtime_id, kind, path)?,
            ),
            Command::ResourceSyncStatus => {
                Response::ResourceSyncStatus(application.resource_sync_status()?)
            }
            Command::ResourceDrift {
                runtime_id,
                template_id,
            } => Response::ResourceDrift(application.resource_drift(runtime_id, template_id)?),
            Command::UnmanagedRuntimeResources { kind } => {
                Response::UnmanagedRuntimeResources(application.unmanaged_runtime_resources(kind)?)
            }
            Command::ApplyResourceTemplate {
                template_id,
                targets,
            } => Response::ApplyResourceTemplate(
                application.apply_resource_template(template_id, targets)?,
            ),
            Command::ResourceNativeImport {
                runtime_id,
                template_id,
            } => Response::ResourceNativeImport(
                application.resource_native_import(runtime_id, template_id)?,
            ),
            Command::ResolveResourceBinding {
                runtime_id,
                template_id,
                resolution,
            } => {
                application.resolve_resource_binding(runtime_id, template_id, resolution)?;
                Response::ResolveResourceBinding
            }
            Command::AnalyticsQuery { query } => {
                Response::AnalyticsQuery(application.analytics_query(query)?)
            }
            Command::RuntimeDiscoveryHints {
                user_home,
                overrides,
            } => Response::RuntimeDiscoveryHints(
                application.runtime_discovery_hints(user_home, overrides)?,
            ),
            Command::DiscoverRuntimes { probes } => {
                Response::DiscoverRuntimes(application.discover_runtimes(probes)?)
            }
            Command::List => Response::List(application.list()?),
            Command::SetOutlinePresentation { presentation } => Response::SetOutlinePresentation(
                application.set_outline_presentation(presentation)?,
            ),
            Command::SetLinkedConversationOpening { opening } => {
                Response::SetLinkedConversationOpening(
                    application.set_linked_conversation_opening(opening)?,
                )
            }
            Command::SetUsageDisplay { display } => {
                Response::SetUsageDisplay(application.set_usage_display(display)?)
            }
            Command::SetStatusLine { items } => {
                Response::SetStatusLine(application.set_status_line(items)?)
            }
            Command::SetConversationBrowserPreferences { preferences } => {
                Response::SetConversationBrowserPreferences(
                    application.set_conversation_browser_preferences(preferences)?,
                )
            }
            Command::ComposerDrafts => Response::ComposerDrafts(application.composer_drafts()?),
            Command::SetComposerDraft {
                conversation_id,
                text,
            } => {
                application.set_composer_draft(conversation_id, text)?;
                Response::SetComposerDraft
            }
            Command::SetConversationBrowserLimits { limits } => {
                Response::SetConversationBrowserLimits(
                    application.set_conversation_browser_limits(limits)?,
                )
            }
            Command::OrganizeConversations {
                conversations,
                change,
            } => Response::OrganizeConversations(
                application.organize_conversations(conversations, change)?,
            ),
            Command::SetProject {
                path,
                primary_folder,
                name,
                folders,
            } => Response::SetProject(application.set_project(
                path,
                primary_folder,
                name,
                folders,
            )?),
            Command::LoadMoreConversations { group } => {
                Response::LoadMoreConversations(application.load_more_conversations(group)?)
            }
            Command::ShowArchivedConversations { archived } => Response::ShowArchivedConversations(
                application.show_archived_conversations(archived)?,
            ),
            Command::SearchConversations { text } => {
                Response::SearchConversations(application.search_conversations(text)?)
            }
            Command::FindConversation { conversation_id } => {
                Response::FindConversation(application.find_conversation(conversation_id)?)
            }
            Command::UpsertRuntime { runtime } => {
                Response::UpsertRuntime(application.upsert_runtime(runtime)?)
            }
            Command::UpsertRuntimeWithExecution { runtime, execution } => {
                Response::UpsertRuntimeWithExecution(
                    application.upsert_runtime_with_execution(runtime, execution)?,
                )
            }
            Command::UpsertAcpRuntime {
                runtime,
                execution,
                launch,
            } => Response::UpsertAcpRuntime(
                application.upsert_acp_runtime(runtime, execution, launch)?,
            ),
            Command::AcpLaunch { runtime_id } => {
                Response::AcpLaunch(application.acp_launch(runtime_id)?)
            }
            Command::AcpRuntimeInfo { runtime_id } => {
                Response::AcpRuntimeInfo(application.acp_runtime_info(runtime_id)?)
            }
            Command::OpenAcpConversation {
                runtime_id,
                native_session_id,
                cwd,
            } => Response::OpenAcpConversation(application.open_acp_conversation(
                runtime_id,
                native_session_id,
                cwd,
            )?),
            Command::AcpSessionConfig {
                runtime_id,
                conversation_id,
            } => Response::AcpSessionConfig(
                application.acp_session_config(runtime_id, conversation_id)?,
            ),
            Command::SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                config_id,
                value,
            } => Response::SetAcpSessionConfig(application.set_acp_session_config(
                runtime_id,
                conversation_id,
                config_id,
                value,
            )?),
            Command::DeleteRuntime { id } => {
                Response::DeleteRuntime(application.delete_runtime(id)?)
            }
            Command::SelectRuntime { id } => {
                Response::SelectRuntime(application.select_runtime(id)?)
            }
            Command::RuntimeExecution { runtime_id } => {
                Response::RuntimeExecution(application.runtime_execution(runtime_id)?)
            }
            Command::RuntimeHomeDirectory { runtime_id } => {
                Response::RuntimeHomeDirectory(application.runtime_home_directory(runtime_id)?)
            }
            Command::ConfigureRuntimeExecution {
                runtime_id,
                execution,
            } => Response::ConfigureRuntimeExecution(
                application.configure_runtime_execution(runtime_id, execution)?,
            ),
            Command::CreateConversation {
                runtime_id,
                cwd,
                model_record_key,
            } => Response::CreateConversation(application.create_conversation(
                runtime_id,
                cwd,
                model_record_key,
            )?),
            Command::CreateConversationWithSelection {
                runtime_id,
                cwd,
                selection,
            } => Response::CreateConversationWithSelection(
                application.create_conversation_with_selection(runtime_id, cwd, selection)?,
            ),
            Command::RuntimeModels { runtime_id } => {
                Response::RuntimeModels(application.runtime_models(runtime_id)?)
            }
            Command::NextTurnModelSelection => {
                Response::NextTurnModelSelection(application.next_turn_model_selection()?)
            }
            Command::OpenConversation {
                runtime_id,
                conversation_id,
            } => Response::OpenConversation(
                application.open_conversation(runtime_id, conversation_id)?,
            ),
            Command::ReadConversation {
                runtime_id,
                conversation_id,
            } => Response::ReadConversation(
                application.read_conversation(runtime_id, conversation_id)?,
            ),
            Command::RenameConversation {
                runtime_id,
                conversation_id,
                title,
            } => Response::RenameConversation(application.rename_conversation(
                runtime_id,
                conversation_id,
                title,
            )?),
            Command::DeleteConversation {
                runtime_id,
                conversation_id,
            } => Response::DeleteConversation(
                application.delete_conversation(runtime_id, conversation_id)?,
            ),
            Command::Snapshot {
                runtime_id,
                conversation_id,
            } => Response::Snapshot(application.snapshot(runtime_id, conversation_id)?),
            Command::SnapshotUpdate {
                runtime_id,
                conversation_id,
                since,
            } => Response::SnapshotUpdate(application.snapshot_update(
                runtime_id,
                conversation_id,
                since,
            )?),
            Command::MessagePayloads {
                runtime_id,
                conversation_id,
                message_ids,
            } => Response::MessagePayloads(application.message_payloads(
                runtime_id,
                conversation_id,
                message_ids,
            )?),
            Command::SendTurn {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text,
            } => Response::SendTurn(application.send_turn(
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text,
            )?),
            Command::SendTurnWithSelection {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
            } => Response::SendTurnWithSelection(application.send_turn_with_selection(
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
            )?),
            Command::ReasoningEfforts {
                runtime_id,
                model_record_key,
            } => Response::ReasoningEfforts(
                application.reasoning_efforts(runtime_id, model_record_key)?,
            ),
            Command::SendQueue {
                runtime_id,
                conversation_id,
                operation,
            } => Response::SendQueue(application.send_queue(
                runtime_id,
                conversation_id,
                operation,
            )?),
            Command::SteerTurn {
                runtime_id,
                conversation_id,
                turn,
            } => Response::SteerTurn(application.steer_turn(runtime_id, conversation_id, turn)?),
            Command::EditMessage {
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn,
            } => Response::EditMessage(application.edit_message(
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn,
            )?),
            Command::Cancel {
                runtime_id,
                conversation_id,
            } => Response::Cancel(application.cancel(runtime_id, conversation_id)?),
            Command::StopSubAgents {
                runtime_id,
                conversation_id,
            } => Response::StopSubAgents(application.stop_sub_agents(runtime_id, conversation_id)?),
            Command::ConversationActivity { since } => {
                Response::ConversationActivity(application.conversation_activity(since)?)
            }
            Command::WorkingChanges {
                runtime_id,
                conversation_id,
            } => {
                Response::WorkingChanges(application.working_changes(runtime_id, conversation_id)?)
            }
            Command::WorkingBranch {
                runtime_id,
                conversation_id,
            } => Response::WorkingBranch(application.working_branch(runtime_id, conversation_id)?),
            Command::FileDiff {
                runtime_id,
                conversation_id,
                path,
            } => Response::FileDiff(application.file_diff(runtime_id, conversation_id, path)?),
            Command::PreviewProviderImport { gateway_id, source } => {
                Response::PreviewProviderImport(
                    application.preview_provider_import(gateway_id, source)?,
                )
            }
            Command::ApplyProviderImport {
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing,
            } => Response::ApplyProviderImport(application.apply_provider_import(
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing,
            )?),
            Command::ProviderQuota {
                gateway_id,
                provider_id,
            } => Response::ProviderQuota(application.provider_quota(gateway_id, provider_id)?),
            Command::RefreshProviderQuota {
                gateway_id,
                provider_id,
            } => Response::RefreshProviderQuota(
                application.refresh_provider_quota(gateway_id, provider_id)?,
            ),
            Command::SaveProvider {
                gateway_id,
                provider,
                authentication_edit,
            } => Response::SaveProvider(application.save_provider(
                gateway_id,
                provider,
                authentication_edit,
            )?),
            Command::DeleteProvider {
                gateway_id,
                provider_id,
            } => Response::DeleteProvider(application.delete_provider(gateway_id, provider_id)?),
            Command::SaveModelRoute { gateway_id, route } => {
                Response::SaveModelRoute(application.save_model_route(gateway_id, route)?)
            }
            Command::DeleteModelRoute {
                gateway_id,
                route_record_key,
            } => Response::DeleteModelRoute(
                application.delete_model_route(gateway_id, route_record_key)?,
            ),
            Command::ReadProviderApiKey {
                gateway_id,
                provider_id,
            } => Response::ReadProviderApiKey(
                application.read_provider_api_key(gateway_id, provider_id)?,
            ),
            Command::SaveModelTemplate { template } => {
                Response::SaveModelTemplate(application.save_model_template(template)?)
            }
            Command::DeleteModelTemplate { id } => {
                Response::DeleteModelTemplate(application.delete_model_template(id)?)
            }
            Command::ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                interaction_id,
                reply,
            } => Response::ReplyRuntimeInteraction(application.reply_runtime_interaction(
                runtime_id,
                conversation_id,
                interaction_id,
                reply,
            )?),
            Command::AuthenticationInspect {
                gateway_id,
                provider_id,
            } => Response::AuthenticationInspect(
                application.authentication_inspect(gateway_id, provider_id)?,
            ),
            Command::AuthenticationStart {
                gateway_id,
                provider_id,
            } => Response::AuthenticationStart(
                application.authentication_start(gateway_id, provider_id)?,
            ),
            Command::AuthenticationLogout {
                gateway_id,
                provider_id,
            } => Response::AuthenticationLogout(
                application.authentication_logout(gateway_id, provider_id)?,
            ),
            Command::AuthenticationPoll => {
                Response::AuthenticationPoll(application.authentication_poll()?)
            }
            Command::AuthenticationReply { prompt_id, value } => {
                Response::AuthenticationReply(application.authentication_reply(prompt_id, value)?)
            }
            Command::AuthenticationCancel => {
                Response::AuthenticationCancel(application.authentication_cancel()?)
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

    fn resolve_remote_attachments(
        &self,
        command: &mut Command,
        view: &ClientViewId,
    ) -> Result<(), Failure> {
        let resolve = |turn: &mut ComposedTurn| -> Result<(), Failure> {
            for reference in &mut turn.attachments {
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
        match command {
            Command::SendTurnWithSelection { turn, .. }
            | Command::SteerTurn { turn, .. }
            | Command::EditMessage { turn, .. } => resolve(turn)?,
            Command::SendQueue {
                operation: SendQueueOperation::Add { turn, .. },
                ..
            } => resolve(turn)?,
            _ => {}
        }
        Ok(())
    }

    fn attachment_content(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    ) -> Result<AttachmentContent, Error> {
        if !matches!(
            self.runtime_execution(runtime_id.clone())?.transport,
            RuntimeTransport::Local
        ) {
            return Err(Error::Unsupported(
                "SSH 执行位置的附件内容尚不支持读取".into(),
            ));
        }
        let messages =
            self.message_payloads(runtime_id, conversation_id, vec![message_id.clone()])?;
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
            | Response::CreateConversationWithSelection(result)
            | Response::OpenAcpConversation(result)
            | Response::OpenConversation(result)
            | Response::ReadConversation(result)
            | Response::Snapshot(result)
            | Response::SendTurn(result)
            | Response::SendTurnWithSelection(result)
            | Response::SendQueue(result)
            | Response::SteerTurn(result)
            | Response::EditMessage(result)
            | Response::Cancel(result)
            | Response::StopSubAgents(result)
            | Response::ReplyRuntimeInteraction(result) => result.snapshot.as_mut(),
            Response::SnapshotUpdate(SnapshotUpdate::Changed { snapshot }) => {
                Some(snapshot.as_mut())
            }
            _ => None,
        };
        if let Some(snapshot) = snapshots {
            let local = matches!(
                self.runtime_execution(snapshot.context_runtime_id.clone())
                    .map_err(Failure::from)?
                    .transport,
                RuntimeTransport::Local
            );
            for message in &mut snapshot.messages {
                encode_images(message, false, local);
            }
        }
        if let Response::MessagePayloads(messages) = &mut response {
            let local = match payload_runtime {
                Some(runtime) => matches!(
                    self.runtime_execution(runtime)
                        .map_err(Failure::from)?
                        .transport,
                    RuntimeTransport::Local
                ),
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
