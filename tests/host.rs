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
    let mut s = UnixStream::connect(dir.join("ipc.sock")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    writeln!(s, "{}", json!({"version":1,"command":cmd})).unwrap();
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
