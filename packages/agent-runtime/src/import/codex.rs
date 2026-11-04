use super::*;

pub fn import_codex_conversation(
    execution: &ExecutionTarget,
    codex_home: &Path,
    cwd: &Path,
    context: &Context,
    cli_version: &str,
    provider: &str,
) -> Result<ImportedConversation> {
    let now = now_unix_ms();
    let thread_id = uuid7(now)?;
    let ids = context.tool_ids("call_")?;
    let mut records = Vec::new();
    let mut push = |kind: &str, payload: Value| {
        records.push(json!({"timestamp":iso_time(now),"ordinal":records.len(),"type":kind,"payload":payload}));
    };
    push(
        "session_meta",
        json!({"id":thread_id,"session_id":thread_id,"timestamp":iso_time(now),"cwd":cwd,
        "runtime_workspace_roots":[cwd],"originator":"velune","cli_version":cli_version,"source":"vscode",
        "model_provider":provider,"history_mode":"paginated"}),
    );
    let mut turn: Option<String> = None;
    for message in &context.messages {
        // A tool-result message continues the turn that issued its calls.
        let starts_turn = message.role == Role::User
            && message
                .content
                .iter()
                .any(|b| !matches!(b, Block::ToolResult { .. }));
        if starts_turn || turn.is_none() {
            if let Some(previous) = turn.take() {
                push(
                    "event_msg",
                    json!({"type":"task_complete","turn_id":previous,"root_turn_id":previous,"last_agent_message":"","started_at":now/1000,"completed_at":now/1000,"duration_ms":0,"time_to_first_token_ms":0}),
                );
            }
            let id = uuid7(now)?;
            push(
                "event_msg",
                json!({"type":"task_started","turn_id":id,"root_turn_id":id,"started_at":now/1000,"collaboration_mode_kind":"default"}),
            );
            turn = Some(id);
        }
        let turn_id = turn.as_ref().expect("import turn");
        let role = if message.role == Role::User {
            "user"
        } else {
            "assistant"
        };
        for block in &message.content {
            let (item, display) = match block {
                Block::Text { text } => {
                    let id = prefixed_id("msg_")?;
                    (
                        json!({"type":"message","id":id,"role":role,"content":[{"type":if role == "user" {"input_text"} else {"output_text"},"text":text}],"phase":"commentary"}),
                        Some(if role == "user" {
                            json!({"type":"UserMessage","id":id,"content":[{"type":"text","text":text,"text_elements":[]}]})
                        } else {
                            json!({"type":"AgentMessage","id":id,"content":[{"type":"Text","text":text}],"phase":"commentary"})
                        }),
                    )
                }
                Block::Image { media_type, data } => {
                    let url = format!("data:{media_type};base64,{data}");
                    let id = prefixed_id("msg_")?;
                    (
                        json!({"type":"message","id":id,"role":role,"content":[{"type":"input_image","image_url":url}]}),
                        Some(
                            json!({"type":"UserMessage","id":id,"content":[{"type":"image","image_url":url}]}),
                        ),
                    )
                }
                Block::ToolCall {
                    id,
                    name,
                    arguments,
                } => (
                    json!({"type":"function_call","id":prefixed_id("fc_")?,"name":tool_name(name),"arguments":arguments.to_string(),"call_id":ids[id.as_str()]}),
                    None,
                ),
                Block::ToolResult {
                    id,
                    content,
                    is_error,
                } => {
                    let output: Vec<Value> = content.iter().map(|b| match b {
                        Block::Text { text } => json!({"type":"input_text","text":text}),
                        Block::Image { media_type, data } => json!({"type":"input_image","image_url":format!("data:{media_type};base64,{data}")}),
                        _ => unreachable!("validated result content"),
                    }).collect();
                    let (name, arguments) = context
                        .messages
                        .iter()
                        .flat_map(|m| &m.content)
                        .find_map(|b| match b {
                            Block::ToolCall {
                                id: call,
                                name,
                                arguments,
                            } if call == id => Some((name, arguments)),
                            _ => None,
                        })
                        .expect("validated tool pair");
                    // Responses has no is_error flag. Keep it in the output text
                    // as well as the native display event.
                    let mut output = output;
                    if *is_error {
                        output.insert(
                            0,
                            json!({"type":"input_text","text":"Tool execution failed:"}),
                        );
                    }
                    (
                        json!({"type":"function_call_output","call_id":ids[id.as_str()],"output":output}),
                        Some(
                            json!({"type":"McpToolCall","id":ids[id.as_str()],"server":"velune-import","tool":tool_name(name),"arguments":arguments,
                        "status":if *is_error {"failed"} else {"completed"},"result":{"content":content.iter().filter_map(|b| match b {Block::Text {text}=>Some(json!({"type":"text","text":text})), _=>None}).collect::<Vec<_>>(),"isError":is_error}}),
                        ),
                    )
                }
            };
            push("response_item", item);
            if let Some(display) = display {
                push(
                    "event_msg",
                    json!({"type":"item_completed","thread_id":thread_id,"turn_id":turn_id,"item":display,"started_at_ms":now,"completed_at_ms":now}),
                );
            }
        }
    }
    if let Some(turn_id) = turn {
        push(
            "event_msg",
            json!({"type":"task_complete","turn_id":turn_id,"root_turn_id":turn_id,"last_agent_message":"","started_at":now/1000,"completed_at":now/1000,"duration_ms":0,"time_to_first_token_ms":0}),
        );
    }
    let date = OffsetDateTime::now_utc();
    let path = codex_home
        .join("sessions")
        .join(format!(
            "{:04}/{:02}/{:02}",
            date.year(),
            u8::from(date.month()),
            date.day()
        ))
        .join(codex_rollout_name(now, &thread_id));
    write_jsonl(execution, &path, &records)?;
    Ok(ImportedConversation::Codex { thread_id })
}
