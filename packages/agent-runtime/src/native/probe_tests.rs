use super::*;
use crate::probe::ProbeBudget;

#[test]
fn cancelled_probe_does_not_spawn_and_steps_share_the_deadline() {
    let cancelled = ProbeBudget::new(Duration::from_secs(2));
    cancelled.cancel();
    // A nonexistent command would report spawn if cancellation were checked late.
    let error = bounded_process_with_budget(
        crate::process::command("/synthetic/must-not-spawn"),
        None,
        1024,
        &cancelled,
    )
    .unwrap_err();
    assert_eq!(error.code(), "cancelled");
    let (_sender, receiver) = mpsc::sync_channel::<()>(1);
    assert_eq!(
        receive_probe_output(&receiver, &cancelled, "synthetic output")
            .unwrap_err()
            .code(),
        "cancelled",
        "EOF draining must share cancellation too"
    );

    let budget = ProbeBudget::new(Duration::from_millis(500));
    let mut first = crate::process::command("python3");
    first.args(["-c", "import time; time.sleep(0.15); print('first')"]);
    assert_eq!(
        bounded_process_with_budget(first, None, 1024, &budget).unwrap(),
        b"first\n"
    );
    let mut second = crate::process::command("python3");
    second.args(["-c", "import time; time.sleep(0.4)"]);
    assert_eq!(
        bounded_process_with_budget(second, None, 1024, &budget)
            .unwrap_err()
            .code(),
        "timeout",
        "the second step must not receive a fresh timeout"
    );
}

#[cfg(unix)]
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
