use super::*;
use std::time::Duration;
#[test]
fn acp_integration_timely_unharvested_write_does_not_expire_dispatch() {
    let (mut client, root) = super::super::tests::synthetic_client("forms");
    client.create(&root, "integration").unwrap();
    client.enable_command_port().unwrap();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = WriteReceipt::test_pending(d);
    sender
        .send(WriteOutcome::Written(crate::WriteCompletion {
            started_at: d - Duration::from_millis(2),
            completed_at: d - Duration::from_millis(1),
            deadline: d,
        }))
        .unwrap();
    client.install_operation(1, json!(123), Kind::Prompt { run_deadline: None }, write, d);
    client.settle_port_barrier().unwrap();
    assert!(!client.needs_cleanup());
    assert!(
        client
            .take_receipts()
            .iter()
            .any(|r| r.stage == AcpReceiptStage::Written)
    );
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn acp_integration_late_written_keeps_cleanup_after_observation_timeout() {
    let (mut client, root) = super::super::tests::synthetic_client("forms");
    client.create(&root, "integration").unwrap();
    client.enable_command_port().unwrap();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = WriteReceipt::test_pending(d);
    client.install_operation(1, json!(123), Kind::Prompt { run_deadline: None }, write, d);
    assert!(client.settle_port_barrier().is_err());
    sender
        .send(WriteOutcome::Written(crate::WriteCompletion {
            started_at: d - Duration::from_millis(1),
            completed_at: d + Duration::from_millis(1),
            deadline: d,
        }))
        .unwrap();
    assert!(client.settle_port_barrier().is_err());
    assert!(
        client
            .take_receipts()
            .iter()
            .any(|r| r.stage == AcpReceiptStage::Written)
    );
    assert!(client.needs_cleanup());
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
