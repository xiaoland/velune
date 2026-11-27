//! Opt-in production Codex CLI integration using only a private synthetic home
//! and a localhost Responses provider. Run explicitly with:
//! `VELUNE_RUN_NATIVE_HANDOFF_E2E=1 VELUNE_CODEX_E2E_BIN=/path/to/codex cargo test -p velune-agent-runtime --test codex_handoff_summary -- --ignored --nocapture`
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use velune_agent_runtime::{
    TurnInput,
    conversation::model_variant::codex_speed_model,
    execution::ExecutionTarget,
    native::{
        HandoffSummaryArtifactState, HandoffSummaryCleanupState, HandoffSummaryPhase,
        HandoffSummaryRequest, HandoffSummaryStatus, NativeConfig, NativeKind, NativeSession,
        RuntimeModel,
    },
    process,
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("velune-handoff-e2e-{stamp}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Provider {
    port: u16,
    requests: Arc<Mutex<Vec<Value>>>,
    next_delay_ms: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}
impl Provider {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let next_delay_ms = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_delay = Arc::clone(&next_delay_ms);
        let thread_stopped = Arc::clone(&stopped);
        let server = thread::spawn(move || {
            while !thread_stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let requests = Arc::clone(&thread_requests);
                        let delay = Arc::clone(&thread_delay);
                        thread::spawn(move || serve_response(stream, requests, delay));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            requests,
            next_delay_ms,
            stopped,
            server: Some(server),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    fn delay_next(&self, duration: Duration) {
        self.next_delay_ms
            .store(duration.as_millis() as u64, Ordering::Relaxed);
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn serve_response(
    mut stream: TcpStream,
    requests: Arc<Mutex<Vec<Value>>>,
    delay_ms: Arc<AtomicU64>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(size) => {
                bytes.extend_from_slice(&chunk[..size]);
                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
            }
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(size) => bytes.extend_from_slice(&chunk[..size]),
        }
    }
    let body: Value = match serde_json::from_slice(&bytes[header_end..header_end + content_length])
    {
        Ok(body) => body,
        Err(_) => return,
    };
    requests.lock().unwrap().push(body.clone());
    let delay = delay_ms.swap(0, Ordering::Relaxed);
    if delay != 0 {
        thread::sleep(Duration::from_millis(delay));
    }

    let request_number = requests.lock().unwrap().len();
    let text = "Synthetic portable summary: preserve the user's goal and decisions.";
    let item = json!({
        "type":"message","id":format!("msg_{request_number}"),"status":"completed",
        "role":"assistant","phase":"final_answer",
        "content":[{"type":"output_text","text":text,"annotations":[]}]
    });
    let response = json!({
        "id":format!("resp_{request_number}"),"object":"response","status":"completed",
        "model":body["model"],"output":[item],
        "usage":{"input_tokens":32,"output_tokens":12,"total_tokens":44}
    });
    let events = [
        json!({"type":"response.created","response":{"id":response["id"],"object":"response","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":item}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":response}),
    ];
    let payload = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {}\n\n",
                event["type"].as_str().unwrap(),
                event
            )
        })
        .collect::<String>();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(payload.as_bytes());
}

fn config(root: &Path, binary: PathBuf) -> NativeConfig {
    let home = root.join("home");
    let codex_home = root.join("codex");
    let workspace = root.join("workspace");
    let resources = root.join("resources");
    let projections = root.join("projections");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&resources).unwrap();
    fs::create_dir_all(&projections).unwrap();
    NativeConfig {
        execution: ExecutionTarget::Local,
        os_home: Some(home),
        kind: NativeKind::Codex166,
        binary,
        node_binary: None,
        agent_dir: codex_home,
        resources_directory: resources,
        projection_directory: projections,
        gateway: None,
        auto_compaction: None,
        mcp_servers: vec![],
        session_agents: vec![],
    }
}

fn wait_terminal(
    session: &mut NativeSession,
    work_id: &str,
    timeout: Duration,
) -> HandoffSummaryStatus {
    let deadline = Instant::now() + timeout;
    let mut previous_phase = None;
    loop {
        session.poll().unwrap();
        if let Some(status) = session.handoff_summary_status(work_id).unwrap() {
            match &*status {
                HandoffSummaryStatus::Running { phase, .. } => {
                    if previous_phase.as_ref() != Some(phase) {
                        eprintln!("synthetic handoff {work_id}: {phase:?}");
                        previous_phase = Some(phase.clone());
                    }
                }
                HandoffSummaryStatus::Ready { cleanup_state, .. } => {
                    eprintln!("synthetic handoff {work_id}: Ready {cleanup_state:?}");
                    return status.into_parts().0;
                }
                HandoffSummaryStatus::Failed { code, detail } => {
                    eprintln!("synthetic handoff {work_id}: Failed {code}: {detail}");
                    return status.into_parts().0;
                }
                HandoffSummaryStatus::Cancelled => {
                    eprintln!("synthetic handoff {work_id}: Cancelled");
                    return status.into_parts().0;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "summary work {work_id} timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn poll_until_requests(
    session: &mut NativeSession,
    provider: &Provider,
    count: usize,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while provider.request_count() < count {
        assert!(
            Instant::now() < deadline,
            "provider did not receive request {count}"
        );
        session.poll().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
}

fn native_id(session: &NativeSession) -> String {
    session
        .snapshot()
        .unwrap()
        .conversation
        .id
        .split_once(':')
        .unwrap()
        .1
        .to_owned()
}

fn rollout_for(codex_home: &Path, id: &str) -> PathBuf {
    fn find(path: &Path, id: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(path).ok()? {
            let entry = entry.ok()?;
            let candidate = entry.path();
            if candidate.is_dir() {
                if let Some(found) = find(&candidate, id) {
                    return Some(found);
                }
            } else if candidate.extension().is_some_and(|ext| ext == "jsonl")
                && fs::read(&candidate)
                    .ok()
                    .is_some_and(|bytes| String::from_utf8_lossy(&bytes).contains(id))
            {
                return Some(candidate);
            }
        }
        None
    }
    find(&codex_home.join("sessions"), id).expect("synthetic thread rollout exists")
}

fn start_summary(
    session: &mut NativeSession,
    work_id: &str,
    source_id: &str,
    selected_model: Option<RuntimeModel>,
) {
    session
        .begin_handoff_summary(HandoffSummaryRequest {
            work_id: work_id.to_owned(),
            source_native_id: source_id.to_owned(),
            source_provider_id: "synthetic".into(),
            source_model_id: "gpt-6.1-sol".into(),
            selected_model,
            effort: Some("medium".into()),
        })
        .unwrap();
}

fn selected(id: String) -> RuntimeModel {
    RuntimeModel {
        provider_id: Some("synthetic".into()),
        name: id.clone(),
        id,
    }
}

struct ReadOnlyDirectory(PathBuf);
impl ReadOnlyDirectory {
    fn new(path: PathBuf) -> Self {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o555)).unwrap();
        Self(path)
    }
}
impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

#[test]
#[ignore = "requires explicit isolated Codex CLI and localhost-only synthetic provider"]
fn production_native_handoff_summary_uses_selected_tier_and_preserves_source() {
    assert_eq!(
        std::env::var("VELUNE_RUN_NATIVE_HANDOFF_E2E").as_deref(),
        Ok("1")
    );
    // The only configured endpoint is localhost. Keep ambient provider
    // credentials and outbound proxy settings out of child CLI processes.
    unsafe {
        for key in [
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "OPENAI_ORG_ID",
            "OPENAI_PROJECT_ID",
            "CODEX_API_KEY",
            "VELUNE_GATEWAY_TOKEN",
        ] {
            std::env::remove_var(key);
        }
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            std::env::set_var(key, "http://127.0.0.1:1");
        }
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
        std::env::set_var("DISABLE_TELEMETRY", "1");
    }
    let binary = std::env::var_os("VELUNE_CODEX_E2E_BIN")
        .map(PathBuf::from)
        .expect("set VELUNE_CODEX_E2E_BIN to the installed Codex CLI");
    assert!(binary.is_file(), "Codex CLI path does not exist");
    let version = process::command(&binary).arg("--version").output().unwrap();
    assert!(version.status.success(), "Codex CLI failed --version");
    let version = String::from_utf8_lossy(&version.stdout);
    assert!(
        version.contains("0.167."),
        "expected tested Codex 0.167 CLI, got {version}"
    );

    let scratch = Scratch::new();
    let provider = Provider::start();
    let root = &scratch.0;
    let workspace = root.join("workspace");
    let codex_home = root.join("codex");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(root.join("home")).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"model = "gpt-6.1-sol"
model_provider = "synthetic"
model_reasoning_effort = "high"
approval_policy = "never"
sandbox_mode = "read-only"
[model_providers.synthetic]
name = "Synthetic"
base_url = "http://127.0.0.1:{}/v1"
wire_api = "responses"
experimental_bearer_token = "synthetic-fake-token"
requires_openai_auth = false
[features]
responses_websockets = false
responses_websockets_v2 = false
hooks = false
goals = false
plugins = false
apps = false
memories = false
shell_snapshot = false
"#,
            provider.port
        ),
    )
    .unwrap();

    let binary = fs::canonicalize(binary).unwrap();
    let mut seed = NativeSession::connect(config(root, binary.clone())).unwrap();
    seed.create(Some(&workspace), "synthetic").unwrap();
    let source_id = native_id(&seed);
    seed.send(&TurnInput {
        text: "Synthetic original user request.".into(),
        effort: None,
        permission_mode: None,
        images: vec![],
    })
    .unwrap();
    let seed_deadline = Instant::now() + Duration::from_secs(30);
    while seed.turn_running() {
        seed.poll().unwrap();
        assert!(
            Instant::now() < seed_deadline,
            "synthetic seed turn timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(seed);
    let source_path = rollout_for(&codex_home, &source_id);
    let compacted = json!({
        "timestamp":"2026-11-12T00:00:00.000Z","type":"compacted",
        "payload":{"message":"","replacement_history":[
            {"type":"message","id":"synthetic_retained_user","role":"user","content":[{"type":"input_text","text":"Synthetic retained user."}]},
            {"type":"compaction","id":"synthetic_encrypted","encrypted_content":"SYNTHETIC-OPAQUE-CONTEXT"},
            {"type":"function_call","call_id":"synthetic-tool","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"synthetic-tool","output":"Synthetic tool result."},
            {"type":"message","id":"synthetic_tail","role":"assistant","content":[{"type":"output_text","text":"Synthetic retained tail."}]}
        ]}
    });
    let mut append = fs::OpenOptions::new()
        .append(true)
        .open(&source_path)
        .unwrap();
    writeln!(append, "{compacted}").unwrap();
    drop(append);
    let original_source = fs::read(&source_path).unwrap();

    // An enabled MCP server in this private CODEX_HOME is an observable canary.
    // It may be discovered and disabled for the temporary fork, but must never start.
    let canary = root.join("mcp-started");
    let mcp = root.join("mcp-canary.sh");
    fs::write(
        &mcp,
        format!("#!/bin/sh\nprintf started > '{}'\n", canary.display()),
    )
    .unwrap();
    fs::set_permissions(&mcp, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config_toml = fs::read_to_string(codex_home.join("config.toml")).unwrap();
    config_toml.push_str(&format!(
        "\n[mcp_servers.canary]\ncommand = \"{}\"\nenabled = true\n",
        mcp.display()
    ));
    fs::write(codex_home.join("config.toml"), config_toml).unwrap();

    let mut session = NativeSession::connect(config(root, binary)).unwrap();
    assert!(
        !canary.exists(),
        "MCP started while summary NativeSession initialized"
    );
    let baseline_http_requests = provider.request_count();

    start_summary(&mut session, "default", &source_id, None);
    let default_status = wait_terminal(&mut session, "default", Duration::from_secs(45));
    let HandoffSummaryStatus::Ready {
        summary,
        cleanup_state,
        ..
    } = default_status
    else {
        panic!("default summary failed: {default_status:?}");
    };
    assert!(summary.contains("Synthetic portable summary"));
    assert_eq!(cleanup_state, HandoffSummaryCleanupState::Archived);
    assert_eq!(fs::read(&source_path).unwrap(), original_source);
    assert!(!canary.exists(), "MCP started during default summary");
    let default_request = provider.requests().last().unwrap().clone();
    assert_eq!(default_request["model"], "gpt-6.1-sol");
    assert!(default_request.get("service_tier").is_none());
    assert_eq!(default_request["reasoning"]["effort"], "medium");

    start_summary(
        &mut session,
        "base",
        &source_id,
        Some(selected("gpt-6.1-sol".into())),
    );
    let base_status = wait_terminal(&mut session, "base", Duration::from_secs(45));
    let HandoffSummaryStatus::Ready { cleanup_state, .. } = base_status else {
        panic!("base summary failed: {base_status:?}");
    };
    assert_eq!(cleanup_state, HandoffSummaryCleanupState::Archived);
    assert_eq!(fs::read(&source_path).unwrap(), original_source);
    let base_request = provider.requests().last().unwrap().clone();
    assert_eq!(base_request["model"], "gpt-6.1-sol");
    assert!(base_request.get("service_tier").is_none());

    let priority_id = codex_speed_model("gpt-6.1-sol", "priority");
    start_summary(
        &mut session,
        "priority",
        &source_id,
        Some(selected(priority_id.clone())),
    );
    let priority_status = wait_terminal(&mut session, "priority", Duration::from_secs(45));
    let HandoffSummaryStatus::Ready {
        summary_model_id,
        cleanup_state,
        ..
    } = priority_status
    else {
        panic!("priority summary failed: {priority_status:?}");
    };
    assert_eq!(summary_model_id, priority_id);
    assert_eq!(cleanup_state, HandoffSummaryCleanupState::Archived);
    assert_eq!(fs::read(&source_path).unwrap(), original_source);
    let priority_request = provider.requests().last().unwrap().clone();
    assert_eq!(priority_request["model"], "gpt-6.1-sol");
    assert_eq!(priority_request["service_tier"], "priority");
    assert!(
        priority_request
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );
    assert!(!canary.exists(), "MCP started during selected-tier summary");

    // Make archive fail after the owned fork has been verified and sampled.
    // A successful summary remains usable and reports retained cleanup.
    let before_archive_count = provider.request_count();
    provider.delay_next(Duration::from_millis(1200));
    start_summary(&mut session, "archive-failure", &source_id, None);
    poll_until_requests(
        &mut session,
        &provider,
        before_archive_count + 1,
        Duration::from_secs(30),
    );
    let archive_status = session.handoff_summary_status("archive-failure").unwrap();
    let archive_thread_id = match archive_status.as_deref() {
        Some(HandoffSummaryStatus::Running {
            temporary_thread_id: Some(id),
            phase: HandoffSummaryPhase::Compacting,
            ..
        }) => id,
        state => panic!("sampling fork identity was not observable: {state:?}"),
    };
    let archive_rollout = rollout_for(&codex_home, archive_thread_id);
    drop(archive_status);
    let readonly_parent = ReadOnlyDirectory::new(archive_rollout.parent().unwrap().to_path_buf());
    let archive_failure = wait_terminal(&mut session, "archive-failure", Duration::from_secs(45));
    drop(readonly_parent);
    let HandoffSummaryStatus::Ready {
        summary,
        cleanup_state,
        ..
    } = archive_failure
    else {
        panic!("archive failure discarded a valid summary: {archive_failure:?}");
    };
    assert!(summary.contains("Synthetic portable summary"));
    assert_eq!(
        cleanup_state,
        HandoffSummaryCleanupState::RetainedUnverified
    );
    assert_eq!(
        session
            .handoff_summary_artifact("archive-failure")
            .unwrap()
            .unwrap()
            .state,
        HandoffSummaryArtifactState::MayExist
    );
    assert_eq!(fs::read(&source_path).unwrap(), original_source);

    // Source changes during sample invalidate the eventual Ready result.
    let before_mutation_count = provider.request_count();
    provider.delay_next(Duration::from_millis(1200));
    start_summary(&mut session, "changed", &source_id, None);
    poll_until_requests(
        &mut session,
        &provider,
        before_mutation_count + 1,
        Duration::from_secs(30),
    );
    // A blank JSONL record changes the native source fingerprint while
    // preserving Codex's valid history semantics.
    let mutation = b"\n";
    let mut append = fs::OpenOptions::new()
        .append(true)
        .open(&source_path)
        .unwrap();
    append.write_all(mutation).unwrap();
    drop(append);
    let expected_changed_source = fs::read(&source_path).unwrap();
    let changed_status = wait_terminal(&mut session, "changed", Duration::from_secs(45));
    assert!(
        matches!(changed_status, HandoffSummaryStatus::Failed { ref code, .. } if code == "source_changed"),
        "unexpected source CAS outcome: {changed_status:?}"
    );
    assert_eq!(fs::read(&source_path).unwrap(), expected_changed_source);
    assert!(!canary.exists());

    // A late response after cancellation must never become a successful summary.
    let before_cancel_count = provider.request_count();
    provider.delay_next(Duration::from_secs(3));
    start_summary(&mut session, "cancel", &source_id, None);
    poll_until_requests(
        &mut session,
        &provider,
        before_cancel_count + 1,
        Duration::from_secs(30),
    );
    session.cancel_handoff_summary("cancel").unwrap();
    let cancelled = wait_terminal(&mut session, "cancel", Duration::from_secs(14));
    assert!(
        !matches!(cancelled, HandoffSummaryStatus::Ready { .. }),
        "late model text was accepted after cancellation"
    );
    thread::sleep(Duration::from_secs(3));
    for _ in 0..20 {
        session.poll().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!matches!(
        session.handoff_summary_status("cancel").unwrap().as_deref(),
        Some(HandoffSummaryStatus::Ready { .. })
    ));
    assert!(!canary.exists());
    assert_eq!(provider.request_count(), baseline_http_requests + 6);

    // A live model response can take longer than Codex thread/sample allowed.
    provider.delay_next(Duration::from_secs(34));
    start_summary(&mut session, "slow-compaction", &source_id, None);
    let slow = wait_terminal(&mut session, "slow-compaction", Duration::from_secs(65));
    assert!(
        matches!(&slow, HandoffSummaryStatus::Ready { summary, .. }
            if summary.contains("Synthetic portable summary")),
        "long compaction failed: {slow:?}"
    );
    assert_eq!(provider.request_count(), baseline_http_requests + 7);
    assert_eq!(fs::read(&source_path).unwrap(), expected_changed_source);
}
