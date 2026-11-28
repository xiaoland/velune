use super::*;
use crate::{ApplicationService, Options};

fn call(
    service: &ApplicationService,
    view: &str,
    operation: ComposerInputOperation,
) -> ComposerInputState {
    let command = Command::ManageComposerInputs { operation };
    let command = serde_json::from_slice(&serde_json::to_vec(&command).unwrap()).unwrap();
    match service
        .execute_checked(
            ClientViewId(view.into()),
            ExecutionContext {
                expected_revision: None,
            },
            command,
        )
        .unwrap()
        .response
    {
        Response::ManageComposerInputs(update) => update.state.unwrap(),
        _ => panic!("unexpected response"),
    }
}

#[test]
fn blank_tab_contexts_keep_independent_content_and_choices_after_promotion() {
    let home = std::env::temp_dir().join(format!("velune-tab-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    let mut store = ComposerInputs::open(&home).unwrap();
    let owner_id = store.owner_id();
    let contexts: Vec<_> = (0..2)
        .map(|_| ComposerContextKey::Conversation {
            owner_id: owner_id.clone(),
            conversation_id: format!("draft:{}", identity()),
        })
        .collect();
    let inputs = [identity(), identity()];
    for (index, context) in contexts.iter().enumerate() {
        store
            .operate(ComposerInputOperation::Ensure {
                context: context.clone(),
                input_id: inputs[index].clone(),
                seed: ComposerInputContent {
                    text: format!("tab {index}"),
                    attachments: vec![],
                },
            })
            .unwrap();
        store
            .operate(ComposerInputOperation::EnsureTarget {
                context: context.clone(),
                seed: ComposerTargetChoices {
                    runtime_id: Some(format!("runtime-{index}")),
                    runtime_owner_id: Some(owner_id.clone()),
                    choice_edited: true,
                    ..Default::default()
                },
            })
            .unwrap();
    }
    if let ComposerContextKey::Conversation {
        conversation_id, ..
    } = &contexts[1]
    {
        store
            .legacy_edit(conversation_id.clone(), Some("tab 1".into()), None)
            .unwrap();
        assert_eq!(store.legacy_input(conversation_id).unwrap().id, inputs[1]);
    }
    store
        .operate(ComposerInputOperation::Promote {
            input_id: inputs[0].clone(),
            owner_id: owner_id.clone(),
            conversation_id: "native-created".into(),
            runtime_id: "runtime-0".into(),
        })
        .unwrap();
    drop(store);
    let restored = ComposerInputs::open(&home).unwrap();
    for (index, context) in contexts.iter().enumerate() {
        let binding = restored
            .stored
            .state
            .bindings
            .iter()
            .find(|binding| &binding.context == context)
            .unwrap();
        assert_eq!(binding.input_id, inputs[index]);
        assert_eq!(
            restored
                .stored
                .state
                .inputs
                .iter()
                .find(|input| input.id == inputs[index])
                .unwrap()
                .content
                .text,
            format!("tab {index}")
        );
        assert_eq!(
            restored.target(context).unwrap().choices.runtime_id,
            Some(format!("runtime-{index}"))
        );
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn borrowed_input_promotes_without_consuming_new_and_legacy_edits_preserve_content() {
    let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    let mut store = ComposerInputs::open(&home).unwrap();
    let new_id = store.stored.state.bindings[0].input_id.clone();
    let attachment = store
        .upload("work.txt".into(), b"synthetic work".to_vec())
        .unwrap();
    let input_id = identity();
    let context = ComposerContextKey::Conversation {
        owner_id: store.owner_id(),
        conversation_id: "A".into(),
    };
    let content = ComposerInputContent {
        text: "borrowed X".into(),
        attachments: vec![attachment.clone()],
    };
    store
        .operate(ComposerInputOperation::Ensure {
            context: context.clone(),
            input_id: input_id.clone(),
            seed: content,
        })
        .unwrap();
    store
        .operate(ComposerInputOperation::EnsureTarget {
            context: context.clone(),
            seed: ComposerTargetChoices {
                runtime_id: Some("chosen-runtime".into()),
                runtime_owner_id: Some(store.owner_id()),
                choices: vec![
                    ComposerRuntimeChoice {
                        owner_id: store.owner_id(),
                        runtime_id: "same-runtime-id".into(),
                        choice: Default::default(),
                    },
                    ComposerRuntimeChoice {
                        owner_id: identity(),
                        runtime_id: "same-runtime-id".into(),
                        choice: Default::default(),
                    },
                ],
                choice_edited: true,
            },
        })
        .unwrap();
    store
        .legacy_edit("A".into(), Some("edited X".into()), None)
        .unwrap();
    let input = store.legacy_input("A").unwrap();
    assert_eq!(input.content.attachments, vec![attachment]);
    assert_eq!(
        store
            .stored
            .state
            .targets
            .iter()
            .find(|target| target.context == context)
            .unwrap()
            .choices
            .choices
            .len(),
        2,
        "same runtime ID on different execution hosts must remain distinct"
    );
    assert_eq!(
        store
            .stored
            .state
            .targets
            .iter()
            .find(|target| target.context == context)
            .unwrap()
            .choices
            .runtime_id
            .as_deref(),
        Some("chosen-runtime")
    );
    let state = store
        .operate(ComposerInputOperation::Promote {
            input_id: input_id.clone(),
            owner_id: store.owner_id(),
            conversation_id: "created-from-X".into(),
            runtime_id: "chosen-runtime".into(),
        })
        .unwrap();
    assert_eq!(
        state
            .bindings
            .iter()
            .find(|binding| binding.context == ComposerContextKey::NewConversation)
            .unwrap()
            .input_id,
        new_id
    );
    assert_eq!(
        state
            .bindings
            .iter()
            .filter(|binding| binding.input_id == input_id)
            .count(),
        2
    );
    let unchanged = store
        .update(ComposerInputOperation::Read {
            since_revision: Some(state.revision),
        })
        .unwrap();
    assert!(unchanged.state.is_none());
    assert_eq!(state.creations.len(), 1);
    let rebound = store
        .operate(ComposerInputOperation::Bind {
            context: ComposerContextKey::NewConversation,
            input_id: input_id.clone(),
            expected_revision: Some(
                state
                    .bindings
                    .iter()
                    .find(|binding| binding.context == ComposerContextKey::NewConversation)
                    .unwrap()
                    .revision,
            ),
        })
        .unwrap();
    let replayed = store
        .operate(ComposerInputOperation::Promote {
            input_id: input_id.clone(),
            owner_id: store.owner_id(),
            conversation_id: "created-from-X".into(),
            runtime_id: "chosen-runtime".into(),
        })
        .unwrap();
    assert_eq!(
        replayed, rebound,
        "a recovered creation must not consume a later new-conversation binding"
    );
    let stale = store.operate(ComposerInputOperation::Bind {
        context: ComposerContextKey::NewConversation,
        input_id: new_id.clone(),
        expected_revision: Some(1),
    });
    assert!(matches!(
        stale,
        Err(Error::Contract(Failure {
            kind: FailureKind::Conflict,
            ..
        }))
    ));
    let input = state
        .inputs
        .iter()
        .find(|input| input.id == input_id)
        .unwrap();
    store
        .operate(ComposerInputOperation::Save {
            input_id: input_id.clone(),
            expected_revision: Some(input.revision),
            content: Default::default(),
        })
        .unwrap();
    assert!(
        store
            .legacy_drafts()
            .iter()
            .filter(
                |draft| draft.conversation_id == "A" || draft.conversation_id == "created-from-X"
            )
            .all(|draft| draft.text.is_empty())
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn current_schema_unknown_nested_input_is_rejected_without_rewrite() {
    let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    ComposerInputs::open(&home).unwrap();
    let path = home.join("composer-inputs.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["state"]["inputs"][0]["content"]["future"] = serde_json::json!("retain");
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(ComposerInputs::open(&home).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn directory_and_attachments_outlive_views_and_native_promotion() {
    let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    let options = Options {
        home_directory: home.clone(),
        resources_directory: home.clone(),
    };
    let service = ApplicationService::open(options.clone()).unwrap();
    let first = call(
        &service,
        "desktop",
        ComposerInputOperation::Read {
            since_revision: None,
        },
    );
    let input_id = first.bindings[0].input_id.clone();
    let attachment = match service
        .execute_checked(
            ClientViewId("desktop".into()),
            ExecutionContext {
                expected_revision: None,
            },
            Command::UploadComposerAttachment {
                name: "pasted.png".into(),
                bytes: b"synthetic-image".to_vec(),
            },
        )
        .unwrap()
        .response
    {
        Response::UploadComposerAttachment(attachment) => attachment,
        _ => panic!("unexpected attachment response"),
    };
    let payload = ComposerInputContent {
        text: "UNSENT X".into(),
        attachments: vec![attachment.clone()],
    };
    call(
        &service,
        "desktop",
        ComposerInputOperation::SaveTarget {
            context: ComposerContextKey::NewConversation,
            expected_revision: Some(1),
            choices: ComposerTargetChoices {
                runtime_id: Some("runtime-a".into()),
                runtime_owner_id: Some(first.owner_id.clone()),
                choices: vec![ComposerRuntimeChoice {
                    owner_id: first.owner_id.clone(),
                    runtime_id: "runtime-a".into(),
                    choice: TurnChoice {
                        model: Some(ModelSelection::Native {
                            model_id: "model-a".into(),
                            provider_id: None,
                        }),
                        effort: Some("high".into()),
                        permission_mode: Some("allow-edits".into()),
                    },
                }],
                choice_edited: true,
            },
        },
    );
    let written = call(
        &service,
        "desktop",
        ComposerInputOperation::Save {
            input_id: input_id.clone(),
            expected_revision: Some(1),
            content: payload.clone(),
        },
    );
    let phone = call(
        &service,
        "phone",
        ComposerInputOperation::Read {
            since_revision: None,
        },
    );
    assert_eq!(
        written, phone,
        "the directory must not swap with ClientState"
    );
    service
        .execute(ClientViewId("desktop".into()), Command::ReleaseView)
        .unwrap();
    let reopened = call(
        &service,
        "another-view",
        ComposerInputOperation::Ensure {
            context: ComposerContextKey::NewConversation,
            input_id: identity(),
            seed: Default::default(),
        },
    );
    assert_eq!(reopened, phone, "opening new must not reset its state");
    let promoted = call(
        &service,
        "phone",
        ComposerInputOperation::Promote {
            input_id: input_id.clone(),
            owner_id: first.owner_id.clone(),
            conversation_id: "created-native-session".into(),
            runtime_id: "runtime-a".into(),
        },
    );
    assert_eq!(
        promoted
            .inputs
            .iter()
            .find(|input| input.id == input_id)
            .unwrap()
            .content,
        payload
    );
    let new_id = &promoted
        .bindings
        .iter()
        .find(|binding| binding.context == ComposerContextKey::NewConversation)
        .unwrap()
        .input_id;
    assert_ne!(new_id, &input_id);
    let next = &promoted
        .inputs
        .iter()
        .find(|input| &input.id == new_id)
        .unwrap()
        .content;
    assert!(next.text.is_empty() && next.attachments.is_empty());
    assert_eq!(
        promoted
            .targets
            .iter()
            .find(|target| target.context == ComposerContextKey::NewConversation)
            .unwrap()
            .choices
            .runtime_id
            .as_deref(),
        Some("runtime-a")
    );
    let bytes = match service
        .execute_checked(
            ClientViewId("another-view".into()),
            ExecutionContext {
                expected_revision: None,
            },
            Command::ReadComposerAttachment {
                attachment_id: attachment.id.clone(),
            },
        )
        .unwrap()
        .response
    {
        Response::ReadComposerAttachment(content) => content.bytes,
        _ => panic!("unexpected attachment content"),
    };
    assert_eq!(bytes, b"synthetic-image");
    service.close(false).unwrap();
    let service = ApplicationService::open(options).unwrap();
    assert_eq!(
        call(
            &service,
            "after-restart",
            ComposerInputOperation::Read {
                since_revision: None
            }
        ),
        promoted
    );
    let Response::ReadComposerAttachment(restored) = service
        .execute(
            ClientViewId("after-restart".into()),
            Command::ReadComposerAttachment {
                attachment_id: attachment.id,
            },
        )
        .unwrap()
    else {
        panic!("expected persisted attachment content")
    };
    assert_eq!(restored.attachment.name, "pasted.png");
    assert_eq!(restored.bytes, b"synthetic-image");
    service.close(false).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn content_and_target_choices_have_independent_revisions_and_stale_targets_cannot_win() {
    let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    let mut store = ComposerInputs::open(&home).unwrap();
    let id = store.stored.state.inputs[0].id.clone();
    let owner = store.owner_id();
    let target = ComposerContextKey::Conversation {
        owner_id: owner.clone(),
        conversation_id: "pi-target".into(),
    };
    let pi = ComposerTargetChoices {
        runtime_id: Some("pi".into()),
        runtime_owner_id: Some(owner),
        choice_edited: true,
        ..Default::default()
    };
    store
        .operate(ComposerInputOperation::SaveTarget {
            context: target.clone(),
            expected_revision: None,
            choices: pi.clone(),
        })
        .unwrap();
    store
        .operate(ComposerInputOperation::Save {
            input_id: id.clone(),
            expected_revision: Some(1),
            content: ComposerInputContent {
                text: "retry".into(),
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(
        store
            .stored
            .state
            .targets
            .iter()
            .find(|value| value.context == target)
            .unwrap()
            .revision,
        1
    );
    let mut changed = pi.clone();
    changed.choices.push(ComposerRuntimeChoice {
        owner_id: store.owner_id(),
        runtime_id: "pi".into(),
        choice: TurnChoice {
            effort: Some("high".into()),
            ..Default::default()
        },
    });
    store
        .operate(ComposerInputOperation::SaveTarget {
            context: target.clone(),
            expected_revision: Some(1),
            choices: changed.clone(),
        })
        .unwrap();
    assert!(
        store
            .operate(ComposerInputOperation::SaveTarget {
                context: target.clone(),
                expected_revision: Some(1),
                choices: pi
            })
            .is_err()
    );
    let content = store
        .stored
        .state
        .inputs
        .iter()
        .find(|input| input.id == id)
        .unwrap();
    assert_eq!(content.revision, 2);
    assert_eq!(content.content.text, "retry");
    assert_eq!(
        store
            .stored
            .state
            .targets
            .iter()
            .find(|value| value.context == target)
            .unwrap()
            .choices,
        changed
    );
    drop(store);
    let reopened = ComposerInputs::open(&home).unwrap();
    assert_eq!(
        reopened
            .stored
            .state
            .targets
            .iter()
            .find(|value| value.context == target)
            .unwrap()
            .choices
            .runtime_id
            .as_deref(),
        Some("pi")
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn stale_write_cannot_resurrect_consumed_input_or_attachment_path() {
    let home = std::env::temp_dir().join(format!("velune-composer-{}", identity()));
    fs::create_dir_all(&home).unwrap();
    let mut store = ComposerInputs::open(&home).unwrap();
    let state = store
        .operate(ComposerInputOperation::Read {
            since_revision: None,
        })
        .unwrap();
    let id = state.bindings[0].input_id.clone();
    let written = store
        .operate(ComposerInputOperation::Save {
            input_id: id.clone(),
            expected_revision: Some(1),
            content: ComposerInputContent {
                text: "X".into(),
                ..Default::default()
            },
        })
        .unwrap();
    let revision = written
        .inputs
        .iter()
        .find(|input| input.id == id)
        .unwrap()
        .revision;
    store
        .operate(ComposerInputOperation::Save {
            input_id: id.clone(),
            expected_revision: Some(revision),
            content: Default::default(),
        })
        .unwrap();
    let failure = store
        .operate(ComposerInputOperation::Save {
            input_id: id.clone(),
            expected_revision: Some(revision),
            content: ComposerInputContent {
                text: "STALE X".into(),
                ..Default::default()
            },
        })
        .unwrap_err();
    assert!(matches!(
        failure,
        Error::Contract(Failure {
            kind: FailureKind::Conflict,
            ..
        })
    ));
    assert!(
        store
            .operate(ComposerInputOperation::Read {
                since_revision: None
            })
            .unwrap()
            .inputs
            .iter()
            .find(|input| input.id == id)
            .unwrap()
            .content
            .text
            .is_empty()
    );
    assert!(store.upload("../escape.png".into(), vec![0]).is_err());
    fs::remove_dir_all(home).unwrap();
}
