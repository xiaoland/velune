use super::*;

fn segment(
    runtime: &str,
    native: &str,
    runtime_type_id: RuntimeKind,
) -> crate::config::ConversationSegmentReference {
    crate::config::ConversationSegmentReference {
        runtime_instance_id: runtime.into(),
        runtime_type_id,
        native_conversation_id: native.into(),
        cwd: Some("/synthetic/workspace".into()),
        cutoff: None,
        handoff: None,
        imported: None,
        excluded_message_ids: Vec::new(),
        message_origins: Default::default(),
        deleted: false,
    }
}

fn temporary_directory() -> PathBuf {
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).expect("random test directory identity");
    let path = std::env::temp_dir().join(format!(
        "velune-handoff-registry-{:x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&path).expect("create private test directory");
    path
}

#[test]
fn logical_handoff_resolves_latest_physical_segment() {
    let codex_to_pi = crate::config::ConversationLink {
        id: "logical:conversation".into(),
        segments: vec![
            segment("codex", "native-codex", RuntimeKind::Codex166),
            segment("pi", "native-pi", RuntimeKind::Pi),
        ],
    };
    assert_eq!(
        logical_handoff_source(
            std::slice::from_ref(&codex_to_pi),
            "codex",
            "logical:conversation"
        )
        .expect("linked source identity"),
        ("pi".into(), "native-pi".into()),
        "a conversation already continued in Pi must inspect the Pi tail, not stale Codex compaction"
    );

    let pi_to_codex = crate::config::ConversationLink {
        id: "logical:conversation".into(),
        segments: vec![
            segment("pi", "native-pi", RuntimeKind::Pi),
            segment("codex-next", "native-codex-next", RuntimeKind::Codex166),
        ],
    };
    assert_eq!(
        logical_handoff_source(&[pi_to_codex], "pi", "logical:conversation")
            .expect("linked source identity"),
        ("codex-next".into(), "native-codex-next".into())
    );
    assert!(
        logical_handoff_source(&[codex_to_pi], "wrong-runtime", "logical:conversation").is_err()
    );
}

#[test]
fn unlinked_handoff_uses_the_logical_runtime_prefix() {
    assert_eq!(
        logical_handoff_source(&[], "codex", "codex:thread-7").expect("native identity"),
        ("codex".into(), "thread-7".into())
    );
    assert!(logical_handoff_source(&[], "codex", "pi:thread-7").is_err());
}

#[test]
fn summary_provenance_projects_one_stable_maintenance_event() {
    let mut imported = segment("pi", "pi:/synthetic/session.jsonl", RuntimeKind::Pi);
    imported.imported = Some(crate::config::ConversationImport {
        prefix: crate::config::ConversationCutoff {
            message_count: 0,
            prefix_digest: super::super::continuation::prefix_digest(&[]).unwrap(),
        },
        summary_work_id: Some("work-1".into()),
    });
    let disk = serde_json::to_vec(&imported).unwrap();
    let reopened: crate::config::ConversationSegmentReference =
        serde_json::from_slice(&disk).unwrap();
    for index in [1, 3] {
        let shown = super::super::continuation::display_segment(&reopened, &[], index).unwrap();
        let [message] = shown.as_slice() else {
            panic!("one maintenance event")
        };
        assert_eq!(message.id, "velune-handoff-compaction:work-1");
        assert_eq!(message.role, crate::conversation::MessageRole::System);
        assert!(message.completed);
        assert!(
            matches!(&message.blocks[..], [crate::conversation::MessageBlock::ContextCompaction { text }]
            if text == "已为接续生成摘要")
        );
    }
}

#[test]
fn linked_codex_handoff_accepts_existing_qualified_ids_and_native_uuids() {
    let uuid = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    for id in [uuid.to_owned(), format!("codex-source:{uuid}")] {
        let link = crate::config::ConversationLink {
            id: "logical:conversation".into(),
            segments: vec![
                segment("pi-origin", "source-file", RuntimeKind::Pi),
                segment("codex-source", &id, RuntimeKind::Codex166),
            ],
        };
        let original = link.clone();
        assert_eq!(
            logical_handoff_source(std::slice::from_ref(&link), "pi-origin", &link.id).unwrap(),
            ("codex-source".into(), uuid.into())
        );
        assert_eq!(
            link.segments[1].native_conversation_id,
            original.segments[1].native_conversation_id
        );
    }

    let pi_path = "C:\\synthetic\\session.jsonl";
    let link = crate::config::ConversationLink {
        id: "logical:conversation".into(),
        segments: vec![segment("C", pi_path, RuntimeKind::Pi)],
    };
    assert_eq!(
        logical_handoff_source(&[link], "C", "logical:conversation").unwrap(),
        ("C".into(), pi_path.into()),
        "a native file path is not a Codex runtime qualifier"
    );
}

#[test]
fn registry_records_exact_recovery_identity_and_removes_archived_work() {
    let home = temporary_directory();
    let mut registry = ForkRegistry::open(&home);
    registry
        .begin("work-1", "codex", "source-thread", &home.join("codex"))
        .expect("register summary work before fork creation");
    registry
        .observe_temporary("work-1", "summary-thread", None)
        .expect("record fork identity as soon as it is observed");
    drop(registry);
    let mut registry = ForkRegistry::open(&home);
    let incomplete = registry
        .recovery_plans()
        .expect("incomplete identity after restart");
    assert_eq!(incomplete.len(), 1);
    assert_eq!(
        incomplete[0].temporary_thread_id.as_deref(),
        Some("summary-thread")
    );
    assert!(incomplete[0].temporary_fingerprint.is_none());
    registry
        .observe_temporary("work-1", "summary-thread", Some("sha256:synthetic"))
        .expect("complete recovery fingerprint");
    drop(registry);
    let mut registry = ForkRegistry::open(&home);
    let plans = registry
        .recovery_plans()
        .expect("valid registry after restart");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].work_id, "work-1");
    assert_eq!(plans[0].source_runtime_id, "codex");
    assert_eq!(plans[0].source_native_id, "source-thread");
    assert_eq!(plans[0].source_home, home.join("codex"));
    assert_eq!(
        plans[0].temporary_thread_id.as_deref(),
        Some("summary-thread")
    );
    assert_eq!(
        plans[0].temporary_fingerprint.as_deref(),
        Some("sha256:synthetic")
    );
    registry.finish("work-1").expect("finish archived work");
    drop(registry);
    let registry = ForkRegistry::open(&home);
    assert!(
        registry
            .recovery_plans()
            .expect("empty registry")
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn malformed_registry_is_preserved_and_only_blocks_registry_writes() {
    let home = temporary_directory();
    let path = home.join("portable-handoff-forks.json");
    let original = b"{not valid registry bytes";
    std::fs::write(&path, original).expect("write synthetic malformed registry");
    let mut registry = ForkRegistry::open(&home);
    assert!(
        registry
            .begin("work-1", "codex", "source-thread", &home.join("codex"))
            .is_err()
    );
    assert_eq!(
        std::fs::read(&path).expect("read preserved registry"),
        original
    );
    let _ = std::fs::remove_dir_all(home);
}
