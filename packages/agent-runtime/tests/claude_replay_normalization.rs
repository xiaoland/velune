//! Synthetic Claude 2.1.292 replay shapes through the production source exporter
//! and generic context validator. No CLI, model, or user session is involved.
use serde_json::{Value, json};
use velune_agent_runtime::{
    creation::Retained,
    execution::ExecutionTarget,
    history::{HistoryConfig, export_context},
    import::{Block, Context, Role},
    version::RuntimeKind,
};
use velune_test_support::TempRoot;

const SESSION: &str = "00000000-0000-4000-8000-000000000001";

struct Fixture(TempRoot);

impl Fixture {
    fn new() -> Self {
        Self(TempRoot::new("claude-replay-normalization"))
    }

    fn config(&self) -> HistoryConfig {
        HistoryConfig {
            kind: RuntimeKind::ClaudeCode,
            execution: ExecutionTarget::Local,
            os_home: Some(self.0.to_path_buf()),
            binary: self.0.join("unused-cli"),
            node_binary: self.0.join("unused-node"),
            resources_directory: self.0.to_path_buf(),
            root: self.0.to_path_buf(),
            home: self.0.to_path_buf(),
        }
    }

    fn save(&self, records: &[Value]) {
        std::fs::write(
            self.0.join(format!("{SESSION}.jsonl")),
            records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }

    fn export(&self, records: &[Value]) -> Retained<Context> {
        self.save(records);
        export_context(&self.config(), SESSION).unwrap()
    }
}

fn user(id: &str, parent: Option<&str>, content: Value) -> Value {
    json!({"type":"user","uuid":id,"parentUuid":parent,"message":{"role":"user","content":content}})
}

fn assistant(id: &str, parent: &str, message_id: &str, content: Value, stop_reason: &str) -> Value {
    json!({"type":"assistant","uuid":id,"parentUuid":parent,
        "message":{"id":message_id,"role":"assistant","model":"synthetic-model",
            "content":content,"stop_reason":stop_reason}})
}

fn tool_calls(context: &Context) -> Vec<String> {
    context
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| {
            if let Block::ToolCall { id, .. } = block {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect()
}

fn tool_results(context: &Context) -> Vec<(String, bool, Vec<String>)> {
    context
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| {
            if let Block::ToolResult {
                id,
                is_error,
                content,
            } = block
            {
                let text = content
                    .iter()
                    .filter_map(|item| {
                        if let Block::Text { text } = item {
                            Some(text.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                Some((id.clone(), *is_error, text))
            } else {
                None
            }
        })
        .collect()
}

fn text(context: &Context) -> Vec<(Role, String)> {
    context
        .messages
        .iter()
        .flat_map(|message| {
            message.content.iter().filter_map(move |block| {
                if let Block::Text { text } = block {
                    Some((message.role, text.clone()))
                } else {
                    None
                }
            })
        })
        .collect()
}

#[test]
fn same_api_message_chunks_keep_parallel_calls_and_results_complete() {
    let f = Fixture::new();
    let records = [
        user(
            "u",
            None,
            json!([{"type":"text","text":"SYNTHETIC_PROMPT"}]),
        ),
        assistant(
            "a1",
            "u",
            "api-shared",
            json!([
                {"type":"text","text":"SYNTHETIC_PREFIX"},
                {"type":"tool_use","id":"toolu_a","name":"Read","input":{"path":"SYNTHETIC_A"}},
                {"type":"tool_use","id":"toolu_b","name":"Read","input":{"path":"SYNTHETIC_B"}}
            ]),
            "tool_use",
        ),
        user(
            "r-a",
            Some("a1"),
            json!([{
                "type":"tool_result","tool_use_id":"toolu_a","content":"SYNTHETIC_RESULT_A","is_error":false
            }]),
        ),
        assistant(
            "a2",
            "r-a",
            "api-shared",
            json!([
                {"type":"text","text":"SYNTHETIC_CHUNK_WHILE_B_PENDING"}
            ]),
            "tool_use",
        ),
        user(
            "r-b",
            Some("a2"),
            json!([{
                "type":"tool_result","tool_use_id":"toolu_b","content":"SYNTHETIC_RESULT_B","is_error":false
            }]),
        ),
        assistant(
            "answer",
            "r-b",
            "api-final",
            json!([
                {"type":"text","text":"SYNTHETIC_FINAL_ANSWER"}
            ]),
            "end_turn",
        ),
    ];

    let context = f.export(&records);
    assert_eq!(
        context.messages.len(),
        4,
        "SDK replay coalesces same-id chunks across tool results"
    );
    assert_eq!(context.messages[0].role, Role::User);
    assert_eq!(context.messages[1].role, Role::Assistant);
    assert_eq!(context.messages[2].role, Role::User);
    assert_eq!(context.messages[3].role, Role::Assistant);
    assert_eq!(
        tool_calls(&context),
        ["toolu_a".to_owned(), "toolu_b".to_owned()],
        "both calls from the first API message survive"
    );
    assert_eq!(
        tool_results(&context),
        [
            ("toolu_a".into(), false, vec!["SYNTHETIC_RESULT_A".into()]),
            ("toolu_b".into(), false, vec!["SYNTHETIC_RESULT_B".into()]),
        ],
        "the result blocks are emitted together in call-id order"
    );
    let assistant_blocks: Vec<_> = context.messages[1]
        .content
        .iter()
        .map(|block| match block {
            Block::Text { text } => format!("text:{text}"),
            Block::ToolCall { id, .. } => format!("tool_call:{id}"),
            other => panic!("unexpected block in SDK-coalesced assistant message: {other:?}"),
        })
        .collect();
    assert_eq!(
        assistant_blocks,
        [
            "text:SYNTHETIC_PREFIX",
            "text:SYNTHETIC_CHUNK_WHILE_B_PENDING",
            "tool_call:toolu_a",
            "tool_call:toolu_b",
        ]
    );
    assert!(text(&context).contains(&(Role::Assistant, "SYNTHETIC_FINAL_ANSWER".into())));
}

#[test]
fn explicit_interruption_drops_only_its_response_and_refusal_keeps_completed_pair() {
    let interrupted = Fixture::new();
    let records = [
        user(
            "u",
            None,
            json!([{"type":"text","text":"SYNTHETIC_USER_PROMPT"}]),
        ),
        assistant(
            "partial",
            "u",
            "api-interrupted",
            json!([
                {"type":"text","text":"SYNTHETIC_PARTIAL_ASSISTANT_TEXT"},
                {"type":"tool_use","id":"toolu_interrupted","name":"Read","input":{"path":"SYNTHETIC_PATH"}}
            ]),
            "tool_use",
        ),
        {
            let mut record = user(
                "interrupt",
                Some("partial"),
                json!([
                    {"type":"text","text":"[Request interrupted by user]"}
                ]),
            );
            record["promptId"] = json!("synthetic-prompt");
            record["interruptedMessageId"] = json!("api-interrupted");
            record
        },
        assistant(
            "retry-answer",
            "interrupt",
            "api-retry",
            json!([
                {"type":"text","text":"SYNTHETIC_RETRY_ANSWER"}
            ]),
            "end_turn",
        ),
    ];
    let context = interrupted.export(&records);
    assert!(
        tool_calls(&context).is_empty(),
        "only the explicitly interrupted API response is omitted"
    );
    assert!(
        tool_results(&context).is_empty(),
        "interruption does not invent a cancellation result"
    );
    let texts = text(&context);
    assert!(texts.contains(&(Role::User, "SYNTHETIC_USER_PROMPT".into())));
    assert!(texts.contains(&(Role::User, "[Request interrupted by user]".into())));
    assert!(texts.contains(&(Role::Assistant, "SYNTHETIC_RETRY_ANSWER".into())));
    assert!(
        !texts
            .iter()
            .any(|(_, text)| text == "SYNTHETIC_PARTIAL_ASSISTANT_TEXT")
    );

    let refusal = Fixture::new();
    let records = [
        user(
            "u",
            None,
            json!([{"type":"text","text":"SYNTHETIC_REFUSAL_PROMPT"}]),
        ),
        assistant(
            "call",
            "u",
            "api-refused",
            json!([
                {"type":"tool_use","id":"toolu_complete","name":"Read","input":{"path":"SYNTHETIC_COMPLETE_PATH"}}
            ]),
            "tool_use",
        ),
        user(
            "result",
            Some("call"),
            json!([{
                "type":"tool_result","tool_use_id":"toolu_complete","content":"SYNTHETIC_COMPLETED_RESULT","is_error":false
            }]),
        ),
        {
            let mut record = user(
                "refusal-marker",
                Some("result"),
                json!([
                    {"type":"text","text":"SYNTHETIC_SAFETY_MARKER"}
                ]),
            );
            record["isMeta"] = json!(true);
            record["refusedMessageId"] = json!("api-refused");
            record
        },
        assistant(
            "answer",
            "refusal-marker",
            "api-recovered",
            json!([
                {"type":"text","text":"SYNTHETIC_RECOVERED_ANSWER"}
            ]),
            "end_turn",
        ),
    ];
    let context = refusal.export(&records);
    assert_eq!(tool_calls(&context), ["toolu_complete"]);
    assert_eq!(
        tool_results(&context),
        [(
            "toolu_complete".into(),
            false,
            vec!["SYNTHETIC_COMPLETED_RESULT".into()]
        )]
    );
    let texts = text(&context);
    assert!(texts.contains(&(Role::User, "SYNTHETIC_SAFETY_MARKER".into())));
    assert!(texts.contains(&(Role::Assistant, "SYNTHETIC_RECOVERED_ANSWER".into())));
}

#[test]
fn ordinary_user_text_matching_interrupt_marker_is_not_a_cancellation() {
    let f = Fixture::new();
    let records = [
        user(
            "u",
            None,
            json!([{"type":"text","text":"SYNTHETIC_PROMPT"}]),
        ),
        assistant(
            "call",
            "u",
            "api-complete",
            json!([
                {"type":"tool_use","id":"toolu_complete","name":"Read","input":{"path":"SYNTHETIC_PATH"}}
            ]),
            "tool_use",
        ),
        user(
            "result",
            Some("call"),
            json!([{
                "type":"tool_result","tool_use_id":"toolu_complete","content":"SYNTHETIC_RESULT","is_error":false
            }]),
        ),
        {
            let mut record = user(
                "same-text-user-prompt",
                Some("result"),
                json!([
                    {"type":"text","text":"[Request interrupted by user]"}
                ]),
            );
            record["promptId"] = json!("different-prompt");
            record["promptSource"] = json!("human");
            record["turnPosition"] = json!(1);
            record
        },
        assistant(
            "answer",
            "same-text-user-prompt",
            "api-answer",
            json!([
                {"type":"text","text":"SYNTHETIC_ANSWER"}
            ]),
            "end_turn",
        ),
    ];

    let context = f.export(&records);
    assert_eq!(tool_calls(&context), ["toolu_complete"]);
    assert_eq!(tool_results(&context).len(), 1);
    let texts = text(&context);
    assert!(texts.contains(&(Role::User, "[Request interrupted by user]".into())));
    assert!(texts.contains(&(Role::Assistant, "SYNTHETIC_ANSWER".into())));
}
