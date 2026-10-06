use super::*;
use serde_json::json;

#[derive(Default)]
pub(super) struct Output {
    pub id: String,
    pub model: String,
    pub created: u64,
    pub text: String,
    pub refusal: String,
    pub reasoning: String,
    pub tools: Vec<Call>,
    pub usage: Value,
    pub finish: String,
}
#[derive(Default)]
pub(super) struct Call {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub(super) fn translate(
    from: LlmProtocol,
    to: LlmProtocol,
    body: &Value,
    tools: &BTreeMap<String, ToolIdentity>,
    notes: &mut Vec<ConversionNote>,
) -> Result<Value, TranslationError> {
    object(body, "response")?;
    if body.get("usage").is_none_or(Value::is_null) {
        request::note(notes, "usage", "unreported");
    }
    request::note(notes, "response.extensions", "unmapped_fields_omitted");
    if let Some(error) = body.get("error").filter(|v| !v.is_null()) {
        return Ok(error_body(to, error));
    }
    if body["id"].as_str().is_none_or(str::is_empty) {
        request::note(notes, "response.id", "local_conversion_identity");
    }
    let mut output = Output {
        id: body["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(local_response_id),
        model: body["model"].as_str().unwrap_or("").to_owned(),
        created: body["created"]
            .as_u64()
            .or_else(|| body["created_at"].as_u64())
            .unwrap_or(0),
        ..Output::default()
    };
    match from {
        LlmProtocol::ChatCompletions => {
            let choices = array(&body["choices"], "choices")?;
            if choices.len() > 1 {
                request::note(notes, "choices", "first_choice_only");
            }
            let choice = choices.first().ok_or_else(|| invalid("choices"))?;
            let message = &choice["message"];
            output.text = text_content(&message["content"])?;
            output.refusal = message["refusal"].as_str().unwrap_or("").to_owned();
            output.reasoning = message["reasoning_content"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            if let Some(calls) = message.get("tool_calls") {
                for call in array(calls, "tool_calls")? {
                    output.tools.push(Call {
                        id: string(&call["id"], "tool_call.id")?.to_owned(),
                        name: string(&call["function"]["name"], "tool_call.name")?.to_owned(),
                        arguments: string(&call["function"]["arguments"], "tool_call.arguments")?
                            .to_owned(),
                    });
                }
            }
            output.finish = string(&choice["finish_reason"], "finish_reason")?.to_owned();
            output.usage = normalize_usage(from, body.get("usage").unwrap_or(&Value::Null));
        }
        LlmProtocol::Responses => {
            for item in array(&body["output"], "output")? {
                match item["type"].as_str() {
                    Some("message") => {
                        for part in array(&item["content"], "output.content")? {
                            match part["type"].as_str() {
                                Some("output_text") => {
                                    output.text.push_str(string(&part["text"], "output.text")?)
                                }
                                Some("refusal") => output
                                    .refusal
                                    .push_str(string(&part["refusal"], "output.refusal")?),
                                _ => {}
                            }
                        }
                    }
                    Some("function_call") => output.tools.push(Call {
                        id: string(&item["call_id"], "function_call.call_id")?.to_owned(),
                        name: string(&item["name"], "function_call.name")?.to_owned(),
                        arguments: string(&item["arguments"], "function_call.arguments")?
                            .to_owned(),
                    }),
                    Some("custom_tool_call") => output.tools.push(Call {
                        id: string(&item["call_id"], "custom_tool_call.call_id")?.to_owned(),
                        name: string(&item["name"], "custom_tool_call.name")?.to_owned(),
                        arguments:
                            json!({"input":string(&item["input"],"custom_tool_call.input")?})
                                .to_string(),
                    }),
                    Some("reasoning") => {
                        if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                            for part in summary {
                                if let Some(text) = part["text"].as_str() {
                                    output.reasoning.push_str(text);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            output.finish=match body["status"].as_str(){Some("completed")=>if output.tools.is_empty(){"stop"}else{"tool_calls"},Some("incomplete")=>match body["incomplete_details"]["reason"].as_str(){Some("content_filter")=>"content_filter",_=>"length"},Some("failed"|"cancelled")=>return Ok(error_body(to,&json!({"type":"upstream_error","message":"upstream response did not complete"}))),_=>return Err(invalid("response.status"))}.into();
            output.usage = normalize_usage(from, body.get("usage").unwrap_or(&Value::Null));
        }
        LlmProtocol::Messages => {
            for part in array(&body["content"], "content")? {
                match part["type"].as_str() {
                    Some("text") => output.text.push_str(string(&part["text"], "content.text")?),
                    Some("tool_use") => {
                        object(&part["input"], "tool_use.input")?;
                        output.tools.push(Call {
                            id: string(&part["id"], "tool_use.id")?.into(),
                            name: string(&part["name"], "tool_use.name")?.into(),
                            arguments: part["input"].to_string(),
                        });
                    }
                    Some("thinking") => {
                        if let Some(text) = part["thinking"].as_str() {
                            output.reasoning.push_str(text);
                        }
                    }
                    _ => {}
                }
            }
            output.finish = match body["stop_reason"].as_str() {
                Some("end_turn" | "stop_sequence") => "stop",
                Some("max_tokens" | "model_context_window_exceeded") => "length",
                Some("tool_use") => "tool_calls",
                Some("refusal") => "content_filter",
                Some(_) => "stop",
                None => return Err(invalid("stop_reason")),
            }
            .into();
            output.usage = normalize_usage(from, body.get("usage").unwrap_or(&Value::Null));
        }
    }
    if !output.reasoning.is_empty() {
        request::note(notes, "reasoning", "best_effort_text");
    }
    if output.usage.get("cached_tokens").is_some() {
        request::note(notes, "usage.cache", "protocol_accounting_projection");
    }
    for call in &output.tools {
        if to == LlmProtocol::Messages && arguments_object(&call.arguments).1 {
            request::note(notes, "tool_call.arguments", "wrapped_raw_input");
        }
        if tools
            .get(&call.name)
            .is_some_and(|identity| identity.custom)
            && custom_input(&call.arguments)?.1
        {
            request::note(notes, "custom_tool.input", "wrapper_mismatch_raw_arguments");
        }
    }
    if to == LlmProtocol::Messages && usage_has_unknown_counts(&output.usage) {
        request::note(notes, "usage", "required_numeric_placeholder");
    }
    encode(to, &output, tools)
}

pub(super) fn text_content(value: &Value) -> Result<String, TranslationError> {
    if value.is_null() {
        return Ok(String::new());
    }
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let mut text = String::new();
    for part in array(value, "content")? {
        if let Some(value) = part.get("text").and_then(Value::as_str) {
            text.push_str(value);
        }
    }
    Ok(text)
}
pub(super) fn normalize_usage(from: LlmProtocol, value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    let (input, output) = if from == LlmProtocol::ChatCompletions {
        ("prompt_tokens", "completion_tokens")
    } else {
        ("input_tokens", "output_tokens")
    };
    let mut result = serde_json::Map::new();
    if let Some(value) = value.get(input) {
        result.insert("input_tokens".into(), value.clone());
    }
    if let Some(value) = value.get(output) {
        result.insert("output_tokens".into(), value.clone());
    }
    let cached = match from {
        LlmProtocol::ChatCompletions => value["prompt_tokens_details"]["cached_tokens"].clone(),
        LlmProtocol::Responses => value["input_tokens_details"]["cached_tokens"].clone(),
        LlmProtocol::Messages => value["cache_read_input_tokens"].clone(),
    };
    if !cached.is_null() {
        result.insert("cached_tokens".into(), cached);
    }
    let reasoning = match from {
        LlmProtocol::ChatCompletions => {
            value["completion_tokens_details"]["reasoning_tokens"].clone()
        }
        LlmProtocol::Responses => value["output_tokens_details"]["reasoning_tokens"].clone(),
        LlmProtocol::Messages => Value::Null,
    };
    if !reasoning.is_null() {
        result.insert("reasoning_tokens".into(), reasoning);
    }
    if let Some(value) = value.get("cache_creation_input_tokens") {
        result.insert("cache_creation_input_tokens".into(), value.clone());
    }
    if from == LlmProtocol::Messages
        && let Some(input) = result.get("input_tokens").and_then(Value::as_u64)
    {
        let cached = result
            .get("cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let created = result
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        result.insert(
            "input_tokens".into(),
            json!(input.saturating_add(cached).saturating_add(created)),
        );
    }
    Value::Object(result)
}
pub(super) fn encode_usage(to: LlmProtocol, value: &Value) -> Value {
    if value.is_null() && to != LlmProtocol::Messages {
        return Value::Null;
    }
    let input = value["input_tokens"].as_u64();
    let output = value["output_tokens"].as_u64();
    match to {
        LlmProtocol::ChatCompletions => {
            let mut usage = json!({});
            if let Some(input) = input {
                usage["prompt_tokens"] = json!(input);
            }
            if let Some(output) = output {
                usage["completion_tokens"] = json!(output);
            }
            if let (Some(input), Some(output)) = (input, output) {
                usage["total_tokens"] = json!(input.saturating_add(output));
            }
            if let Some(cached) = value.get("cached_tokens") {
                usage["prompt_tokens_details"] = json!({"cached_tokens":cached});
            }
            if let Some(reasoning) = value.get("reasoning_tokens") {
                usage["completion_tokens_details"] = json!({"reasoning_tokens":reasoning});
            }
            usage
        }
        LlmProtocol::Responses => {
            let mut usage = json!({});
            if let Some(input) = input {
                usage["input_tokens"] = json!(input);
            }
            if let Some(output) = output {
                usage["output_tokens"] = json!(output);
            }
            if let (Some(input), Some(output)) = (input, output) {
                usage["total_tokens"] = json!(input.saturating_add(output));
            }
            if let Some(cached) = value.get("cached_tokens") {
                usage["input_tokens_details"] = json!({"cached_tokens":cached});
            }
            if let Some(reasoning) = value.get("reasoning_tokens") {
                usage["output_tokens_details"] = json!({"reasoning_tokens":reasoning});
            }
            usage
        }
        LlmProtocol::Messages => {
            let base = input
                .unwrap_or(0)
                .saturating_sub(value["cached_tokens"].as_u64().unwrap_or(0))
                .saturating_sub(value["cache_creation_input_tokens"].as_u64().unwrap_or(0));
            let mut usage = json!({"input_tokens":base,"output_tokens":output.unwrap_or(0)});
            if let Some(cached) = value.get("cached_tokens") {
                usage["cache_read_input_tokens"] = cached.clone();
            }
            if let Some(created) = value.get("cache_creation_input_tokens") {
                usage["cache_creation_input_tokens"] = created.clone();
            }
            usage
        }
    }
}
pub(super) fn encode(
    to: LlmProtocol,
    output: &Output,
    tools: &BTreeMap<String, ToolIdentity>,
) -> Result<Value, TranslationError> {
    let mut calls = vec![];
    let mut items = vec![];
    let mut blocks = vec![];
    if !output.text.is_empty() {
        blocks.push(json!({"type":"text","text":output.text}));
    }
    if !output.refusal.is_empty() {
        blocks.push(json!({"type":"text","text":output.refusal}));
    }
    let mut parts = vec![];
    if !output.text.is_empty() {
        parts.push(json!({"type":"output_text","text":output.text,"annotations":[]}));
    }
    if !output.refusal.is_empty() {
        parts.push(json!({"type":"refusal","refusal":output.refusal}));
    }
    if !parts.is_empty() {
        items.push(json!({"type":"message","id":format!("{}_message",output.id),"role":"assistant","status":"completed","content":parts}));
    }
    // Full thinking and reasoning summaries are not interchangeable; retain the
    // available text as an explicit best-effort output, never opaque signatures.
    if !output.reasoning.is_empty() {
        match to{LlmProtocol::Responses=>items.push(json!({"type":"reasoning","id":format!("{}_reasoning",output.id),"summary":[{"type":"summary_text","text":output.reasoning}]})),LlmProtocol::Messages=>blocks.push(json!({"type":"text","text":output.reasoning})),LlmProtocol::ChatCompletions=>{}}
    }
    for call in &output.tools {
        let (parsed, _) = arguments_object(&call.arguments);
        let identity = tools.get(&call.name);
        let name = identity.map(|v| v.original.as_str()).unwrap_or(&call.name);
        calls.push(json!({"id":call.id,"type":"function","function":{"name":name,"arguments":call.arguments}}));
        blocks.push(json!({"type":"tool_use","id":call.id,"name":name,"input":parsed}));
        let mut item = if identity.is_some_and(|v| v.custom) {
            json!({"type":"custom_tool_call","id":format!("{}_{}_item",output.id,call.id),"call_id":call.id,"name":name,"input":custom_input(&call.arguments)?.0,"status":"completed"})
        } else {
            json!({"type":"function_call","id":format!("{}_{}_item",output.id,call.id),"call_id":call.id,"name":name,"arguments":call.arguments,"status":"completed"})
        };
        if let Some(namespace) = identity.and_then(|v| v.namespace.as_ref()) {
            item["namespace"] = json!(namespace);
        }
        items.push(item);
    }
    let usage = encode_usage(to, &output.usage);
    Ok(match to {
        LlmProtocol::ChatCompletions => {
            let mut message = json!({"role":"assistant","content":if output.text.is_empty(){Value::Null}else{json!(output.text)}});
            if !calls.is_empty() {
                message["tool_calls"] = json!(calls);
            }
            if !output.refusal.is_empty() {
                message["refusal"] = json!(output.refusal);
            }
            if !output.reasoning.is_empty() {
                message["reasoning_content"] = json!(output.reasoning);
            }
            json!({"id":output.id,"object":"chat.completion","created":output.created,"model":output.model,"choices":[{"index":0,"message":message,"finish_reason":output.finish}],"usage":usage})
        }
        LlmProtocol::Responses => {
            json!({"id":output.id,"object":"response","created_at":output.created,"model":output.model,"status":if output.finish=="length"||output.finish=="content_filter"{"incomplete"}else{"completed"},"output":items,"usage":usage,"error":null,"incomplete_details":if output.finish=="length"{json!({"reason":"max_output_tokens"})}else if output.finish=="content_filter"{json!({"reason":"content_filter"})}else{Value::Null}})
        }
        LlmProtocol::Messages => {
            json!({"id":output.id,"type":"message","role":"assistant","model":output.model,"content":blocks,"stop_reason":match output.finish.as_str(){"length"=>"max_tokens","tool_calls"=>"tool_use","content_filter"=>"refusal",_=>"end_turn"},"stop_sequence":null,"usage":usage})
        }
    })
}
pub(super) fn error_body(to: LlmProtocol, error: &Value) -> Value {
    let message = error["message"]
        .as_str()
        .unwrap_or("upstream protocol error");
    let kind = error["type"].as_str().unwrap_or("upstream_error");
    if to == LlmProtocol::Messages {
        json!({"type":"error","error":{"type":kind,"message":message}})
    } else {
        json!({"error":{"type":kind,"message":message,"code":error.get("code").cloned().unwrap_or(Value::Null)}})
    }
}

pub(super) fn custom_input(arguments: &str) -> Result<(String, bool), TranslationError> {
    let parsed: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    Ok(match parsed.get("input").and_then(Value::as_str) {
        Some(input) => (input.to_owned(), false),
        None => (arguments.to_owned(), true),
    })
}

pub(super) fn arguments_object(arguments: &str) -> (Value, bool) {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) if value.is_object() => (value, false),
        _ => (json!({"input":arguments}), true),
    }
}

pub(super) fn usage_has_unknown_counts(value: &Value) -> bool {
    value["input_tokens"].as_u64().is_none() || value["output_tokens"].as_u64().is_none()
}
