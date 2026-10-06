use super::*;
use serde_json::{Map, json};

type Prepared = (Value, BTreeMap<String, ToolIdentity>, Vec<ConversionNote>);

pub(super) fn translate(
    from: LlmProtocol,
    to: LlmProtocol,
    body: &Value,
    limit: Option<u32>,
) -> Result<Prepared, TranslationError> {
    object(body, "request")?;
    let mut notes = vec![];
    let mut tools = BTreeMap::new();
    let mut messages = match from {
        LlmProtocol::ChatCompletions => array(&body["messages"], "messages")?.clone(),
        LlmProtocol::Responses => responses_messages(body, &mut notes)?,
        LlmProtocol::Messages => anthropic_messages(body, &mut notes)?,
    };
    let definitions = normalize_tools(from, body.get("tools"), &mut tools, &mut notes)?;
    for message in &mut messages {
        object(message, "message")?;
        if let Some(calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut) {
            for call in calls {
                let namespace = call
                    .get("namespace")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let function = call
                    .get_mut("function")
                    .ok_or_else(|| invalid("tool_call.function"))?;
                let name = string(&function["name"], "tool_call.name")?.to_owned();
                if let Some((mapped, _)) = tools.iter().find(|(_, identity)| {
                    identity.original == name
                        && identity.namespace.as_deref() == namespace.as_deref()
                }) {
                    function["name"] = json!(mapped);
                }
                string(&function["arguments"], "tool_call.arguments")?;
                if let Some(fields) = call.as_object_mut() {
                    fields.remove("namespace");
                }
            }
        }
    }
    let mut output = Map::new();
    output.insert(
        "model".into(),
        body.get("model").cloned().ok_or_else(|| invalid("model"))?,
    );
    for field in ["stream", "temperature", "top_p"] {
        if let Some(value) = body.get(field) {
            output.insert(field.into(), value.clone());
        }
    }
    let explicit = match from {
        LlmProtocol::ChatCompletions => body
            .get("max_completion_tokens")
            .or_else(|| body.get("max_tokens")),
        LlmProtocol::Responses => body.get("max_output_tokens"),
        LlmProtocol::Messages => body.get("max_tokens"),
    };
    let output_limit = match explicit {
        Some(value) => Some(value.as_u64().ok_or_else(|| invalid("output_limit"))?),
        None if to == LlmProtocol::Messages => limit.map(u64::from),
        None => None,
    };
    if let Some(limit) = output_limit {
        output.insert(
            match to {
                LlmProtocol::ChatCompletions => "max_completion_tokens",
                LlmProtocol::Responses => "max_output_tokens",
                LlmProtocol::Messages => "max_tokens",
            }
            .into(),
            json!(limit),
        );
    } else if to == LlmProtocol::Messages {
        return Err(invalid("max_tokens_required"));
    }
    if explicit.is_none() && to == LlmProtocol::Messages {
        note(&mut notes, "max_tokens", "model_limit_default");
    }
    let effort = match from {
        LlmProtocol::ChatCompletions => body.get("reasoning_effort"),
        LlmProtocol::Responses => body.get("reasoning").and_then(|v| v.get("effort")),
        LlmProtocol::Messages => None,
    };
    if let Some(effort) = effort {
        match to {
            LlmProtocol::ChatCompletions => {
                output.insert("reasoning_effort".into(), effort.clone());
            }
            LlmProtocol::Responses => {
                output.insert("reasoning".into(), json!({"effort": effort}));
            }
            LlmProtocol::Messages => note(&mut notes, "reasoning_effort", "not_expressible"),
        }
    }
    let stop = body.get(if from == LlmProtocol::Messages {
        "stop_sequences"
    } else {
        "stop"
    });
    if let Some(stop) = stop {
        if to == LlmProtocol::Messages {
            output.insert(
                "stop_sequences".into(),
                if stop.is_string() {
                    json!([stop])
                } else {
                    stop.clone()
                },
            );
        } else if to == LlmProtocol::ChatCompletions {
            output.insert("stop".into(), stop.clone());
        } else {
            note(&mut notes, "stop", "not_expressible");
        }
    }
    match to {
        LlmProtocol::ChatCompletions => {
            output.insert("messages".into(), json!(messages));
            if !definitions.is_empty() {
                output.insert("tools".into(), json!(definitions));
            }
        }
        LlmProtocol::Responses => {
            output.insert("input".into(), to_responses(&messages, &mut notes)?);
            output.insert("store".into(), json!(false));
            if !definitions.is_empty() {
                output.insert(
                    "tools".into(),
                    json!(
                        definitions
                            .iter()
                            .map(|v| {
                                let mut f = v["function"].clone();
                                f["type"] = json!("function");
                                f
                            })
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        LlmProtocol::Messages => {
            let (system, content) = to_anthropic(&messages, &mut notes)?;
            output.insert("messages".into(), content);
            if !system.is_empty() {
                output.insert("system".into(), json!(system));
            }
            if !definitions.is_empty() {
                output.insert("tools".into(),json!(definitions.iter().map(|v| json!({"name":v["function"]["name"],"description":v["function"].get("description").cloned().unwrap_or(json!("")),"input_schema":v["function"]["parameters"]})).collect::<Vec<_>>()));
            }
        }
    }
    if let Some(choice) = body.get("tool_choice")
        && let Some(choice) = translate_choice(from, to, choice, &tools, &mut notes)?
    {
        output.insert("tool_choice".into(), choice);
    }
    let parallel = body.get("parallel_tool_calls").cloned().or_else(|| {
        if from == LlmProtocol::Messages {
            body.get("tool_choice")
                .and_then(|v| v.get("disable_parallel_tool_use"))
                .and_then(Value::as_bool)
                .map(|v| json!(!v))
        } else {
            None
        }
    });
    if let Some(parallel) = parallel.as_ref() {
        if to == LlmProtocol::Messages {
            if let Some(choice) = output.get_mut("tool_choice").and_then(Value::as_object_mut) {
                choice.insert(
                    "disable_parallel_tool_use".into(),
                    json!(
                        !parallel
                            .as_bool()
                            .ok_or_else(|| invalid("parallel_tool_calls"))?
                    ),
                );
            } else {
                output.insert("tool_choice".into(),json!({"type":"auto","disable_parallel_tool_use":!parallel.as_bool().ok_or_else(|| invalid("parallel_tool_calls"))?}));
            }
        } else {
            output.insert("parallel_tool_calls".into(), parallel.clone());
        }
    }
    let handled = [
        "model",
        "messages",
        "input",
        "instructions",
        "system",
        "tools",
        "tool_choice",
        "stream",
        "temperature",
        "top_p",
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "reasoning_effort",
        "reasoning",
        "stop",
        "stop_sequences",
        "parallel_tool_calls",
    ];
    if object(body, "request")?
        .keys()
        .any(|key| !handled.contains(&key.as_str()))
    {
        note(&mut notes, "request.extensions", "dropped_unmapped_fields");
    }
    if body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        note(&mut notes, "previous_response_id", "history_not_expanded");
    }
    if body
        .get("reasoning")
        .is_some_and(|v| v.get("summary").is_some())
    {
        note(&mut notes, "reasoning.summary", "dropped_unmapped_fields");
    }
    if body.get("thinking").is_some() {
        note(&mut notes, "thinking", "not_expressible");
    }
    Ok((Value::Object(output), tools, notes))
}

pub(super) fn note(notes: &mut Vec<ConversionNote>, field: &'static str, reason: &'static str) {
    let value = ConversionNote {
        field,
        code: reason,
    };
    if !notes.contains(&value) {
        notes.push(value);
    }
}

fn responses_messages(
    body: &Value,
    notes: &mut Vec<ConversionNote>,
) -> Result<Vec<Value>, TranslationError> {
    let mut messages = vec![];
    if let Some(instructions) = body.get("instructions").filter(|v| !v.is_null()) {
        messages.push(json!({"role":"system","content":string(instructions,"instructions")?}));
    }
    if let Some(text) = body["input"].as_str() {
        messages.push(json!({"role":"user","content":text}));
        return Ok(messages);
    }
    for item in array(&body["input"], "input")? {
        match item["type"].as_str().unwrap_or("message") {
            "message" => { let role=string(&item["role"],"input.role")?; let content=normalize_content(item.get("content").unwrap_or(&Value::Null),LlmProtocol::Responses,notes)?; messages.push(json!({"role":role,"content":content})); },
            "function_call" => { let args=string(&item["arguments"],"function_call.arguments")?; messages.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":string(&item["call_id"],"function_call.call_id")?,"type":"function","namespace":item.get("namespace").cloned().unwrap_or(Value::Null),"function":{"name":string(&item["name"],"function_call.name")?,"arguments":args}}]})); },
            "custom_tool_call" => messages.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":string(&item["call_id"],"custom_tool_call.call_id")?,"type":"function","namespace":item.get("namespace").cloned().unwrap_or(Value::Null),"function":{"name":string(&item["name"],"custom_tool_call.name")?,"arguments":json!({"input":string(&item["input"],"custom_tool_call.input")?}).to_string()}}]})),
            "function_call_output"|"custom_tool_call_output" => messages.push(json!({"role":"tool","tool_call_id":string(&item["call_id"],"tool_result.call_id")?,"content":normalize_content(&item["output"],LlmProtocol::Responses,notes)?})),
            "reasoning" => { note(notes,"input.reasoning","opaque_reasoning_omitted"); },
            _ => note(notes,"input.item","unsupported_item_omitted"),
        }
    }
    Ok(messages)
}
fn anthropic_messages(
    body: &Value,
    notes: &mut Vec<ConversionNote>,
) -> Result<Vec<Value>, TranslationError> {
    let mut messages = vec![];
    if let Some(system) = body.get("system").filter(|v| !v.is_null()) {
        messages.push(json!({"role":"system","content":normalize_content(system,LlmProtocol::Messages,notes)?}));
    }
    for message in array(&body["messages"], "messages")? {
        let role = string(&message["role"], "messages.role")?;
        if message["content"].is_string() {
            messages.push(json!({"role":role,"content":message["content"]}));
            continue;
        }
        let mut text = vec![];
        for block in array(&message["content"], "messages.content")? {
            match block["type"].as_str() {
                Some("tool_use") => {
                    if !text.is_empty() {
                        messages.push(json!({"role":role,"content":std::mem::take(&mut text)}));
                    }
                    object(&block["input"], "tool_use.input")?;
                    messages.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":string(&block["id"],"tool_use.id")?,"type":"function","function":{"name":string(&block["name"],"tool_use.name")?,"arguments":block["input"].to_string()}}]}));
                }
                Some("tool_result") => {
                    if !text.is_empty() {
                        messages.push(json!({"role":role,"content":std::mem::take(&mut text)}));
                    }
                    if block["is_error"].as_bool() == Some(true) {
                        note(notes, "tool_result.is_error", "error_flag_omitted");
                    }
                    messages.push(json!({"role":"tool","tool_call_id":string(&block["tool_use_id"],"tool_result.id")?,"content":normalize_content(&block["content"],LlmProtocol::Messages,notes)?}));
                }
                _ => {
                    let parts = normalize_content(&json!([block]), LlmProtocol::Messages, notes)?;
                    if let Some(parts) = parts.as_array() {
                        text.extend(parts.clone());
                    }
                }
            }
        }
        if !text.is_empty() {
            messages.push(json!({"role":role,"content":text}));
        }
    }
    Ok(messages)
}
fn normalize_content(
    value: &Value,
    protocol: LlmProtocol,
    notes: &mut Vec<ConversionNote>,
) -> Result<Value, TranslationError> {
    if value.is_string() || value.is_null() {
        return Ok(value.clone());
    }
    let mut result = vec![];
    for part in array(value, "content")? {
        match part["type"].as_str() {
            Some("text"|"input_text"|"output_text") => result.push(json!({"type":"text","text":string(&part["text"],"content.text")?})),
            Some("image_url") => result.push(part.clone()),
            Some("input_image") => result.push(json!({"type":"image_url","image_url":{"url":string(&part["image_url"],"content.image_url")?}})),
            Some("image") if protocol==LlmProtocol::Messages => {
                let source=&part["source"];let url=match source["type"].as_str(){Some("url")=>string(&source["url"],"image.url")?.to_owned(),Some("base64")=>format!("data:{};base64,{}",string(&source["media_type"],"image.media_type")?,string(&source["data"],"image.data")?),_=>{note(notes,"content.image","unsupported_source_omitted");continue;}};
                result.push(json!({"type":"image_url","image_url":{"url":url}}));
            },
            Some("refusal") => { result.push(json!({"type":"text","text":string(&part["refusal"],"content.refusal")?})); note(notes,"content.refusal","rendered_as_text"); },
            Some("thinking"|"redacted_thinking") => note(notes,"content.thinking","opaque_reasoning_omitted"),
            _ => note(notes,"content.part","unsupported_part_omitted"),
        }
    }
    Ok(json!(result))
}
fn to_responses(
    messages: &[Value],
    notes: &mut Vec<ConversionNote>,
) -> Result<Value, TranslationError> {
    let mut items = vec![];
    for message in messages {
        let role = string(&message["role"], "message.role")?;
        if role == "tool" {
            let mut output =
                normalize_content(&message["content"], LlmProtocol::ChatCompletions, notes)?;
            if let Some(parts) = output.as_array_mut() {
                for part in parts {
                    if part["type"] == "text" {
                        part["type"] = json!("input_text");
                    } else if part["type"] == "image_url" {
                        *part = json!({"type":"input_image","image_url":part["image_url"]["url"]});
                    }
                }
            }
            items.push(json!({"type":"function_call_output","call_id":string(&message["tool_call_id"],"tool_result.call_id")?,"output":output}));
            continue;
        }
        if !message["content"].is_null() {
            let normalized =
                normalize_content(&message["content"], LlmProtocol::ChatCompletions, notes)?;
            let mut content = vec![];
            if let Some(text) = normalized.as_str() {
                content.push(json!({"type":if role=="assistant"{"output_text"}else{"input_text"},"text":text}));
            } else {
                for part in array(&normalized, "content")? {
                    match part["type"].as_str(){Some("text")=>content.push(json!({"type":if role=="assistant"{"output_text"}else{"input_text"},"text":part["text"]})),Some("image_url")=>content.push(json!({"type":"input_image","image_url":part["image_url"]["url"]})),_=>{}}
                }
            }
            items.push(json!({"type":"message","role":role,"content":content}));
        }
        if let Some(calls) = message.get("tool_calls") {
            for call in array(calls, "tool_calls")? {
                items.push(json!({"type":"function_call","call_id":call["id"],"name":call["function"]["name"],"arguments":call["function"]["arguments"]}));
            }
        }
        if message.get("reasoning_content").is_some() {
            note(notes, "message.reasoning_content", "reasoning_omitted");
        }
    }
    Ok(json!(items))
}
fn to_anthropic(
    messages: &[Value],
    notes: &mut Vec<ConversionNote>,
) -> Result<(Vec<Value>, Value), TranslationError> {
    let mut system = vec![];
    let mut result: Vec<Value> = vec![];
    for message in messages {
        let role = string(&message["role"], "message.role")?;
        let mut content = vec![];
        if role == "tool" {
            content.push(json!({"type":"tool_result","tool_use_id":message["tool_call_id"],"content":message["content"]}));
        } else if !message["content"].is_null() {
            let normalized =
                normalize_content(&message["content"], LlmProtocol::ChatCompletions, notes)?;
            if let Some(text) = normalized.as_str() {
                content.push(json!({"type":"text","text":text}));
            } else {
                for part in array(&normalized, "content")? {
                    match part["type"].as_str() {
                        Some("text") => content.push(part.clone()),
                        Some("image_url") => {
                            let url = string(&part["image_url"]["url"], "image.url")?;
                            let source = if let Some(data) = url.strip_prefix("data:") {
                                let (mime, data) = data
                                    .split_once(";base64,")
                                    .ok_or_else(|| invalid("image.data_url"))?;
                                json!({"type":"base64","media_type":mime,"data":data})
                            } else {
                                json!({"type":"url","url":url})
                            };
                            content.push(json!({"type":"image","source":source}));
                        }
                        _ => {}
                    }
                }
            }
        }
        if let Some(calls) = message.get("tool_calls") {
            for call in array(calls, "tool_calls")? {
                let raw = string(&call["function"]["arguments"], "tool_call.arguments")?;
                let (input, fallback) = response::arguments_object(raw);
                if fallback {
                    note(notes, "tool_call.arguments", "wrapped_raw_input");
                }
                content.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":input}));
            }
        }
        if role == "system" || role == "developer" {
            system.extend(content);
            if role == "developer" {
                note(notes, "developer", "merged_into_system");
            }
            continue;
        }
        if message.get("reasoning_content").is_some() {
            note(notes, "message.reasoning_content", "reasoning_omitted");
        }
        if content.is_empty() {
            continue;
        }
        let target_role = if role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        if let Some(previous) = result.last_mut().filter(|v| v["role"] == target_role) {
            previous["content"]
                .as_array_mut()
                .ok_or_else(|| invalid("messages.content"))?
                .extend(content);
        } else {
            result.push(json!({"role":target_role,"content":content}));
        }
    }
    Ok((system, json!(result)))
}
fn normalize_tools(
    from: LlmProtocol,
    value: Option<&Value>,
    identities: &mut BTreeMap<String, ToolIdentity>,
    notes: &mut Vec<ConversionNote>,
) -> Result<Vec<Value>, TranslationError> {
    let mut definitions = vec![];
    if let Some(value) = value {
        for tool in array(value, "tools")? {
            flatten_tool(from, tool, None, identities, &mut definitions, notes)?;
        }
    }
    Ok(definitions)
}
fn flatten_tool(
    from: LlmProtocol,
    tool: &Value,
    namespace: Option<&str>,
    identities: &mut BTreeMap<String, ToolIdentity>,
    definitions: &mut Vec<Value>,
    notes: &mut Vec<ConversionNote>,
) -> Result<(), TranslationError> {
    if tool["type"] == "namespace" {
        let name = string(&tool["name"], "tool.namespace")?;
        for nested in array(&tool["tools"], "tool.namespace.tools")? {
            flatten_tool(from, nested, Some(name), identities, definitions, notes)?;
        }
        note(notes, "tools.namespace", "flattened_request_scoped_names");
        return Ok(());
    }
    let source = if from == LlmProtocol::ChatCompletions {
        &tool["function"]
    } else {
        tool
    };
    let kind = tool["type"].as_str().unwrap_or("function");
    if !["function", "custom"].contains(&kind) {
        note(notes, "tools.hosted", "unsupported_tool_omitted");
        return Ok(());
    }
    let name = string(&source["name"], "tool.name")?;
    let mapped = if namespace.is_some() {
        format!("velune_tool_{}", identities.len())
    } else {
        name.to_owned()
    };
    if identities.contains_key(&mapped) {
        return Err(invalid("tools.duplicate_name"));
    }
    let custom = kind == "custom";
    identities.insert(
        mapped.clone(),
        ToolIdentity {
            original: name.to_owned(),
            namespace: namespace.map(str::to_owned),
            custom,
        },
    );
    let mut description = source["description"].as_str().unwrap_or("").to_owned();
    if custom && source.get("format").is_some() {
        description.push_str("\nOriginal input format (advisory; not enforced by this protocol): ");
        description.push_str(&source["format"].to_string());
        note(notes, "tools.custom.format", "grammar_advisory_only");
    }
    let parameters = if custom {
        json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false})
    } else {
        source
            .get(if from == LlmProtocol::Messages {
                "input_schema"
            } else {
                "parameters"
            })
            .cloned()
            .unwrap_or(json!({"type":"object","properties":{}}))
    };
    object(&parameters, "tool.parameters")?;
    definitions.push(json!({"type":"function","function":{"name":mapped,"description":description,"parameters":parameters}}));
    if source.get("strict").is_some() {
        note(notes, "tools.strict", "constraint_not_preserved");
    }
    Ok(())
}
fn translate_choice(
    from: LlmProtocol,
    to: LlmProtocol,
    value: &Value,
    tools: &BTreeMap<String, ToolIdentity>,
    notes: &mut Vec<ConversionNote>,
) -> Result<Option<Value>, TranslationError> {
    let kind = value
        .as_str()
        .or_else(|| value["type"].as_str())
        .ok_or_else(|| invalid("tool_choice"))?;
    let canonical = if kind == "any" { "required" } else { kind };
    if ["auto", "none", "required"].contains(&canonical) {
        return Ok(Some(if to == LlmProtocol::Messages {
            json!({"type":if canonical=="required"{"any"}else{canonical}})
        } else {
            json!(canonical)
        }));
    }
    let name = if from == LlmProtocol::ChatCompletions {
        value["function"]["name"].as_str()
    } else {
        value["name"].as_str()
    };
    if let Some(name) = name {
        let mapped = tools
            .iter()
            .find(|(_, identity)| {
                identity.original == name
                    && identity.namespace.as_deref()
                        == value.get("namespace").and_then(Value::as_str)
            })
            .map(|(key, _)| key.as_str())
            .unwrap_or(name);
        return Ok(Some(match to {
            LlmProtocol::ChatCompletions => json!({"type":"function","function":{"name":mapped}}),
            LlmProtocol::Responses => json!({"type":"function","name":mapped}),
            LlmProtocol::Messages => json!({"type":"tool","name":mapped}),
        }));
    }
    note(notes, "tool_choice", "unsupported_choice_omitted");
    Ok(None)
}
