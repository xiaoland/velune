//! The logical source and the current projection are two addresses of one view.
use super::*;

pub(super) struct ConversationAddress {
    pub source_runtime_id: String,
    pub context_runtime_id: String,
}

impl CoreRuntime {
    /// Browsing may name either the logical source (list/composer) or the tail
    /// context (snapshot follow). Neither may silently become the send target.
    pub(super) fn conversation_read_address(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationAddress, RuntimeError> {
        resolve(&self.conversation_links, runtime_id, id, false)
    }

    pub(super) fn conversation_source_address(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationAddress, RuntimeError> {
        resolve(&self.conversation_links, runtime_id, id, true)
    }
}

fn resolve(
    links: &[crate::config::ConversationLink],
    runtime_id: &str,
    id: &str,
    source_only: bool,
) -> Result<ConversationAddress, RuntimeError> {
    let Some(link) = links.iter().find(|link| link.id == id) else {
        return Ok(ConversationAddress {
            source_runtime_id: runtime_id.into(),
            context_runtime_id: runtime_id.into(),
        });
    };
    let source = link
        .segments
        .first()
        .ok_or_else(|| RuntimeError::invalid("关联会话缺少来源"))?;
    let context = link.segments.last().expect("nonempty conversation link");
    if runtime_id != source.runtime_instance_id
        && (source_only || runtime_id != context.runtime_instance_id)
    {
        return Err(RuntimeError::invalid("会话来源运行时不匹配"));
    }
    Ok(ConversationAddress {
        source_runtime_id: source.runtime_instance_id.clone(),
        context_runtime_id: context.runtime_instance_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn cold_tail_read_and_composer_switch_preserve_the_logical_source() {
        use super::super::conversation_history::ConversationRead;
        use sha2::{Digest, Sha256};
        use velune_application_api::{ComposerTargetChoices, ComposerTargetRequest};
        let root = std::env::temp_dir().join(format!(
            "velune-logical-address-{}",
            crate::new_record_key()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.join("app"),
            resources_directory: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../agent-runtime/resources"),
        })
        .unwrap();
        let core = &mut app.runtime;
        let gateway_id = "synthetic".to_owned();
        core.gateways.push(GatewayConfig {
            id: gateway_id.clone(),
            name: "Synthetic".into(),
            providers: Vec::new(),
            routes: Vec::new(),
        });
        let context = serde_json::from_value(serde_json::json!({"messages": [
            {"role":"user","content":[{"type":"text","text":"synthetic question"}]},
            {"role":"assistant","content":[{"type":"text","text":"synthetic reply"}]}
        ]}))
        .unwrap();
        let mut segments = Vec::new();
        for runtime_id in ["source", "tail", "next"] {
            let agent_dir = root.join(runtime_id);
            std::fs::create_dir(&agent_dir).unwrap();
            core.runtime_instances.push(RuntimeInstance {
                enabled: true,
                id: runtime_id.into(),
                name: runtime_id.into(),
                type_id: RuntimeKind::ClaudeCode,
                gateway_id: gateway_id.clone(),
                settings: BTreeMap::from([
                    ("binary".into(), "/synthetic/not-executed".into()),
                    ("agentDir".into(), agent_dir.to_string_lossy().into_owned()),
                ]),
            });
            if runtime_id == "next" {
                continue;
            }
            let imported = velune_agent_runtime::import::import_claude_conversation(
                &velune_agent_runtime::execution::ExecutionTarget::Local,
                &agent_dir,
                &root,
                &context,
                "2.1.286",
            )
            .unwrap();
            let velune_agent_runtime::import::ImportedConversation::ClaudeCode { session_id } =
                imported
            else {
                panic!("synthetic Claude import")
            };
            let id = format!("{runtime_id}:{session_id}");
            let snapshot = core.read_history(runtime_id, &id).unwrap();
            let content: Vec<_> = snapshot
                .messages
                .iter()
                .map(|message| (&message.role, &message.blocks))
                .collect();
            let prefix_digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&content).unwrap())
            );
            segments.push(
                serde_json::from_value(serde_json::json!({
                    "runtimeInstanceId": runtime_id,
                    "runtimeTypeId": "claude-code-2.1",
                    "nativeConversationId": id,
                    "cwd": root.to_string_lossy(),
                    "cutoff": (runtime_id == "source").then(|| crate::config::ConversationCutoff {
                        message_count: snapshot.messages.len() as u64,
                        prefix_digest,
                    }),
                    "handoff": null,
                    "imported": null,
                    "deleted": false,
                }))
                .unwrap(),
            );
        }
        let id = "logical:synthetic";
        core.conversation_links
            .push(crate::config::ConversationLink {
                id: id.into(),
                segments,
            });
        // Repeat after eviction: a context-addressed cold load must not depend
        // on a previously loaded source-addressed projection.
        for _ in 0..2 {
            let ConversationRead::Load(read) = core.prepare_conversation_read("tail", id).unwrap()
            else {
                panic!("expected cold history read")
            };
            let snapshot = core
                .finish_conversation_read(read.load())
                .unwrap()
                .snapshot
                .unwrap();
            assert_eq!(snapshot.conversation.runtime_id, "source");
            assert_eq!(snapshot.context_runtime_id, "tail");
            assert_eq!(snapshot.conversation.id, id);
            core.focus_conversation(id);
            assert!(
                core.read_present_conversation("source", id)
                    .unwrap()
                    .is_some()
            );
            assert!(
                core.read_present_conversation("tail", id)
                    .unwrap()
                    .is_some()
            );
            let request = ComposerTargetRequest {
                runtime_id: "source".into(),
                conversation_id: Some(id.into()),
            };
            let basis = core
                .composer_target_basis(
                    request.clone(),
                    "owner".into(),
                    Some(ComposerTargetChoices {
                        runtime_id: Some("next".into()),
                        choice_edited: true,
                        ..Default::default()
                    }),
                )
                .unwrap();
            assert_eq!(basis.request, request);
            assert_eq!(basis.default_runtime_id, "tail");
            assert_eq!(basis.input.runtime_id, "next");
            assert_eq!(basis.input.conversation_runtime_id.as_deref(), Some("tail"));
            assert!(
                core.composer_target_identity(
                    ComposerTargetRequest {
                        runtime_id: "tail".into(),
                        conversation_id: Some(id.into())
                    },
                    "owner".into()
                )
                .is_err()
            );
            assert!(core.prepare_conversation_read("next", id).is_err());
            core.session = Default::default();
            core.parked.clear();
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn logical_reads_accept_source_and_current_context_but_writes_keep_source_identity() {
        let segment = |runtime: &str| {
            serde_json::from_value(serde_json::json!({
                "runtimeInstanceId": runtime,
                "runtimeTypeId": "codex-0.166",
                "nativeConversationId": format!("{runtime}:native"),
                "cwd": null, "cutoff": null, "handoff": null,
                "imported": null, "deleted": false,
            }))
            .unwrap()
        };
        let link = crate::config::ConversationLink {
            id: "logical:synthetic".into(),
            segments: vec![segment("source"), segment("middle"), segment("tail")],
        };
        for runtime in ["source", "tail"] {
            let address = resolve(std::slice::from_ref(&link), runtime, &link.id, false).unwrap();
            assert_eq!(address.source_runtime_id, "source");
            assert_eq!(address.context_runtime_id, "tail");
        }
        assert!(resolve(std::slice::from_ref(&link), "tail", &link.id, true).is_err());
        assert!(resolve(std::slice::from_ref(&link), "middle", &link.id, false).is_err());
        assert!(resolve(std::slice::from_ref(&link), "unrelated", &link.id, false).is_err());
        assert!(resolve(std::slice::from_ref(&link), "source", &link.id, true).is_ok());
    }
}
