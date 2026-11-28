use super::*;

#[test]
fn permission_projection_uses_effect_declared_by_agent() {
    let projected = |id: &'static str, kind| {
        let option = project_permission_option(wire::PermissionOption::new(id, id, kind));
        assert_eq!(option.id, id);
        option.approval_kind
    };
    assert_eq!(
        projected("allow", wire::PermissionOptionKind::AllowAlways),
        Some(ApprovalOptionKind::AllowAlways)
    );
    assert_eq!(
        projected("not-an-allow-id", wire::PermissionOptionKind::AllowOnce),
        Some(ApprovalOptionKind::AllowOnce)
    );
}

#[test]
fn rpc_error_keeps_code_message_and_native_data() {
    let error = rpc_error(&json!({"code":-32000,"message":"synthetic unavailable",
            "data":{"upstreamCode":"server_overloaded","requestId":"synthetic"}}))
    .unwrap();
    let text = error.to_string();
    for value in [
        "-32000",
        "synthetic unavailable",
        "server_overloaded",
        "requestId",
    ] {
        assert!(text.contains(value));
    }
    assert!(rpc_error(&json!({"code":"invalid","message":"bad"})).is_err());
}

pub(super) fn synthetic_client(mode: &str) -> (Client, velune_test_support::TempRoot) {
    let root = velune_test_support::TempRoot::new("acp-peer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let helper = root.write("peer.py", include_str!("tests/peer.py"));
    let client = Client::connect(Config {
        binary: "/usr/bin/python3".into(),
        args: vec![helper.to_string_lossy().into_owned(), mode.into()],
        environment: Default::default(),
        execution: ExecutionTarget::Local,
        os_home: Some(root.path().to_owned()),
        mcp_servers: vec![],
    })
    .unwrap();
    (client, root)
}

#[test]
fn elicitation_form_negotiates_and_routes_typed_reply_and_cancellation() {
    let (mut client, root) = synthetic_client("form");
    client.create(&root, "synthetic-acp").unwrap();
    for turn in 0..3 {
        client.send("question", &[]).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while client.snapshot().unwrap().pending_interactions.is_empty()
            && client.turn_running()
            && Instant::now() < deadline
        {
            client.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if turn != 1 {
            let interaction = client.snapshot().unwrap().pending_interactions[0].clone();
            assert!(matches!(
                interaction.kind,
                InteractionKind::UserInput { .. }
            ));
            if turn == 0 {
                client
                    .reply(
                        &interaction.id,
                        RuntimeInteractionReply::Answers {
                            answers: vec![InteractionAnswer {
                                question_id: "count".into(),
                                values: vec!["2".into()],
                            }],
                        },
                    )
                    .unwrap();
            } else {
                client.cancel().unwrap();
            }
        }
        while client.turn_running() && Instant::now() < deadline {
            client.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!client.turn_running());
        assert!(client.snapshot().unwrap().pending_interactions.is_empty());
    }
    client.shutdown().unwrap();
}

fn poll_until(client: &mut Client, ready: impl Fn(&Client) -> bool) -> Result<()> {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !ready(client) {
        assert!(
            Instant::now() < deadline,
            "ACP peer did not reach the expected state"
        );
        client.poll()?;
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
}

fn text_messages(snapshot: &ConversationSnapshot) -> Vec<&str> {
    snapshot
        .messages
        .iter()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| {
            if let MessageBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn create_load_and_resume_use_the_confirmed_history_and_returned_model() {
    let (mut client, home) = synthetic_client("history");
    let created = client.create(&home, "synthetic-acp").unwrap();
    assert_eq!(created.conversation.id, "synthetic-acp:synthetic");
    assert_eq!(text_messages(&created), ["first second"]);
    assert_eq!(client.current_model_id(), Some("model/a"));
    client.select_model("model/a").unwrap();
    assert_eq!(
        client.current_model_id(),
        Some("model/b"),
        "use the agent's reply, even when it differs from the requested model"
    );

    let loaded = client.load("loaded", &home, "synthetic-acp").unwrap();
    assert_eq!(loaded.conversation.id, "synthetic-acp:loaded");
    assert_eq!(text_messages(&loaded), ["authoritative replay"]);
    assert!(loaded.messages[0].completed);
    let resumed = client.resume("loaded", &home, "synthetic-acp").unwrap();
    assert_eq!(resumed.conversation.id, "synthetic-acp:loaded");
    assert!(text_messages(&resumed).is_empty());
    client.shutdown().unwrap();
}

#[test]
fn unsuccessful_open_never_publishes_a_provisional_identity_or_history() {
    for method in ["create", "load", "resume"] {
        let (mut client, home) = synthetic_client("reject-open");
        let (result, category, detail) = match method {
            "create" => (
                client.create(&home, "synthetic-acp"),
                "internal",
                "ACP 会话配置身份重复",
            ),
            "load" => (
                client.load("rejected", &home, "synthetic-acp"),
                "rpc",
                "not opened",
            ),
            "resume" => (
                client.resume("rejected", &home, "synthetic-acp"),
                "rpc",
                "not opened",
            ),
            _ => unreachable!(),
        };
        let error = result.expect_err("open must fail");
        assert_eq!(error.code(), category);
        assert!(error.to_string().contains(detail), "{method}: {error}");
        assert!(
            client.snapshot().is_none(),
            "{method} published a rejected session identity"
        );
        assert!(
            client
                .send("must not target the rejected session", &[])
                .unwrap_err()
                .to_string()
                .contains("连接已关闭")
        );
    }
}

#[test]
fn control_rejection_keeps_the_connection_and_turn_error_preserves_output() {
    let (mut client, home) = synthetic_client("reject");
    client.create(&home, "synthetic-acp").unwrap();
    let rejected = client.select_model("model/b").unwrap_err();
    assert_eq!(rejected.code(), "rpc");
    assert!(rejected.to_string().contains("synthetic rejected"));
    assert_eq!(client.current_model_id(), Some("model/a"));
    client.close_session().unwrap();

    let (mut client, home) = synthetic_client("prompt-error");
    client.create(&home, "synthetic-acp").unwrap();
    client.send("question", &[]).unwrap();
    poll_until(&mut client, |client| !client.turn_running()).unwrap();
    let snapshot = client.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert_eq!(text_messages(snapshot), ["question", "output before error"]);
    assert!(snapshot.messages.iter().flat_map(|m| &m.blocks).any(|b|
        matches!(b, MessageBlock::Notice {text} if text.contains("after execution") && text.contains("toolRan"))));
    client.close_session().unwrap();
}

#[test]
fn acknowledged_config_survives_disconnect_and_unconfirmed_interactions_expire() {
    let (mut client, home) = synthetic_client("accepted-disconnect");
    client.create(&home, "synthetic-acp").unwrap();
    client.select_model("model/a").unwrap();
    assert_eq!(client.current_model_id(), Some("model/b"));
    assert_eq!(
        poll_until(&mut client, |_| false).unwrap_err().code(),
        "transport"
    );
    assert_eq!(client.current_model_id(), Some("model/b"));
    assert!(!client.snapshot().unwrap().actions.can_send);

    let (mut client, home) = synthetic_client("disconnect");
    client.create(&home, "synthetic-acp").unwrap();
    client.send("question", &[]).unwrap();
    poll_until(&mut client, |client| {
        !client.snapshot().unwrap().pending_interactions.is_empty()
    })
    .unwrap();
    let id = client.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    // End this private peer with an answerable form still pending.
    home.touch("disconnect-now");
    assert_eq!(
        poll_until(&mut client, |_| false).unwrap_err().code(),
        "transport"
    );
    let snapshot = client.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(snapshot.pending_interactions.is_empty());
    assert_eq!(
        text_messages(snapshot),
        ["question", "output before disconnect"]
    );
    assert!(
        client
            .reply(&id, RuntimeInteractionReply::Cancel)
            .unwrap_err()
            .to_string()
            .contains("请求已失效")
    );
    client.poll().unwrap();
    assert!(!client.snapshot().unwrap().actions.can_send);
}

#[test]
fn live_permission_form_and_cancel_reach_their_own_peer_in_order() {
    let (mut client, home) = synthetic_client("live");
    client.create(&home, "synthetic-acp").unwrap();
    client.send("question", &[]).unwrap();
    poll_until(&mut client, |client| {
        client.snapshot().unwrap().pending_interactions.len() == 2
    })
    .unwrap();
    let pending = client.snapshot().unwrap().pending_interactions.clone();
    let (mut next, next_home) = synthetic_client("live");
    next.create(&next_home, "synthetic-acp").unwrap();
    next.send("question", &[]).unwrap();
    poll_until(&mut next, |c| {
        c.snapshot().unwrap().pending_interactions.len() == 2
    })
    .unwrap();
    // Both peers use wire IDs p and q. A UI reply from the other connection must have no effect.
    assert!(
        next.reply(&pending[0].id, RuntimeInteractionReply::Cancel)
            .unwrap_err()
            .to_string()
            .contains("请求已失效")
    );
    assert_eq!(next.snapshot().unwrap().pending_interactions.len(), 2);
    next.shutdown().unwrap();

    for entry in pending {
        let reply = match entry.kind {
            InteractionKind::Approval { .. } => RuntimeInteractionReply::Decision {
                option_id: "yes".into(),
            },
            InteractionKind::UserInput { .. } => RuntimeInteractionReply::Answers {
                answers: vec![InteractionAnswer {
                    question_id: "value".into(),
                    values: vec!["answer".into()],
                }],
            },
            other => panic!("unexpected interaction: {other:?}"),
        };
        client.reply(&entry.id, reply).unwrap();
    }
    assert!(client.turn_running());
    client.cancel().unwrap();
    poll_until(&mut client, |client| !client.turn_running()).unwrap();
    let snapshot = client.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Idle);
    assert!(snapshot.pending_interactions.is_empty());
    assert_eq!(
        text_messages(snapshot),
        ["question", "p deliveredq deliveredcancel delivered"]
    );
}

#[test]
fn same_wire_request_id_cannot_identify_both_a_permission_and_a_form() {
    for mode in ["duplicate-permission-first", "duplicate-elicitation-first"] {
        let (mut client, home) = synthetic_client(mode);
        client.create(&home, "synthetic-acp").unwrap();
        client.send("question", &[]).unwrap();
        let error = poll_until(&mut client, |_| false).unwrap_err();
        assert!(
            error.to_string().contains("ACP Agent 请求身份重复"),
            "{mode}: {error}"
        );
        let snapshot = client.snapshot().unwrap();
        assert!(snapshot.pending_interactions.is_empty());
        assert!(!snapshot.actions.can_send);
        assert!(!home.exists("duplicate-reply"));
    }
}

#[test]
fn idle_form_remains_answerable_after_the_control_timeout() {
    let (mut client, home) = synthetic_client("idle-form");
    client.create(&home, "synthetic-acp").unwrap();
    poll_until(&mut client, |c| {
        !c.snapshot().unwrap().pending_interactions.is_empty()
    })
    .unwrap();
    let id = client.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    // Exercise the elapsed timeout that used to expire a human reply.
    std::thread::sleep(CONTROL_TIMEOUT + std::time::Duration::from_millis(100));
    client.poll().unwrap();
    client
        .reply(
            &id,
            RuntimeInteractionReply::Answers {
                answers: vec![InteractionAnswer {
                    question_id: "value".into(),
                    values: vec!["still here".into()],
                }],
            },
        )
        .unwrap();
    poll_until(&mut client, |c| {
        text_messages(c.snapshot().unwrap()) == ["idle answer delivered"]
    })
    .unwrap();
    assert!(client.snapshot().unwrap().pending_interactions.is_empty());
    assert_eq!(home.read("reply-count"), "1");
    client.shutdown().unwrap();
}
