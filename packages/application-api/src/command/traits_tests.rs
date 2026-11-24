use super::*;

/// Recorded against the pre-migration classifiers. The fixtures cover every Command,
/// every scheduled/composer operation, and the composer history-address exceptions.
#[test]
fn command_traits_match_baseline() {
    let cases = include_str!("traits_cases.tsv").lines();
    let expected = include_str!("traits_expected.tsv").lines().skip(1);
    assert_eq!(cases.clone().count(), expected.clone().count());

    for (case, expected) in cases.zip(expected) {
        let (name, json) = case.split_once('\t').expect("fixture name and command");
        let command: Command =
            serde_json::from_str(json).unwrap_or_else(|error| panic!("{name}: {error}"));
        let traits = command.traits();
        let invalidates = ["conversation", "unrelated", "missing"]
            .map(|id| traits.affects_history(&[id.to_owned()]));
        let (runtime_id, conversation_id) = traits
            .operation_target
            .map(|target| (target.runtime_id, target.conversation_id.unwrap_or("-")))
            .unwrap_or(("-", "-"));
        let actual = format!(
            "{name}\t{:?}\t{}\t{}\t{}\t{}\t{invalidates:?}\t{}\t{:?}\t{runtime_id}\t{conversation_id}",
            command.delivery_class(),
            command.requires_revision(),
            command.changes_shared_state(),
            traits.touch_view,
            traits.trace_owner_queue,
            route_description(traits.route),
            command.kind(),
        );
        assert_eq!(actual, expected, "{name}");
    }
}

fn route_description(route: CommandRoute<'_>) -> String {
    match route {
        CommandRoute::Owner {
            submission_conversation_id,
        } => submission_conversation_id
            .map_or_else(|| "owner".into(), |id| format!("owner:submission({id})")),
        CommandRoute::Live(LiveRoute { kind, address }) => {
            let response = kind.response()(SnapshotResult {
                snapshot: None,
                submitted_message_id: None,
                submission_receipt: None,
            });
            let response = serde_json::to_value(response).unwrap();
            let response = response["response"].as_str().unwrap();
            format!(
                "live({kind:?},{},{},{},{response})",
                address.runtime_id,
                address.conversation_id.unwrap_or_default(),
                kind == ExecutionCommandKind::Create,
            )
        }
        CommandRoute::ListRefresh => "list-refresh".into(),
        CommandRoute::ReadHistory(address) => history_description("read", address),
        CommandRoute::HistoryWithSourceRefresh(HistoryLookup::Refresh(address)) => {
            history_description("refresh", address)
        }
        CommandRoute::HistoryWithSourceRefresh(HistoryLookup::Retry(address)) => address
            .map_or_else(
                || "history:retry".into(),
                |address| history_description("retry", address),
            ),
        CommandRoute::Handoff(route) => match route {
            HandoffRoute::QueueSummary => "handoff:queue-summary".into(),
            HandoffRoute::BeginSummary {
                source:
                    ConversationAddress {
                        runtime_id,
                        conversation_id,
                    },
                target_runtime_id,
                summary_model,
                effort,
            } => format!(
                "handoff:begin-summary({runtime_id},{conversation_id},{target_runtime_id},{summary_model:?},{effort:?})"
            ),
            HandoffRoute::ReadSummary(work_id) => format!("handoff:read-summary({work_id})"),
            HandoffRoute::CancelSummary(work_id) => format!("handoff:cancel-summary({work_id})"),
            HandoffRoute::SendPrepared => "handoff:send-prepared".into(),
            HandoffRoute::ReadRequirements {
                source:
                    ConversationAddress {
                        runtime_id,
                        conversation_id,
                    },
                target_runtime_id,
                request_id,
            } => format!(
                "handoff:requirements({runtime_id},{conversation_id},{target_runtime_id},{request_id:?})"
            ),
        },
    }
}

fn history_description(kind: &str, address: ConversationAddress<'_>) -> String {
    let ConversationAddress {
        runtime_id,
        conversation_id,
    } = address;
    format!("history:{kind}({runtime_id},{conversation_id})")
}
