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
