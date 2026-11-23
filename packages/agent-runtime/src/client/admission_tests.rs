use super::*;
use crate::transport::{OwnedFrame, WriteClass};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn pi_owned_bridge_admits_control_while_blocked_and_preserves_cancel_dependency() {
    let root = Directory(std::env::temp_dir().join(format!(
                "velune-pi-admission-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
    std::fs::create_dir(&root.0).unwrap();
    let helper = root.0.join("helper.py");
    std::fs::write(
        &helper,
        r#"
import sys, pathlib, time, json
root = pathlib.Path(__file__).parent
print(json.dumps({'type':'synthetic_notice'}), flush=True)
while not (root / 'release').exists(): time.sleep(.005)
for line in sys.stdin:
    value = json.loads(line)
    print(json.dumps({'type':'synthetic_wire', 'command':value['type']}), flush=True)
    if value['type'] in ('clear_queue', 'abort'):
        print(json.dumps({'type':'response','id':value['id'],'success':True}), flush=True)
"#,
    )
    .unwrap();
    let python = crate::process::command("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "binary":"/synthetic/pi",
        "node_binary":String::from_utf8(python.stdout).unwrap().trim(),
        "rpc_entry":helper, "os_home":root.0, "agent_dir":root.0,
        "working_dir":root.0, "session_dir":root.0,
    }))
    .unwrap();
    let mut client = Client::spawn(config).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes =
        serde_json::to_vec(&json!({"type":"prompt","message":"x".repeat(9 * 1024 * 1024)}))
            .unwrap();
    bytes.push(b'\n');
    let prompt = client
        .try_enqueue_owned_frame(
            OwnedFrame::new(bytes, 32 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    let reply = client
        .try_enqueue_owned_frame(
            OwnedFrame::new(b"{\"type\":\"extension_ui_response\"}\n".to_vec(), 1024),
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
        .unwrap();
    let cancel = client
        .try_enqueue_owned_frame(
            OwnedFrame::new(b"{\"type\":\"explicit_cancel\"}\n".to_vec(), 1024),
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
        .unwrap();
    let mut noticed = false;
    while !noticed && Instant::now() < deadline {
        noticed = client
            .poll()
            .iter()
            .any(|v| v["type"] == "synthetic_notice");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(noticed);
    for receipt in [&prompt, &reply, &cancel] {
        assert!(receipt.try_complete().is_none());
    }
    client.terminal_error = Some("synthetic recorded terminal".into());
    let frame = OwnedFrame::new(Vec::with_capacity(128), 1024);
    let pointer = frame.bytes.as_ptr();
    let rejected = client
        .try_enqueue_owned_frame(frame, WriteClass::ProtocolReplyCancel, deadline)
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
    assert_eq!(rejected.deadline, deadline);
    assert!(client.terminal_error.is_some());
    client.terminal_error = None;
    let rejected = client
        .try_enqueue_owned_frame(
            OwnedFrame::new(Vec::with_capacity(128), 64),
            WriteClass::Ordinary,
            deadline,
        )
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(client.terminal_error.is_none());
    std::fs::write(root.0.join("release"), []).unwrap();
    let mut wire = Vec::new();
    while wire.len() < 3 && Instant::now() < deadline {
        wire.extend(client.poll());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        wire.iter()
            .map(|v| v["command"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["prompt", "extension_ui_response", "explicit_cancel"]
    );
    for receipt in [prompt, reply, cancel] {
        receipt.wait().unwrap();
    }
    assert_eq!(client.cancel().unwrap().len(), 2);
    let cancel_wire = client.poll();
    assert_eq!(
        cancel_wire
            .iter()
            .map(|v| v["command"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["clear_queue", "abort"]
    );
    client.child.stop();
}
