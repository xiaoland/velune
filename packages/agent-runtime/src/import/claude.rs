use super::*;
use std::collections::HashMap;

fn content(blocks: &[Block], ids: &HashMap<&str, String>) -> Vec<Value> {
    blocks.iter().map(|block| match block {
        Block::Text { text } => json!({"type":"text","text":text}),
        Block::Image { media_type, data } => json!({"type":"image","source":{"type":"base64","media_type":media_type,"data":data}}),
        Block::ToolCall { id, name, arguments } => json!({"type":"tool_use","id":ids[id.as_str()],"name":tool_name(name),"input":arguments}),
        Block::ToolResult { id, content: result, is_error } => json!({"type":"tool_result","tool_use_id":ids[id.as_str()],"content":content(result, ids),"is_error":is_error}),
    }).collect()
}

pub fn import_claude_conversation(
    execution: &ExecutionTarget,
    config_dir: &Path,
    cwd: &Path,
    context: &Context,
    cli_version: &str,
) -> Result<ImportedConversation> {
    import_claude_conversation_with_origins(execution, config_dir, cwd, context, cli_version)
        .map(|(imported, _)| imported)
}

pub fn import_claude_conversation_with_origins(
    execution: &ExecutionTarget,
    config_dir: &Path,
    cwd: &Path,
    context: &Context,
    cli_version: &str,
) -> Result<(
    ImportedConversation,
    std::collections::BTreeMap<String, Vec<String>>,
)> {
    import_claude_conversation_owned(execution, config_dir, cwd, context, cli_version)
        .map(|(imported, origins, _)| (imported, origins))
}

pub fn import_claude_conversation_owned(
    execution: &ExecutionTarget,
    config_dir: &Path,
    cwd: &Path,
    context: &Context,
    cli_version: &str,
) -> Result<OwnedImport> {
    let mut origins = std::collections::BTreeMap::<String, Vec<String>>::new();
    let session_id = uuid4()?;
    let ids = context.tool_ids("toolu_")?;
    let mut records = Vec::new();
    let mut parent = None;
    let now = now_unix_ms();
    if context.messages.is_empty() {
        // A native non-message record gives history discovery a cwd/session
        // identity. No fabricated human prompt enters the model context.
        records.push(json!({"type":"queue-operation","operation":"dequeue",
            "sessionId":session_id,"cwd":cwd,"timestamp":iso_time(now),
            "veluneEmptyImport":true}));
    }
    for (ordinal, message) in context.messages.iter().enumerate() {
        let uuid = uuid4()?;
        if message.role == Role::User
            && let Some(source) = &message.source_message_id
        {
            origins
                .entry(source.clone())
                .or_default()
                .push(uuid.clone());
        }
        let role = if message.role == Role::User {
            "user"
        } else {
            "assistant"
        };
        let mut native = json!({"role":role,"content":content(&message.content, &ids)});
        if message.role == Role::Assistant {
            native["id"] = json!(prefixed_id("msg_")?);
            native["type"] = json!("message");
            native["stop_reason"] = json!(if message
                .content
                .iter()
                .any(|b| matches!(b, Block::ToolCall { .. }))
            {
                "tool_use"
            } else {
                "end_turn"
            });
        }
        records.push(
            json!({"type":role,"uuid":uuid,"parentUuid":parent,"isSidechain":false,
            "timestamp":iso_time(now + ordinal as i64),"cwd":cwd,"sessionId":session_id,
            "version":cli_version,"userType":"external","entrypoint":"velune","message":native}),
        );
        parent = Some(uuid);
    }
    let path = claude_project_directory(config_dir, cwd).join(format!("{session_id}.jsonl"));
    write_jsonl(execution, &path, &records)?;
    let imported = ImportedConversation::ClaudeCode { session_id };
    let ownership = ImportOwnership::capture(execution, path, imported.clone())?;
    Ok((imported, origins, ownership))
}
