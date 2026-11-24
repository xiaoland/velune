use super::*;
use crate::native::rpc::bounded_process_with_budget;
use std::{process::Stdio, sync::mpsc, thread};

#[test]
fn cancelling_probe_stops_its_owned_process_tree_before_returning() {
    let root = std::env::temp_dir().join(format!(
        "velune-probe-cancel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let ready = root.join("ready");
    let budget = ProbeBudget::new(Duration::from_secs(5));
    let worker_budget = budget.clone();
    let mut command = crate::process::command("python3");
    command.args([
        "-c",
        r#"
import os, pathlib, subprocess, sys, time
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
pathlib.Path(sys.argv[1]).write_text(str(os.getpid()) + '\n' + str(child.pid))
time.sleep(60)
"#,
    ]);
    command.arg(&ready);
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let _ = sender.send(bounded_process_with_budget(
            command,
            None,
            1024,
            &worker_budget,
        ));
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    budget.cancel();
    let result = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
    assert_eq!(result.unwrap_err().code(), "cancelled");
    let pids = std::fs::read_to_string(&ready).expect("helper started its descendant");
    for pid in pids.lines() {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let alive = crate::process::command("/bin/kill")
                .args(["-0", pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            if !alive {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned helper process {pid} survived cancellation"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
