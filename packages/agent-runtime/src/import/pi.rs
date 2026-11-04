use super::*;

fn content(blocks: &[Block]) -> Vec<Value> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Text { text } => json!({"type":"text","text":text}),
            Block::Image { media_type, data } => {
                json!({"type":"image","mimeType":media_type,"data":data})
            }
            _ => unreachable!("validated result content"),
        })
        .collect()
}
pub fn import_pi_conversation(
    execution: &ExecutionTarget,
    agent_dir: &Path,
    session_dir: Option<&Path>,
    cwd: &Path,
    context: &Context,
) -> Result<ImportedConversation> {
    let session_id = uuid4()?;
    let now = now_unix_ms();
    let ids = context.tool_ids("tool_")?;
    let names: std::collections::HashMap<_, _> = context
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolCall { id, name, .. } => Some((id.as_str(), tool_name(name))),
            _ => None,
        })
        .collect();
    let mut records = vec![
        json!({"type":"session","version":3,"id":session_id,"timestamp":iso_time(now),"cwd":cwd}),
    ];
    let mut parent = None;
    for message in &context.messages {
        let mut native = Vec::new();
        let mut ordinary = Vec::new();
        // Results precede the next user text. Preserve their order and images.
        for block in &message.content {
            match block {
                Block::ToolResult { id, content: output, is_error } => {
                    if !ordinary.is_empty() {
                        native.push(json!({"role":"user","content":std::mem::take(&mut ordinary)}));
                    }
                    native.push(json!({"role":"toolResult","toolCallId":ids[id.as_str()],"toolName":names[id.as_str()],"content":content(output),"isError":is_error}));
                }
                Block::ToolCall { id, name, arguments } => ordinary.push(json!({"type":"toolCall","id":ids[id.as_str()],"name":tool_name(name),"arguments":arguments})),
                Block::Text { text } => ordinary.push(json!({"type":"text","text":text})),
                Block::Image { media_type, data } => ordinary.push(json!({"type":"image","mimeType":media_type,"data":data})),
            }
        }
        if !ordinary.is_empty() {
            let role = if message.role == Role::User {
                "user"
            } else {
                "assistant"
            };
            let mut entry = json!({"role":role,"content":ordinary});
            if message.role == Role::Assistant {
                entry["api"] = json!("openai-completions");
                entry["provider"] = json!("velune-import");
                entry["model"] = json!("unknown");
                entry["stopReason"] = json!(if message
                    .content
                    .iter()
                    .any(|b| matches!(b, Block::ToolCall { .. }))
                {
                    "toolUse"
                } else {
                    "stop"
                });
                entry["usage"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}});
            }
            native.push(entry);
        }
        for mut entry in native {
            let timestamp = now + records.len() as i64;
            entry["timestamp"] = json!(timestamp);
            let id = hex_id()?;
            records.push(json!({"type":"message","id":id,"parentId":parent,"timestamp":iso_time(timestamp),"message":entry}));
            parent = Some(id);
        }
    }
    let root = session_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| agent_dir.join("sessions"));
    let directory = if session_dir.is_some() {
        root
    } else {
        pi_session_directory(&root, cwd)
    };
    let path = directory.join(format!(
        "{}_{session_id}.jsonl",
        iso_time(now).replace([':', '.'], "-")
    ));
    write_jsonl(execution, &path, &records)?;
    Ok(ImportedConversation::Pi { path })
}
