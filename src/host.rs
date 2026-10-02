//! Local same-user prototype IPC. No TCP/LAN listener, no credentials, no launch agent.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use velune_core::Core;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn request(dir: &Path, command: &str) -> Result<String> {
    let mut stream = UnixStream::connect(dir.join("ipc.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{}", json!({"version":1,"command":command}))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err("host closed without response".into());
    }
    Ok(line)
}
pub fn serve(dir: &Path) -> Result<()> {
    if dir.exists() && fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err("host directory must not be a symlink".into());
    }
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    // Acquire DB ownership before removing a stale socket. A second Host cannot unlink a live one.
    let mut core = Core::open(dir.join("core.sqlite"))?;
    let socket = dir.join("ipc.sock");
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut last = Instant::now();
    let mut last_error: Option<String> = None;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_millis(500)))?;
                stream.set_write_timeout(Some(Duration::from_millis(500)))?;
                let mut line = String::new();
                // Commands contain no payload. Bound parsing and ignore incomplete client requests.
                let read = std::io::Read::take(&mut stream, 4096);
                if BufReader::new(read).read_line(&mut line).is_err() {
                    continue;
                }
                let value: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let command = value["command"].as_str().unwrap_or("");
                let mut stop = false;
                let outcome: velune_core::Result<()> = if value["version"] != 1 {
                    Err(velune_core::Error::Unsupported("IPC version"))
                } else {
                    match command {
                        "run" => {
                            last = Instant::now();
                            core.enqueue_fixture(now()).map(|_| ())
                        }
                        "status" => Ok(()),
                        "cancel" => core.cancel_pending(),
                        "stop" => {
                            stop = true;
                            core.cancel_pending()
                        }
                        _ => Err(velune_core::Error::Unsupported("IPC command")),
                    }
                };
                let response = match outcome {
                    Ok(()) => {
                        json!({"ok":true,"diagnostics":core.diagnostics()?,"host_error":last_error})
                    }
                    Err(error) => json!({"ok":false,"error":error.to_string()}),
                };
                // A disconnected UI must not terminate the independent Host.
                let _ = writeln!(stream, "{response}");
                if stop {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if last.elapsed() >= Duration::from_millis(900) {
            if let Err(error) = core.tick(now()) {
                last_error = Some(error.to_string());
            }
            last = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    fs::remove_file(socket)?;
    Ok(())
}
