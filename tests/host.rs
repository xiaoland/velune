use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Host(Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn call(dir: &std::path::Path, cmd: &str) -> Value {
    call_value(dir, &json!({"version":1,"command":cmd}))
}
fn call_value(dir: &std::path::Path, request: &Value) -> Value {
    let mut s = UnixStream::connect(dir.join("ipc.sock")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    writeln!(s, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
fn spawn(dir: &std::path::Path) -> Host {
    let h = Host(
        Command::new(env!("CARGO_BIN_EXE_velune-core"))
            .arg("host")
            .arg(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(4);
    while UnixStream::connect(dir.join("ipc.sock")).is_err() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(30));
    }
    h
}
#[test]
fn local_host_survives_client_exit_cancel_restart_and_explicit_stop() {
    let d = tempfile::tempdir().unwrap();
    let mut host = spawn(d.path());
    assert_eq!(call(d.path(), "run")["ok"], true);
    assert_eq!(call(d.path(), "cancel")["ok"], true);
    let s = call(d.path(), "status");
    assert!(
        s["diagnostics"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["state"] == "cancelled")
    );
    assert_eq!(call(d.path(), "unsupported")["ok"], false);
    assert_eq!(call(d.path(), "run")["ok"], true);
    // All connections close after each call; host continues without a UI/client.
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let s = call(d.path(), "status");
        if s["diagnostics"]["attempts"].as_array().unwrap().len() == 4 {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(call(d.path(), "stop")["ok"], true);
    assert!(host.0.wait().unwrap().success());
    let mut restarted = spawn(d.path());
    let s = call(d.path(), "status");
    assert_eq!(s["diagnostics"]["epoch"], 2);
    assert_eq!(s["diagnostics"]["attempts"].as_array().unwrap().len(), 4);
    call(d.path(), "stop");
    assert!(restarted.0.wait().unwrap().success());
}

#[test]
fn generic_ipc_returns_runtime_neutral_projection() {
    let d = tempfile::tempdir().unwrap();
    let mut host = spawn(d.path());
    let response = call_value(d.path(), &json!({"version":3,"action":"list","payload":{}}));
    assert_eq!(response["ok"], true);
    assert!(response["data"]["conversations"].is_array());
    assert!(response["data"]["runtimeTypes"].is_array());
    assert!(response.get("pi").is_none());
    assert_eq!(
        call_value(d.path(), &json!({"version":3,"action":"unknown"}))["ok"],
        false
    );
    assert_eq!(
        call_value(d.path(), &json!({"version":3,"action":"list"}))["ok"],
        true
    );
    assert_eq!(call(d.path(), "stop")["ok"], true);
    assert!(host.0.wait().unwrap().success());
}

#[test]
fn legacy_direct_resource_and_authorization_actions_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let mut host = spawn(d.path());
    for action in ["resources", "settings", "beginAuthorization"] {
        let response = call_value(d.path(), &json!({"version":3,"action":action,"payload":{}}));
        assert_eq!(response["version"], 3);
        assert_eq!(response["ok"], false);
    }
    assert_eq!(call(d.path(), "stop")["ok"], true);
    assert!(host.0.wait().unwrap().success());
}

#[test]
fn gateway_and_runtime_instances_persist_as_independent_configuration() {
    let d = tempfile::tempdir().unwrap();
    let mut host = spawn(d.path());
    let gateway = json!({
        "id":"gateway-1",
        "name":"Fixture gateway",
        "models":[{"id":"model-1","nickname":"Fixture model","icon":null,"maxOutputTokens":64,"reasoningLevels":["standard"]}],
        "providers":[{"id":"provider-1","name":"Fixture provider","protocol":"chatCompletionsV1","endpoint":"http://127.0.0.1:1/v1","credentialRef":"fixture","models":[{"modelId":"model-1","externalModelId":"external-model"}]}],
        "routes":[{"modelId":"model-1","providerId":"provider-1"}],
        "failover":{"mode":"disabled"}
    });
    let saved_gateway = call_value(
        d.path(),
        &json!({"version":3,"action":"gateways","payload":{"operation":"upsert","gateway":gateway.to_string()}}),
    );
    assert_eq!(saved_gateway["version"], 3);
    assert_eq!(saved_gateway["data"]["gateways"][0]["id"], "gateway-1");
    let runtime = json!({
        "id":"runtime-1",
        "name":"Fixture runtime",
        "typeId":"pi",
        "gatewayId":"gateway-1",
        "settings":{"binary":"/tmp/pi","agentDir":"/tmp/pi-agent","workingDir":"/tmp"},
        "modelId":"model-1"
    });
    let saved_runtime = call_value(
        d.path(),
        &json!({"version":3,"action":"runtimeInstances","payload":{"operation":"upsert","runtimeInstance":runtime.to_string()}}),
    );
    assert_eq!(saved_runtime["version"], 3);
    assert_eq!(
        saved_runtime["data"]["runtimeInstances"][0]["id"],
        "runtime-1"
    );
    assert_eq!(call(d.path(), "stop")["ok"], true);
    assert!(host.0.wait().unwrap().success());
    let mut restarted = spawn(d.path());
    let listed = call_value(d.path(), &json!({"version":3,"action":"list","payload":{}}));
    assert_eq!(listed["version"], 3);
    assert_eq!(listed["data"]["gateways"][0]["models"][0]["id"], "model-1");
    assert_eq!(listed["data"]["runtimeInstances"][0]["id"], "runtime-1");
    assert!(listed["data"]["activeRuntimeInstanceID"].is_null());
    assert_eq!(call(d.path(), "stop")["ok"], true);
    assert!(restarted.0.wait().unwrap().success());
}
