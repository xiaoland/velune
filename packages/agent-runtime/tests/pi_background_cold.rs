//! Opt-in regression through the installed Pi SDK and production Rust transport.
//! Input must be the authorized synthetic fixture, which is cloned before use.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use velune_agent_runtime::{
    Client, Config, PiProjection, PiSession,
    conversation::{ConversationSummary, RunState},
};
use velune_test_support::TempRoot;

fn clone_fixture(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in source.read_dir().unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        assert!(
            !ty.is_symlink(),
            "synthetic fixture must not link to another source"
        );
        if ty.is_dir() {
            clone_fixture(&entry.path(), &target.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}
#[test]
#[ignore = "requires VELUNE_TEST_PI_SYNTHETIC_FIXTURE, VELUNE_TEST_PI_BINARY and VELUNE_TEST_NODE_BINARY"]
fn absent_optional_plugins_do_not_timeout_cold_bridge_or_create_model_work() {
    let required = |key| {
        PathBuf::from(std::env::var_os(key).unwrap_or_else(|| {
            panic!("enabled SDK test requires {key}");
        }))
    };
    let fixture = required("VELUNE_TEST_PI_SYNTHETIC_FIXTURE");
    let cli = required("VELUNE_TEST_PI_BINARY");
    let node = required("VELUNE_TEST_NODE_BINARY");
    let root = TempRoot::new("pi-bridge-cold");
    let agent = root.join("agent");
    clone_fixture(&fixture.join("pi"), &agent);
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let source = agent
        .join("sessions")
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .unwrap();
    let original = std::fs::read_to_string(&source).unwrap();
    let mut records: Vec<Value> = original
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    records[0]["cwd"] = json!(workspace);
    std::fs::write(
        &source,
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let before = records
        .iter()
        .filter(|record| record["type"] == "message")
        .count();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let config: Config =
        serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,
        "binary":cli,"node_binary":node,"sdk_helper":resources.join("pi_sessions.mjs"),
        "rpc_entry":resources.join("pi_rpc.mjs"),"extension":resources.join("pi_virtual_model.mjs"),
        "agent_dir":agent,"working_dir":workspace,"provider":"synthetic","model":"alpha",
        "session_dir":agent.join("sessions"),"session":source,"side":false}))
        .unwrap();
    let mut client = Client::spawn(config.clone()).unwrap();
    let (_, messages) = client.state().unwrap();
    let mut projection = PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: format!("synthetic:{}", source.display()),
        runtime_id: "synthetic".into(),
        title: velune_agent_runtime::conversation::conversation_title(None, None),
        cwd: workspace.to_str().map(str::to_owned),
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    });
    projection.replace_history(&messages);
    let mut session = PiSession::new(config.clone());
    // start() performs the first bridge RPC with the production 3s deadline.
    session.start(client, projection);
    assert!(
        !session.has_pending_metadata(),
        "optional absent plugins became unknown work"
    );
    assert!(session.work().is_empty());
    assert!(!session.turn_running());
    assert_eq!(session.snapshot().unwrap().run_state, RunState::Idle);
    session.stop().unwrap();
    // Load a real SDK extension that rejects its Host API request. This is an
    // injected plugin failure, not a mocked SDK or an absent-plugin timeout.
    let extensions = agent.join("extensions");
    std::fs::create_dir_all(&extensions).unwrap();
    std::fs::write(extensions.join("host-failure.ts"), r#"
export default function(pi) {
  pi.events.on("pi-background-tasks:host:request:v1", request => {
    pi.events.emit("pi-background-tasks:host:response:v1", {
      schema_version:"pi-background-tasks.host-response.v1", request_id:request.request_id,
      operation:request.operation, ok:false, error:{code:"synthetic_failure",message:"synthetic host refused"}
    });
  });
}
"#).unwrap();
    let mut failed_host = Client::spawn(config).unwrap();
    failed_host.state().unwrap();
    let failed = failed_host
        .background_work_command(&json!({"operation":"snapshot"}))
        .unwrap();
    assert_eq!(failed["kind"], "unknown");
    assert_eq!(failed["supported"], true);
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("synthetic host refused")
    );
    assert!(
        !failed_host
            .take_buffered_events()
            .iter()
            .any(|event| matches!(
                event["type"].as_str(),
                Some("agent_start" | "message_start" | "message_end")
            ))
    );
    failed_host.shutdown().unwrap();
    let after = std::fs::read_to_string(&source)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["type"] == "message")
        .count();
    assert_eq!(
        before, after,
        "a control query generated native model messages"
    );
}
