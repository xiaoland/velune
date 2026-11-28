//! Opt-in regression through the installed Pi SDK and production Rust transport.
//! The checked-in synthetic session is materialized in an isolated agent directory.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use velune_agent_runtime::{
    Client, Config, PiProjection, PiSession,
    conversation::{ConversationSummary, RunState},
};
use velune_test_support::TempRoot;

struct ModelProbe {
    address: std::net::SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ModelProbe {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, done) = (requests.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut bytes = [0; 4096];
                        if stream.read(&mut bytes).unwrap_or(0) != 0 {
                            count.fetch_add(1, Ordering::SeqCst);
                            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if done.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("model probe: {error}"),
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for ModelProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn settle_background(session: &mut PiSession) {
    let end = Instant::now() + Duration::from_secs(5);
    while session.has_pending_metadata() && session.background_tasks().issue.is_none() {
        session.poll();
        assert!(
            Instant::now() < end,
            "background discovery must return a snapshot or diagnostic without a model turn"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn start_session(config: Config, workspace: &Path, source: &Path) -> PiSession {
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
    let mut session = PiSession::new(config);
    session.start(client, projection);
    settle_background(&mut session);
    session
}

#[test]
#[ignore = "requires VELUNE_TEST_PI_BINARY and VELUNE_TEST_NODE_BINARY"]
fn absent_optional_plugins_do_not_timeout_cold_bridge_or_create_model_work() {
    let required = |key| {
        PathBuf::from(std::env::var_os(key).unwrap_or_else(|| {
            panic!("enabled SDK test requires {key}");
        }))
    };
    let cli = required("VELUNE_TEST_PI_BINARY");
    let node = required("VELUNE_TEST_NODE_BINARY");
    let root = TempRoot::new("pi-bridge-cold");
    let agent = root.join("agent");
    let workspace = root.dir("workspace");
    let model = ModelProbe::start();
    let source = velune_test_support::write_pi_three_turn_session(
        &agent,
        &workspace,
        &format!("http://{}/v1", model.address),
    );
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let config: Config =
        serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,
        "binary":cli,"node_binary":node,"sdk_helper":resources.join("pi_sessions.mjs"),
        "rpc_entry":resources.join("pi_rpc.mjs"),"extension":resources.join("pi_virtual_model.mjs"),
        "agent_dir":agent,"working_dir":workspace,"provider":"synthetic","model":"alpha",
        "session_dir":agent.join("sessions"),"session":source,"side":false}))
        .unwrap();
    let mut session = start_session(config.clone(), &workspace, &source);
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
    let mut failed_host = start_session(config, &workspace, &source);
    let tasks = failed_host.background_tasks();
    assert!(!tasks.live);
    assert!(
        tasks
            .issue
            .as_deref()
            .unwrap()
            .contains("synthetic host refused")
    );
    assert!(!failed_host.turn_running());
    failed_host.stop().unwrap();
    let model_requests = model.requests.clone();
    drop(model); // Drain queued loopback requests after both SDK processes have stopped.
    assert_eq!(
        model_requests.load(Ordering::SeqCst),
        0,
        "background control discovery must not contact the model endpoint"
    );
    let after = std::fs::read_to_string(&source)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["type"] == "message")
        .count();
    assert_eq!(after, 6, "a control query generated native model messages");
}
