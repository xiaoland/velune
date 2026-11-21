use super::*;
use std::time::Duration;
#[test]
fn claude_integration_late_written_fact_survives_consumed_failure_receipt() {
    let mut fixture = super::super::claude_port_tests::Fixture::new("hold");
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, receipt) = crate::transport::WriteReceipt::test_pending(d);
    let ticket = ClaudeOperationTicket {
        session: fixture.session.epoch.clone(),
        generation: fixture.session.claude.commands.generation,
        sequence: 100,
        request_id: Value::Null,
    };
    fixture.session.claude.commands.pending.insert(
        100,
        Operation {
            ticket: ticket.clone(),
            kind: Kind::OneWay,
            ack_deadline: None,
            physical_write: None,
        },
    );
    fixture.session.claude.commands.writes.insert(
        100,
        PendingWrite {
            receipt,
            bytes: 0,
            ticket: ticket.clone(),
            retention: DataRetention::default(),
        },
    );
    assert!(fixture.session.claude_settle_barrier().is_err());
    fixture.session.take_claude_receipts();
    sender
        .send(crate::transport::WriteOutcome::Written(
            crate::WriteCompletion {
                started_at: d - Duration::from_millis(1),
                completed_at: d + Duration::from_millis(1),
                deadline: d,
            },
        ))
        .unwrap();
    assert!(fixture.session.claude_settle_barrier().is_err());
    let receipts = fixture.session.take_claude_receipts();
    assert!(receipts.iter().any(|r| r.ticket == ticket
        && matches!(r.delivery, ClaudeDelivery::Indeterminate { .. })
        && matches!(r.physical_write, Some(crate::WriteFact::Written(c)) if !c.met_deadline())));
    assert!(fixture.session.claude_cleanup_required());
}
