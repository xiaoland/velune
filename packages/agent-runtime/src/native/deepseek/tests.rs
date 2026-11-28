//! Synthetic child protocols exercise the same NativeSession entry points as application.
use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    session: NativeSession,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::current_dir().unwrap().join(format!(
            "tmp/deepseek-normal-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("dsh.py");
        let script = include_str!("fixture.py")
            .replace("MODE = None", &format!("MODE = {mode:?}"))
            .replace(
                "ROOT = None",
                &format!("ROOT = {:?}", root.to_string_lossy()),
            );
        std::fs::write(&binary, script).unwrap();
        let session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::DeepSeek,
            binary,
            node_binary: Some(PathBuf::from("/usr/bin/python3")),
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        Self { root, session }
    }
    fn establish(&mut self) {
        self.session.create(Some(&self.root), "runtime").unwrap();
    }
    fn until(&mut self, predicate: impl Fn(&NativeSession) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(&self.session) {
            assert!(
                Instant::now() < deadline,
                "synthetic DeepSeek condition timed out"
            );
            self.session.poll().unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn wire(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("wire.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn await_wire(&mut self, predicate: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let wire = self.wire();
            if predicate(&wire) {
                return wire;
            }
            assert!(
                Instant::now() < deadline,
                "synthetic DeepSeek wire acknowledgement timed out"
            );
            self.session.poll().unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn input(text: &str) -> crate::TurnInput {
    crate::TurnInput {
        text: text.into(),
        effort: None,
        permission_mode: None,
        images: vec![],
    }
}
fn text_seen(session: &NativeSession, expected: &str) -> bool {
    session.snapshot().is_some_and(|snapshot| {
        snapshot.messages.iter().any(|message| {
            message
                .blocks
                .iter()
                .any(|block| matches!(block, MessageBlock::Text { text } | MessageBlock::Notice { text } if text == expected))
        })
    })
}

#[test]
fn deepseek_normal_prompt_streams_and_replies_before_its_result() {
    for mode in ["late", "turn-error"] {
        let mut f = Fixture::new(mode);
        f.establish();
        f.session.send(&input("Only once")).unwrap();
        f.until(|session| session.has_pending_requests());
        assert!(f.session.turn_running());
        assert!(text_seen(&f.session, "Streaming"));
        let interaction = f.session.snapshot().unwrap().pending_interactions[0]
            .id
            .clone();
        let invalid = f
            .session
            .reply(
                &interaction,
                RuntimeInteractionReply::Decision {
                    option_id: "not-advertised".into(),
                },
            )
            .unwrap_err();
        assert!(invalid.to_string().contains("此权限请求不接受该回复"));
        assert!(f.session.has_pending_requests());
        f.session
            .reply(
                &interaction,
                RuntimeInteractionReply::Decision {
                    option_id: "once".into(),
                },
            )
            .unwrap();
        f.until(|session| !session.turn_running());
        let wire = f.await_wire(|wire| {
            wire.iter()
                .any(|r| r["id"] == "permission" && r.get("result").is_some())
        });
        assert_eq!(
            wire.iter()
                .filter(|r| r["method"] == "session/prompt")
                .count(),
            1
        );
        assert_eq!(
            wire.iter()
                .find(|r| r["id"] == "permission" && r.get("result").is_some())
                .unwrap()["result"],
            json!({"outcome":{"outcome":"selected","optionId":"once"}})
        );
        assert_eq!(
            wire.iter().find(|r| r["id"] == "wrong-permission").unwrap()["result"],
            json!({"outcome":{"outcome":"cancelled"}})
        );
        assert_eq!(
            wire.iter().find(|r| r["id"] == "unsupported").unwrap()["error"]["code"],
            -32601
        );
        assert!(
            f.session
                .snapshot()
                .unwrap()
                .pending_interactions
                .is_empty()
        );
        assert_eq!(
            f.session.snapshot().unwrap().run_state,
            if mode == "turn-error" {
                RunState::Failed
            } else {
                RunState::Idle
            }
        );
        if mode == "turn-error" {
            assert!(f.session.snapshot().unwrap().messages.iter().flat_map(|m| &m.blocks)
                .any(|b| matches!(b, MessageBlock::Notice { text } if text.contains("Late tool failure"))));
            assert!(
                !f.session.failed,
                "native turn rejection is not a transport disconnect"
            );
        }
    }
}

#[test]
fn deepseek_normal_cancel_is_written_for_the_active_prompt() {
    let mut f = Fixture::new("cancel");
    f.establish();
    f.session.send(&input("Wait for cancellation")).unwrap();
    f.until(|session| text_seen(session, "Streaming"));
    assert!(f.session.turn_running());
    f.session.cancel().unwrap();
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Stopping);
    f.until(|session| !session.turn_running());
    let wire = f.wire();
    let prompt = wire
        .iter()
        .position(|r| r["method"] == "session/prompt")
        .unwrap();
    let cancel = wire
        .iter()
        .position(|r| r["method"] == "session/cancel")
        .unwrap();
    assert!(prompt < cancel);
    assert_eq!(wire[cancel]["params"], json!({"sessionId":"session"}));
    assert!(wire[cancel].get("id").is_none(), "ACP cancel is one-way");
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Idle);
    assert!(text_seen(&f.session, "DeepSeek 本轮已取消。"));
}

#[test]
fn deepseek_normal_create_routes_early_records_only_after_confirmed_identity() {
    let mut f = Fixture::new("early");
    f.establish();
    f.until(|session| session.has_pending_requests());
    assert_eq!(f.session.native_id.as_deref(), Some("session"));
    assert!(text_seen(&f.session, "Before identity response"));
    assert!(!text_seen(&f.session, "Wrong session must stay invisible"));
    assert_eq!(f.session.snapshot().unwrap().pending_interactions.len(), 1);
    let interaction = f.session.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    f.session
        .reply(&interaction, RuntimeInteractionReply::Cancel)
        .unwrap();
    let wire = f.await_wire(|wire| {
        wire.iter()
            .any(|r| r["id"] == "early-session" && r.get("result").is_some())
    });
    for id in ["early-wrong", "early-session"] {
        assert_eq!(
            wire.iter().find(|r| r["id"] == id).unwrap()["result"],
            json!({"outcome":{"outcome":"cancelled"}})
        );
    }
    let created = wire.iter().find(|r| r["method"] == "session/new").unwrap();
    assert_eq!(created["params"]["cwd"], f.root.to_string_lossy().as_ref());
    assert_eq!(created["params"]["mcpServers"], json!([]));
}

#[test]
fn deepseek_normal_rejected_creation_has_no_active_identity_or_replay() {
    let mut f = Fixture::new("reject");
    let error = f.session.create(Some(&f.root), "runtime").err().unwrap();
    assert!(error.to_string().contains("Synthetic rejection"), "{error}");
    assert!(f.session.native_id.is_none());
    assert!(f.session.snapshot().is_none());
    let error = f.session.send(&input("must not reach native")).unwrap_err();
    assert!(error.to_string().contains("没有活动会话"), "{error}");
    let wire = f.wire();
    assert_eq!(
        wire.iter().filter(|r| r["method"] == "session/new").count(),
        1
    );
    assert!(!wire.iter().any(|r| r["method"] == "session/prompt"));
}

#[test]
fn deepseek_normal_resume_preserves_history_and_rejection_cannot_send() {
    for mode in ["early", "resume-reject"] {
        let mut f = Fixture::new(mode);
        let history = vec![Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: "saved-history".into(),
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text {
                text: "Caller history".into(),
            }],
        }];
        let result = f.session.open(
            "session",
            Some(&f.root),
            "runtime",
            Retained::from_parts(history.clone(), DataRetention::default()),
        );
        if mode == "resume-reject" {
            assert!(
                result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("Synthetic rejection")
            );
            assert!(f.session.native_id.is_none());
            assert!(
                f.session
                    .send(&input("must not reach native"))
                    .unwrap_err()
                    .to_string()
                    .contains("没有活动会话")
            );
            f.session.poll().unwrap();
            let wire = f.await_wire(|wire| {
                wire.iter()
                    .any(|r| r["id"] == "early-session" && r.get("result").is_some())
            });
            assert_eq!(
                wire.iter().find(|r| r["id"] == "early-session").unwrap()["result"],
                json!({"outcome":{"outcome":"cancelled"}})
            );
            assert!(!wire.iter().any(|r| r["method"] == "session/prompt"));
            assert!(f.session.snapshot().is_none());
        } else {
            assert_eq!(result.unwrap().messages, history);
            f.until(|session| session.has_pending_requests());
            assert!(text_seen(&f.session, "Caller history"));
            assert!(text_seen(&f.session, "Before identity response"));
            assert!(!text_seen(&f.session, "Wrong session must stay invisible"));
        }
    }
}

#[test]
fn deepseek_normal_model_selection_roundtrips_the_advertised_opaque_value() {
    let mut f = Fixture::new("normal");
    f.session
        .select_native_model(Retained::from_parts(
            (Some("provider".into()), "other".into()),
            DataRetention::default(),
        ))
        .unwrap();
    f.establish();
    assert_eq!(
        f.session.dsh_current_model(),
        Some(("provider".into(), "other".into()))
    );
    f.session.send(&input("selected model")).unwrap();
    f.until(|session| !session.turn_running());
    let wire = f.wire();
    let setter = wire
        .iter()
        .find(|r| r["method"] == "session/set_config_option")
        .unwrap();
    assert_eq!(
        setter["params"],
        json!({"sessionId":"session","configId":"model","value":"[ \"provider\", \"other\" ]"})
    );
}

#[test]
fn deepseek_normal_send_preserves_a_legal_prompt_larger_than_eight_mib() {
    let mut f = Fixture::new("normal");
    f.establish();
    let text = "x".repeat(8 * 1024 * 1024 + 1);
    f.session.send(&input(&text)).unwrap();
    f.until(|session| !session.turn_running());
    let wire = f.wire();
    let prompts = wire
        .iter()
        .filter(|r| r["method"] == "session/prompt")
        .collect::<Vec<_>>();
    assert_eq!(prompts.len(), 1);
    assert_eq!(
        prompts[0]["params"]["prompt"],
        json!([{"type":"text","text":text}])
    );
}

#[test]
fn deepseek_connect_validates_protocol_resume_and_required_http_not_plugin_release() {
    let f = Fixture::new("normal");
    // connect prepares optional MCP endpoints before the ACP handshake. Port
    // zero is rejected there and silently omitted, so it cannot make HTTP
    // capability mandatory. Hold a real private endpoint for this matrix.
    let mcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", mcp.local_addr().unwrap());
    for (protocol, resume, http, require_http, error) in [
        (json!(2), json!({}), true, false, Some("协议或恢复能力")),
        (json!(1), Value::Null, true, false, Some("协议或恢复能力")),
        (json!(1), json!(false), true, false, Some("协议或恢复能力")),
        (
            json!(1),
            json!({}),
            false,
            true,
            Some("未公布 HTTP MCP 能力"),
        ),
        (json!(1), json!({}), true, true, None),
    ] {
        let handshake = json!({"protocolVersion":protocol,
            "agentCapabilities":{"sessionCapabilities":{"resume":resume},"mcpCapabilities":{"http":http}},
            "agentInfo":{"name":"synthetic-plugin","version":"999.0.0"}});
        std::fs::write(f.root.join("handshake.json"), handshake.to_string()).unwrap();
        let mut config = f.session.config.clone();
        config.mcp_servers = if require_http {
            vec![crate::mcp::Server {
                name: "synthetic".into(),
                endpoint: endpoint.clone(),
                token: "synthetic".into(),
            }]
        } else {
            vec![]
        };
        match (NativeSession::connect(config), error) {
            (Ok(mut session), None) => {
                assert_eq!(session.config.mcp_servers.len(), usize::from(require_http));
                if require_http {
                    assert_eq!(session.config.mcp_servers[0].endpoint, endpoint);
                }
                session.shutdown().unwrap();
            }
            (Err(actual), Some(expected)) => {
                assert!(actual.to_string().contains(expected), "{actual}")
            }
            (Ok(mut session), Some(expected)) => {
                session.shutdown().unwrap();
                panic!("accepted invalid handshake: {expected}");
            }
            (Err(actual), None) => panic!("rejected valid handshake: {actual}"),
        }
    }
}

#[test]
fn deepseek_overlay_preserves_model_metadata_disables_old_endpoint_and_gates_handoff() {
    let mut f = Fixture::new("normal");
    f.establish();
    let target = crate::control::Target {
        execution: f.session.config.execution.clone(),
        os_home: f.session.config.os_home.clone(),
        kind: crate::version::RuntimeKind::DeepSeek,
        agent_dir: f.root.clone(),
        native_id: "session".into(),
        node_binary: PathBuf::from("/usr/bin/python3"),
        resources: f.root.clone(),
    };
    assert!(f.session.can_handoff_control(Some(&target)));
    let mut config = f.session.config.clone();
    config.gateway = Some(GatewayInjection {
        endpoint: "http://127.0.0.1:12345".into(),
        token: "synthetic-token".into(),
        model_alias: "one".into(),
        protocol: "openai-completions".into(),
        context_window: Some(10000),
        max_output_tokens: None,
        reasoning_levels: None,
    });
    assert!(NativeSession::dsh_command(&config).unwrap().1);
    config.gateway.as_mut().unwrap().model_alias = "two".into();
    NativeSession::dsh_command(&config).unwrap();
    config.gateway = None;
    assert!(NativeSession::dsh_command(&config).unwrap().1);
    let native: Value = serde_json::from_slice(
        &std::fs::read(f.root.join("deepseek-native-history.patch.yml")).unwrap(),
    )
    .unwrap();
    let provider = &native[0]["config"]["providers"]["velune-gateway"];
    assert_eq!(provider["baseURL"], "http://127.0.0.1:0");
    assert_eq!(provider["models"].as_array().unwrap().len(), 2);
    assert_eq!(provider["models"][0]["id"], "one");
    assert_eq!(provider["models"][1]["id"], "two");
    assert!(provider["models"][0].get("maxTokens").is_none());
    f.session.dsh_history_overlay = true;
    assert!(!f.session.can_handoff_control(Some(&target)));
}

#[test]
fn deepseek_auto_compaction_applies_on_same_session_resume_and_clear_restores_profile() {
    use velune_conversation::AutoCompactionSetting;

    let mut f = Fixture::new("normal");
    std::fs::write(
        f.root.join("composed.yml"),
        r#"- id: other
  config: {secret: synthetic-unrelated}
- id: agent
  group: true
  config:
  - id: compaction-basic
    name: '@deepseek-ai/dsh-compaction-basic'
    config:
      retainRatio: 0.22
      maxTokens: 3000
      headroomTokens: !!js '1024 + 1024'
      modelPolicies:
      - {provider: provider, model: model, thresholdRatio: 0.63, retainTokens: 1024}
      - {provider: provider, model: sibling, thresholdRatio: 0.58}
"#,
    )
    .unwrap();
    f.session.create(Some(&f.root), "runtime").unwrap();
    assert_eq!(
        f.session.dsh_current_model(),
        Some(("provider".into(), "model".into()))
    );
    f.session
        .set_auto_compaction(Some(AutoCompactionSetting::DeepSeek {
            threshold_percent: 74,
        }));
    f.session
        .send(&crate::TurnInput {
            text: "synthetic".into(),
            effort: None,
            permission_mode: None,
            images: Vec::new(),
        })
        .unwrap();
    f.until(|session| !session.turn_running());
    f.session.set_auto_compaction(None);
    f.session
        .send(&crate::TurnInput {
            text: "clear".into(),
            effort: None,
            permission_mode: None,
            images: Vec::new(),
        })
        .unwrap();
    f.until(|session| !session.turn_running());
    let wire = f.wire();
    let patch = wire
        .iter()
        .find_map(|record| record["compactionPatch"].as_str())
        .unwrap();
    assert!(patch.contains("!!js"));
    assert!(!patch.contains("synthetic-unrelated"));
    let patch: serde_yaml_ng::Value = serde_yaml_ng::from_str(patch).unwrap();
    let config = &patch[0]["config"];
    assert_eq!(config["retainRatio"].as_f64(), Some(0.22));
    assert_eq!(config["maxTokens"].as_u64(), Some(3000));
    assert_eq!(config["headroomTokens"].as_str(), Some("1024 + 1024"));
    let policies = config["modelPolicies"].as_sequence().unwrap();
    assert_eq!(policies.len(), 2);
    assert_eq!(policies[0]["thresholdRatio"].as_f64(), Some(0.74));
    assert_eq!(policies[0]["retainTokens"].as_u64(), Some(1024));
    assert_eq!(policies[1]["thresholdRatio"].as_f64(), Some(0.58));
    let resumes = wire
        .iter()
        .filter(|record| record["method"] == "session/resume")
        .map(|record| record["params"]["sessionId"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(resumes, ["session", "session"]);
    assert_eq!(
        wire.iter()
            .filter(|record| record.get("configDump").is_some())
            .count(),
        1
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record.get("compactionPatch").is_some())
            .count(),
        1
    );
    assert!(std::fs::read_dir(&f.root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("dsh-compaction-")
    }));
}

#[test]
fn deepseek_compaction_config_uses_gateway_route_identity() {
    use velune_conversation::AutoCompactionSetting;

    let f = Fixture::new("normal");
    let mut config = f.session.config.clone();
    std::fs::write(
        f.root.join("composed.yml"),
        "- id: compaction-basic\n  name: '@deepseek-ai/dsh-compaction-basic'\n",
    )
    .unwrap();
    config.gateway = Some(GatewayInjection {
        endpoint: "http://127.0.0.1:12345".into(),
        token: "synthetic-token".into(),
        model_alias: "route-alias".into(),
        protocol: "openai-completions".into(),
        context_window: Some(180_000),
        max_output_tokens: None,
        reasoning_levels: None,
    });
    config.auto_compaction = Some(AutoCompactionSetting::DeepSeek {
        threshold_percent: 16,
    });
    let conflict = NativeSession::dsh_spawn(&config, None)
        .err()
        .expect("the threshold must exceed the native default retained fraction");
    assert!(conflict.to_string().contains("原生近期上下文保留比例"));
    config.auto_compaction = Some(AutoCompactionSetting::DeepSeek {
        threshold_percent: 85,
    });
    let (mut rpc, overlay) = NativeSession::dsh_spawn(&config, None).unwrap();
    assert!(overlay);
    rpc.finish();
    let launch = f
        .wire()
        .into_iter()
        .find(|record| record.get("compactionPatch").is_some())
        .unwrap();
    let patch: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(launch["compactionPatch"].as_str().unwrap()).unwrap();
    let policy = &patch[0]["config"]["modelPolicies"][0];
    assert_eq!(policy["provider"].as_str(), Some("velune-gateway"));
    assert_eq!(policy["model"].as_str(), Some("route-alias"));
    assert_eq!(policy["thresholdRatio"].as_f64(), Some(0.85));
}

#[test]
fn deepseek_blocking_catalog_keeps_private_persistence_lifecycle_and_removes_failure_artifacts() {
    for mode in ["normal", "reject"] {
        let mut f = Fixture::new(mode);
        let directory = f.root.join(format!("dsh-catalog-{}", f.session.epoch));
        let result = f.session.models();
        match result {
            Ok(models) => {
                assert_eq!(mode, "normal");
                assert_eq!(
                    models
                        .iter()
                        .map(|model| (model.model.id.as_str(), model.model.provider_id.as_deref()))
                        .collect::<Vec<_>>(),
                    vec![("model", Some("provider")), ("other", Some("provider"))]
                );
            }
            Err(error) => {
                assert_eq!(mode, "reject");
                assert!(error.to_string().contains("Synthetic rejection"), "{error}");
            }
        }
        assert!(!directory.exists());
        assert!(f.session.native_id.is_none());
        let wire = f.wire();
        let launch = wire
            .iter()
            .find(|record| record.get("patch").is_some())
            .unwrap();
        assert_eq!(
            launch["patch"][0]["config"]["root"],
            directory.join("sessions").to_string_lossy().as_ref()
        );
        assert_eq!(
            launch["patch"][1]["config"]["root"],
            directory.join("storages").to_string_lossy().as_ref()
        );
        assert_eq!(launch["patch"][2]["config"]["path"], ":memory:");
        let persisted = wire
            .iter()
            .filter_map(|record| record["catalogPersisted"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            persisted,
            [
                directory.join("sessions/synthetic.json").to_string_lossy(),
                directory.join("storages/synthetic.json").to_string_lossy()
            ]
        );
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "session/new")
                .count(),
            1
        );
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "session/close")
                .count(),
            usize::from(mode == "normal")
        );
    }
}
