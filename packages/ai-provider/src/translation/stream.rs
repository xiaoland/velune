use super::response::{self, Call, Output};
use super::*;
use serde_json::json;

struct ToolState {
    call: Call,
    index: usize,
    started: bool,
    sent: usize,
    custom_source: bool,
}

/// Semantic SSE conversion. The caller owns SSE framing, UTF-8 validation,
/// cancellation and transport backpressure. Text and ordinary OpenAI function
/// argument deltas are emitted immediately. Messages requires an object, and
/// custom-tool codecs require a complete wrapper, so those tool arguments are
/// buffered until completion. The caller controls transport backpressure and
/// any application-specific resource budget.
pub struct StreamTranslator {
    from: LlmProtocol,
    to: LlmProtocol,
    model: String,
    tools: BTreeMap<String, ToolIdentity>,
    output: Output,
    calls: BTreeMap<u64, ToolState>,
    item_keys: BTreeMap<String, u64>,
    next_index: usize,
    text_index: Option<usize>,
    refusal_index: Option<usize>,
    reasoning_index: Option<usize>,
    started: bool,
    terminal: bool,
    finish_seen: bool,
    sequence: u64,
    notes: Vec<ConversionNote>,
}
impl fmt::Debug for StreamTranslator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamTranslator")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}
impl StreamTranslator {
    pub(super) fn new(
        from: LlmProtocol,
        to: LlmProtocol,
        model: String,
        tools: BTreeMap<String, ToolIdentity>,
    ) -> Self {
        Self {
            from,
            to,
            model,
            tools,
            output: Output::default(),
            calls: BTreeMap::new(),
            item_keys: BTreeMap::new(),
            next_index: 0,
            text_index: None,
            refusal_index: None,
            reasoning_index: None,
            started: false,
            terminal: false,
            finish_seen: false,
            sequence: 0,
            notes: vec![],
        }
    }
    pub fn notes(&self) -> &[ConversionNote] {
        &self.notes
    }
    pub fn push_event(&mut self, event: &str, data: &str) -> Result<Vec<u8>, TranslationError> {
        if data.trim().is_empty() {
            return Ok(vec![]);
        }
        if self.terminal {
            request::note(
                &mut self.notes,
                "stream.tail",
                "event_after_terminal_omitted",
            );
            return Ok(vec![]);
        }
        if data.trim() == "[DONE]" {
            if self.from != LlmProtocol::ChatCompletions {
                return Err(invalid("stream.unexpected_done"));
            }
            if !self.finish_seen {
                self.output.finish = "unknown".into();
                request::note(&mut self.notes, "finish_reason", "unreported_at_done");
            }
            return self.complete();
        }
        let value: Value = serde_json::from_str(data).map_err(|_| invalid("stream.json"))?;
        object(&value, "stream.event")?;
        if value.get("error").is_some_and(|v| !v.is_null())
            || event == "error"
            || value["type"] == "error"
        {
            let error = value.get("error").unwrap_or(&value);
            let body = response::error_body(self.to, error);
            self.terminal = true;
            let mut bytes = vec![];
            self.event(
                &mut bytes,
                if self.to == LlmProtocol::ChatCompletions {
                    ""
                } else {
                    "error"
                },
                body,
            );
            return Ok(bytes);
        }
        match self.from {
            LlmProtocol::ChatCompletions => self.chat(value),
            LlmProtocol::Responses => self.responses(event, value),
            LlmProtocol::Messages => self.messages(event, value),
        }
    }
    /// A clean EOF can close Chat Completions only after a finish reason. Other
    /// protocols require their explicit native terminal event.
    pub fn finish(&mut self) -> Result<Vec<u8>, TranslationError> {
        if self.terminal {
            return Ok(vec![]);
        }
        if self.finish_seen {
            request::note(
                &mut self.notes,
                "stream.terminator",
                "missing_redundant_terminator",
            );
            self.complete()
        } else {
            Err(TranslationError {
                kind: TranslationErrorKind::Truncated,
                field: "stream.missing_terminal",
            })
        }
    }
    fn event(&mut self, output: &mut Vec<u8>, event: &str, mut value: Value) {
        if self.to == LlmProtocol::Responses {
            value["sequence_number"] = json!(self.sequence);
            self.sequence += 1;
        }
        if !event.is_empty() {
            output.extend_from_slice(format!("event: {event}\n").as_bytes());
        }
        output.extend_from_slice(format!("data: {value}\n\n").as_bytes());
    }
    fn observe_id(&mut self, id: Option<&str>) {
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            if self.output.id.is_empty() && !self.started {
                self.output.id = id.to_owned();
            } else if self.output.id != id {
                request::note(
                    &mut self.notes,
                    "response.id",
                    "later_upstream_identity_ignored",
                );
            }
        }
    }
    fn start(&mut self, bytes: &mut Vec<u8>) {
        if self.started {
            return;
        }
        self.started = true;
        if self.output.id.is_empty() {
            self.output.id = local_response_id();
            request::note(&mut self.notes, "response.id", "local_conversion_identity");
        }
        if self.output.model.is_empty() {
            self.output.model = self.model.clone();
        }
        match self.to {
            LlmProtocol::ChatCompletions => {
                self.chat_chunk(bytes, json!({"role":"assistant"}), Value::Null, Value::Null)
            }
            LlmProtocol::Responses => {
                let response = json!({"id":self.output.id,"object":"response","created_at":self.output.created,"status":"in_progress","model":self.output.model,"output":[],"usage":null,"error":null,"incomplete_details":null});
                self.event(
                    bytes,
                    "response.created",
                    json!({"type":"response.created","response":response}),
                );
                self.event(
                    bytes,
                    "response.in_progress",
                    json!({"type":"response.in_progress","response":response}),
                );
            }
            LlmProtocol::Messages => {
                if response::usage_has_unknown_counts(&self.output.usage) {
                    request::note(&mut self.notes, "usage", "required_numeric_placeholder");
                }
                let usage = response::encode_usage(self.to, &self.output.usage);
                self.event(bytes,"message_start",json!({"type":"message_start","message":{"id":self.output.id,"type":"message","role":"assistant","model":self.output.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":usage}}));
            }
        }
    }
    fn chat_chunk(&mut self, bytes: &mut Vec<u8>, delta: Value, finish: Value, usage: Value) {
        self.event(bytes,"",json!({"id":self.output.id,"object":"chat.completion.chunk","created":self.output.created,"model":self.output.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}],"usage":usage}));
    }
    fn allocate(&mut self) -> usize {
        let index = self.next_index;
        self.next_index += 1;
        index
    }
    fn text_delta(
        &mut self,
        bytes: &mut Vec<u8>,
        text: &str,
        kind: &str,
    ) -> Result<(), TranslationError> {
        self.start(bytes);
        let existing = match kind {
            "refusal" => self.refusal_index,
            "reasoning" => self.reasoning_index,
            _ => self.text_index,
        };
        let index = if let Some(index) = existing {
            index
        } else {
            let index = self.allocate();
            match kind {
                "refusal" => self.refusal_index = Some(index),
                "reasoning" => self.reasoning_index = Some(index),
                _ => self.text_index = Some(index),
            };
            match self.to{
                LlmProtocol::Messages=>self.event(bytes,"content_block_start",json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}})),
                LlmProtocol::Responses=>{
                    let item=if kind=="reasoning"{json!({"type":"reasoning","id":format!("{}_reasoning",self.output.id),"summary":[]})}else{json!({"type":"message","id":format!("{}_{}_message",self.output.id,index),"role":"assistant","status":"in_progress","content":[]})};
                    self.event(bytes,"response.output_item.added",json!({"type":"response.output_item.added","output_index":index,"item":item}));
                    if kind=="reasoning"{self.event(bytes,"response.reasoning_summary_part.added",json!({"type":"response.reasoning_summary_part.added","output_index":index,"item_id":item["id"],"summary_index":0,"part":{"type":"summary_text","text":""}}));}
                    else{self.event(bytes,"response.content_part.added",json!({"type":"response.content_part.added","output_index":index,"item_id":item["id"],"content_index":0,"part":if kind=="refusal"{json!({"type":"refusal","refusal":""})}else{json!({"type":"output_text","text":"","annotations":[]})}}));}
                },
                LlmProtocol::ChatCompletions=>{}
            }
            index
        };
        match kind {
            "refusal" => self.output.refusal.push_str(text),
            "reasoning" => self.output.reasoning.push_str(text),
            _ => self.output.text.push_str(text),
        }
        match self.to{
            LlmProtocol::ChatCompletions=>self.chat_chunk(bytes,json!({match kind{"refusal"=>"refusal","reasoning"=>"reasoning_content",_=>"content"}:text}),Value::Null,Value::Null),
            LlmProtocol::Messages=>{if kind!="text"{request::note(&mut self.notes,"response.reasoning_refusal","rendered_as_text");}self.event(bytes,"content_block_delta",json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}}));},
            LlmProtocol::Responses=>{let (event,item_id,index_name)=if kind=="reasoning"{("response.reasoning_summary_text.delta",format!("{}_reasoning",self.output.id),"summary_index")}else if kind=="refusal"{("response.refusal.delta",format!("{}_{}_message",self.output.id,index),"content_index")}else{("response.output_text.delta",format!("{}_{}_message",self.output.id,index),"content_index")};self.event(bytes,event,json!({"type":event,"output_index":index,"item_id":item_id,index_name:0,"delta":text}));}
        }
        Ok(())
    }
    fn tool_delta(
        &mut self,
        bytes: &mut Vec<u8>,
        key: u64,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
        custom_source: bool,
    ) -> Result<(), TranslationError> {
        self.start(bytes);
        if !self.calls.contains_key(&key) {
            let index = self.allocate();
            self.calls.insert(
                key,
                ToolState {
                    call: Call::default(),
                    index,
                    started: false,
                    sent: 0,
                    custom_source,
                },
            );
        }
        if let Some(id) = id
            && self
                .calls
                .iter()
                .any(|(other, state)| *other != key && state.call.id == id)
        {
            return Err(invalid("stream.duplicate_tool_id"));
        }
        let state = self
            .calls
            .get_mut(&key)
            .ok_or_else(|| invalid("tool_call.index"))?;
        if let Some(id) = id {
            if state.call.id.is_empty() {
                state.call.id = id.into();
            } else if state.call.id != id {
                return Err(invalid("tool_call.identity_changed"));
            }
        }
        if let Some(name) = name {
            if state.call.name.is_empty() {
                state.call.name = name.into();
            } else if state.call.name != name {
                if state.started {
                    return Err(invalid("stream.tool_name_changed"));
                }
                state.call.name.push_str(name);
            }
        }
        state.call.arguments.push_str(arguments);
        if state.call.id.is_empty() || state.call.name.is_empty() {
            return Ok(());
        }
        let index = state.index;
        let id = state.call.id.clone();
        let name = state.call.name.clone();
        let identity = self.tools.get(&name);
        let mapped = identity.map(|v| v.original.clone()).unwrap_or(name);
        let custom = identity.is_some_and(|v| v.custom) && self.to == LlmProtocol::Responses;
        let should_start = !state.started;
        state.started = true;
        let delta = state.call.arguments[state.sent..].to_owned();
        if !custom && !state.custom_source && self.to != LlmProtocol::Messages {
            state.sent = state.call.arguments.len();
        }
        if should_start {
            match self.to{
            LlmProtocol::ChatCompletions=>self.chat_chunk(bytes,json!({"tool_calls":[{"index":index,"id":id,"type":"function","function":{"name":mapped,"arguments":""}}]}),Value::Null,Value::Null),
            LlmProtocol::Messages=>self.event(bytes,"content_block_start",json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":id,"name":mapped,"input":{}}})),
            LlmProtocol::Responses=>{let mut item=if custom{json!({"type":"custom_tool_call","id":format!("{}_{id}_item",self.output.id),"call_id":id,"name":mapped,"input":"","status":"in_progress"})}else{json!({"type":"function_call","id":format!("{}_{id}_item",self.output.id),"call_id":id,"name":mapped,"arguments":"","status":"in_progress"})};if let Some(namespace)=self.tools.get(&self.calls[&key].call.name).and_then(|v|v.namespace.as_ref()){item["namespace"]=json!(namespace);}self.event(bytes,"response.output_item.added",json!({"type":"response.output_item.added","output_index":index,"item":item}));}
        }
        }
        if !delta.is_empty() && !custom && !custom_source && self.to != LlmProtocol::Messages {
            self.argument_delta(bytes, index, &id, &delta, false);
        }
        Ok(())
    }
    fn argument_delta(
        &mut self,
        bytes: &mut Vec<u8>,
        index: usize,
        id: &str,
        delta: &str,
        custom: bool,
    ) {
        match self.to{
        LlmProtocol::ChatCompletions=>self.chat_chunk(bytes,json!({"tool_calls":[{"index":index,"function":{"arguments":delta}}]}),Value::Null,Value::Null),
        LlmProtocol::Messages=>self.event(bytes,"content_block_delta",json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":delta}})),
        LlmProtocol::Responses=>{let event=if custom{"response.custom_tool_call_input.delta"}else{"response.function_call_arguments.delta"};self.event(bytes,event,json!({"type":event,"output_index":index,"item_id":format!("{}_{id}_item",self.output.id),"delta":delta}));}
    }
    }
    fn chat(&mut self, value: Value) -> Result<Vec<u8>, TranslationError> {
        let mut bytes = vec![];
        self.observe_id(value["id"].as_str());
        if let Some(model) = value["model"].as_str() {
            self.output.model = model.into();
        }
        if let Some(created) = value["created"].as_u64() {
            self.output.created = created;
        }
        if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
            self.output.usage = response::normalize_usage(self.from, usage);
        }
        if value.get("choices").is_none() {
            request::note(&mut self.notes, "stream.event", "unknown_event_omitted");
            return Ok(bytes);
        }
        for choice in array(&value["choices"], "stream.choices")? {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                request::note(&mut self.notes, "choices", "first_choice_only");
                continue;
            }
            let delta = &choice["delta"];
            for (field, kind) in [
                ("content", "text"),
                ("refusal", "refusal"),
                ("reasoning_content", "reasoning"),
            ] {
                if let Some(text) = delta[field].as_str() {
                    self.text_delta(&mut bytes, text, kind)?;
                }
            }
            if let Some(calls) = delta.get("tool_calls") {
                for call in array(calls, "stream.tool_calls")? {
                    let key = call["index"]
                        .as_u64()
                        .ok_or_else(|| invalid("stream.tool_call.index"))?;
                    self.tool_delta(
                        &mut bytes,
                        key,
                        call["id"].as_str(),
                        call["function"]["name"].as_str(),
                        call["function"]["arguments"].as_str().unwrap_or(""),
                        false,
                    )?;
                }
            }
            if let Some(finish) = choice["finish_reason"].as_str() {
                self.output.finish = finish.into();
                self.finish_seen = true;
            }
        }
        Ok(bytes)
    }
    fn responses(&mut self, event: &str, value: Value) -> Result<Vec<u8>, TranslationError> {
        let mut bytes = vec![];
        let kind = value["type"].as_str().unwrap_or(event);
        match kind {
            "response.created" | "response.in_progress" => {
                let response = &value["response"];
                self.observe_id(response["id"].as_str());
                if let Some(model) = response["model"].as_str() {
                    self.output.model = model.into();
                }
                self.output.created = response["created_at"].as_u64().unwrap_or(0);
                self.start(&mut bytes);
            }
            "response.output_text.delta" => self.text_delta(
                &mut bytes,
                string(&value["delta"], "stream.text_delta")?,
                "text",
            )?,
            "response.refusal.delta" => self.text_delta(
                &mut bytes,
                string(&value["delta"], "stream.refusal_delta")?,
                "refusal",
            )?,
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                request::note(&mut self.notes, "reasoning", "best_effort_text");
                self.text_delta(
                    &mut bytes,
                    string(&value["delta"], "stream.reasoning_delta")?,
                    "reasoning",
                )?;
            }
            "response.output_item.added" => {
                let item = &value["item"];
                if item["type"] == "function_call" || item["type"] == "custom_tool_call" {
                    let key = value["output_index"]
                        .as_u64()
                        .ok_or_else(|| invalid("stream.output_index"))?;
                    let id = string(&item["id"], "stream.item_id")?;
                    self.item_keys.insert(id.into(), key);
                    self.tool_delta(
                        &mut bytes,
                        key,
                        item["call_id"].as_str(),
                        item["name"].as_str(),
                        if item["type"] == "custom_tool_call" {
                            item["input"].as_str().unwrap_or("")
                        } else {
                            item["arguments"].as_str().unwrap_or("")
                        },
                        item["type"] == "custom_tool_call",
                    )?;
                }
            }
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                let key = value["output_index"]
                    .as_u64()
                    .or_else(|| {
                        value["item_id"]
                            .as_str()
                            .and_then(|id| self.item_keys.get(id).copied())
                    })
                    .ok_or_else(|| invalid("stream.tool_call.index"))?;
                self.tool_delta(
                    &mut bytes,
                    key,
                    None,
                    None,
                    string(&value["delta"], "stream.tool_call.delta")?,
                    kind == "response.custom_tool_call_input.delta",
                )?;
            }
            "response.completed" | "response.incomplete" => {
                let response = &value["response"];
                self.output.finish = if kind == "response.incomplete" {
                    if response["incomplete_details"]["reason"] == "content_filter" {
                        "content_filter"
                    } else {
                        "length"
                    }
                } else if self.calls.is_empty() {
                    "stop"
                } else {
                    "tool_calls"
                }
                .into();
                self.output.usage = response::normalize_usage(self.from, &response["usage"]);
                self.finish_seen = true;
                self.fill_response_snapshot(response, &mut bytes)?;
                bytes.extend(self.complete()?);
            }
            "response.failed" | "response.cancelled" => {
                self.terminal = true;
                let error=value["response"].get("error").filter(|v|!v.is_null()).cloned().unwrap_or(json!({"type":"upstream_error","message":"upstream response did not complete"}));
                self.event(
                    &mut bytes,
                    if self.to == LlmProtocol::ChatCompletions {
                        ""
                    } else {
                        "error"
                    },
                    response::error_body(self.to, &error),
                );
            }
            "response.output_text.done"
            | "response.refusal.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.function_call_arguments.done"
            | "response.custom_tool_call_input.done"
            | "response.output_item.done"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done" => {}
            _ => request::note(&mut self.notes, "stream.event", "unknown_event_omitted"),
        }
        Ok(bytes)
    }
    fn fill_response_snapshot(
        &mut self,
        value: &Value,
        bytes: &mut Vec<u8>,
    ) -> Result<(), TranslationError> {
        if let Some(items) = value["output"].as_array() {
            for (key, item) in items.iter().enumerate() {
                match item["type"].as_str() {
                    Some("message")
                        if self.output.text.is_empty() && self.output.refusal.is_empty() =>
                    {
                        if let Some(parts) = item["content"].as_array() {
                            for part in parts {
                                match part["type"].as_str() {
                                    Some("output_text") => self.text_delta(
                                        bytes,
                                        string(&part["text"], "output.text")?,
                                        "text",
                                    )?,
                                    Some("refusal") => self.text_delta(
                                        bytes,
                                        string(&part["refusal"], "output.refusal")?,
                                        "refusal",
                                    )?,
                                    _ => {}
                                }
                            }
                        }
                    }
                    Some("function_call" | "custom_tool_call") => {
                        let key = key as u64;
                        if !self.calls.contains_key(&key) {
                            self.tool_delta(
                                bytes,
                                key,
                                item["call_id"].as_str(),
                                item["name"].as_str(),
                                if item["type"] == "custom_tool_call" {
                                    item["input"].as_str().unwrap_or("")
                                } else {
                                    item["arguments"].as_str().unwrap_or("")
                                },
                                item["type"] == "custom_tool_call",
                            )?;
                        } else if self.calls[&key].call.arguments.is_empty() {
                            let arguments = if item["type"] == "custom_tool_call" {
                                item["input"].as_str().unwrap_or("")
                            } else {
                                item["arguments"].as_str().unwrap_or("")
                            };
                            self.tool_delta(
                                bytes,
                                key,
                                None,
                                None,
                                arguments,
                                item["type"] == "custom_tool_call",
                            )?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    fn messages(&mut self, event: &str, value: Value) -> Result<Vec<u8>, TranslationError> {
        let mut bytes = vec![];
        let kind = value["type"].as_str().unwrap_or(event);
        match kind {
            "message_start" => {
                let message = &value["message"];
                self.observe_id(message["id"].as_str());
                self.output.model = message["model"].as_str().unwrap_or(&self.model).into();
                self.output.usage = response::normalize_usage(self.from, &message["usage"]);
                self.start(&mut bytes);
            }
            "content_block_start" => {
                let block = &value["content_block"];
                let key = value["index"]
                    .as_u64()
                    .ok_or_else(|| invalid("stream.block.index"))?;
                match block["type"].as_str() {
                    Some("tool_use") => {
                        let initial = if block["input"].as_object().is_some_and(|v| !v.is_empty()) {
                            block["input"].to_string()
                        } else {
                            String::new()
                        };
                        self.tool_delta(
                            &mut bytes,
                            key,
                            block["id"].as_str(),
                            block["name"].as_str(),
                            &initial,
                            false,
                        )?;
                    }
                    Some("text") => {
                        if let Some(text) = block["text"].as_str().filter(|v| !v.is_empty()) {
                            self.text_delta(&mut bytes, text, "text")?;
                        }
                    }
                    Some("thinking") => {
                        request::note(&mut self.notes, "thinking", "signature_omitted");
                        if let Some(text) = block["thinking"].as_str().filter(|v| !v.is_empty()) {
                            self.text_delta(&mut bytes, text, "reasoning")?;
                        }
                    }
                    _ => request::note(
                        &mut self.notes,
                        "content.block",
                        "unsupported_block_omitted",
                    ),
                }
            }
            "content_block_delta" => {
                let delta = &value["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => self.text_delta(
                        &mut bytes,
                        string(&delta["text"], "stream.text_delta")?,
                        "text",
                    )?,
                    Some("thinking_delta") => self.text_delta(
                        &mut bytes,
                        string(&delta["thinking"], "stream.thinking_delta")?,
                        "reasoning",
                    )?,
                    Some("input_json_delta") => {
                        let key = value["index"]
                            .as_u64()
                            .ok_or_else(|| invalid("stream.block.index"))?;
                        if !self.calls.contains_key(&key) {
                            return Err(invalid("stream.tool_delta_before_start"));
                        }
                        self.tool_delta(
                            &mut bytes,
                            key,
                            None,
                            None,
                            string(&delta["partial_json"], "stream.tool_json_delta")?,
                            false,
                        )?;
                    }
                    Some("signature_delta") => request::note(
                        &mut self.notes,
                        "thinking.signature",
                        "opaque_reasoning_omitted",
                    ),
                    _ => {
                        request::note(&mut self.notes, "stream.delta", "unsupported_delta_omitted")
                    }
                }
            }
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.output.finish = match reason {
                        "max_tokens" | "model_context_window_exceeded" => "length",
                        "tool_use" => "tool_calls",
                        "refusal" => "content_filter",
                        _ => "stop",
                    }
                    .into();
                    self.finish_seen = true;
                }
                let usage = response::normalize_usage(self.from, &value["usage"]);
                if let Some(values) = usage.as_object() {
                    if !self.output.usage.is_object() {
                        self.output.usage = json!({});
                    }
                    for (key, value) in values {
                        self.output.usage[key] = value.clone();
                    }
                }
            }
            "message_stop" => {
                if !self.finish_seen {
                    return Err(invalid("stream.stop_without_finish"));
                }
                bytes.extend(self.complete()?);
            }
            "content_block_stop" | "ping" => {}
            _ => request::note(&mut self.notes, "stream.event", "unknown_event_omitted"),
        }
        Ok(bytes)
    }
    fn complete(&mut self) -> Result<Vec<u8>, TranslationError> {
        let mut bytes = vec![];
        self.start(&mut bytes);
        let mut calls = vec![];
        for state in self.calls.values() {
            if !state.started {
                return Err(invalid("stream.tool_call.identity"));
            }
            let arguments = if state.custom_source {
                json!({"input":state.call.arguments}).to_string()
            } else if state.call.arguments.is_empty() && self.from == LlmProtocol::Messages {
                "{}".into()
            } else {
                state.call.arguments.clone()
            };

            calls.push((
                state.index,
                Call {
                    id: state.call.id.clone(),
                    name: state.call.name.clone(),
                    arguments,
                },
                state.custom_source,
            ));
        }
        for (index, call, custom_source) in &calls {
            let custom = self.to == LlmProtocol::Responses
                && self.tools.get(&call.name).is_some_and(|v| v.custom);
            if custom {
                let (input, fallback) = response::custom_input(&call.arguments)?;
                if fallback {
                    request::note(
                        &mut self.notes,
                        "custom_tool.input",
                        "wrapper_mismatch_raw_arguments",
                    );
                }
                self.argument_delta(&mut bytes, *index, &call.id, &input, true);
            } else if self.to == LlmProtocol::Messages {
                let (input, fallback) = response::arguments_object(&call.arguments);
                if fallback {
                    request::note(&mut self.notes, "tool_call.arguments", "wrapped_raw_input");
                }
                request::note(
                    &mut self.notes,
                    "tool_call.arguments",
                    "buffered_for_object_target",
                );
                self.argument_delta(&mut bytes, *index, &call.id, &input.to_string(), false);
            } else if *custom_source {
                self.argument_delta(&mut bytes, *index, &call.id, &call.arguments, false);
            }
        }
        self.output.tools = calls.into_iter().map(|(_, call, _)| call).collect();
        if self.to == LlmProtocol::Messages
            && response::usage_has_unknown_counts(&self.output.usage)
        {
            request::note(&mut self.notes, "usage", "required_numeric_placeholder");
        }
        if self.output.usage.is_null() {
            request::note(&mut self.notes, "usage", "unreported");
        }
        match self.to {
            LlmProtocol::ChatCompletions => {
                self.chat_chunk(
                    &mut bytes,
                    json!({}),
                    if self.output.finish == "unknown" {
                        Value::Null
                    } else {
                        json!(self.output.finish)
                    },
                    response::encode_usage(self.to, &self.output.usage),
                );
                bytes.extend_from_slice(b"data: [DONE]\n\n");
            }
            LlmProtocol::Messages => {
                for index in 0..self.next_index {
                    self.event(
                        &mut bytes,
                        "content_block_stop",
                        json!({"type":"content_block_stop","index":index}),
                    );
                }
                let stop = match self.output.finish.as_str() {
                    "length" => "max_tokens",
                    "tool_calls" => "tool_use",
                    "content_filter" => "refusal",
                    _ => "end_turn",
                };
                self.event(&mut bytes,"message_delta",json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":response::encode_usage(self.to,&self.output.usage)}));
                self.event(&mut bytes, "message_stop", json!({"type":"message_stop"}));
            }
            LlmProtocol::Responses => {
                let items = self.close_response_parts(&mut bytes)?;
                let mut response = response::encode(self.to, &self.output, &self.tools)?;
                response["output"] = json!(items);
                let event =
                    if self.output.finish == "length" || self.output.finish == "content_filter" {
                        "response.incomplete"
                    } else {
                        "response.completed"
                    };
                self.event(&mut bytes, event, json!({"type":event,"response":response}));
            }
        }
        self.terminal = true;
        Ok(bytes)
    }
    fn close_response_parts(
        &mut self,
        bytes: &mut Vec<u8>,
    ) -> Result<Vec<Value>, TranslationError> {
        let mut items = BTreeMap::new();
        for (index, kind, text) in [
            (self.text_index, "text", self.output.text.clone()),
            (self.refusal_index, "refusal", self.output.refusal.clone()),
            (
                self.reasoning_index,
                "reasoning",
                self.output.reasoning.clone(),
            ),
        ] {
            if let Some(index) = index {
                let id = if kind == "reasoning" {
                    format!("{}_reasoning", self.output.id)
                } else {
                    format!("{}_{}_message", self.output.id, index)
                };
                let event = match kind {
                    "refusal" => "response.refusal.done",
                    "reasoning" => "response.reasoning_summary_text.done",
                    _ => "response.output_text.done",
                };
                self.event(bytes,event,json!({"type":event,"output_index":index,"item_id":id,"content_index":0,"summary_index":0,if kind=="refusal"{"refusal"}else{"text"}:text}));
                let part = match kind {
                    "refusal" => json!({"type":"refusal","refusal":text}),
                    "reasoning" => json!({"type":"summary_text","text":text}),
                    _ => json!({"type":"output_text","text":text,"annotations":[]}),
                };
                let event = if kind == "reasoning" {
                    "response.reasoning_summary_part.done"
                } else {
                    "response.content_part.done"
                };
                self.event(bytes,event,json!({"type":event,"output_index":index,"item_id":id,"content_index":0,"summary_index":0,"part":part}));
                let item = if kind == "reasoning" {
                    json!({"type":"reasoning","id":id,"summary":[part]})
                } else {
                    json!({"type":"message","id":id,"role":"assistant","status":"completed","content":[part]})
                };
                items.insert(index, item.clone());
                self.event(
                    bytes,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":index,"item":item}),
                );
            }
        }
        let calls = self
            .calls
            .iter()
            .map(|(key, state)| {
                (
                    *key,
                    state.index,
                    state.call.id.clone(),
                    state.call.name.clone(),
                )
            })
            .collect::<Vec<_>>();
        for (key, index, id, name) in calls {
            let call = self
                .output
                .tools
                .iter()
                .find(|call| call.id == id)
                .ok_or_else(|| invalid("stream.tool_call.id"))?;
            let identity = self.tools.get(&name);
            let custom = identity.is_some_and(|v| v.custom);
            let mut item = if custom {
                json!({"type":"custom_tool_call","id":format!("{}_{id}_item",self.output.id),"call_id":id,"name":identity.map(|v|v.original.as_str()).unwrap_or(&name),"input":response::custom_input(&call.arguments)?.0,"status":"completed"})
            } else {
                json!({"type":"function_call","id":format!("{}_{id}_item",self.output.id),"call_id":id,"name":identity.map(|v|v.original.as_str()).unwrap_or(&name),"arguments":call.arguments,"status":"completed"})
            };
            if let Some(namespace) = identity.and_then(|v| v.namespace.as_ref()) {
                item["namespace"] = json!(namespace);
            }
            let event = if custom {
                "response.custom_tool_call_input.done"
            } else {
                "response.function_call_arguments.done"
            };
            self.event(bytes,event,json!({"type":event,"output_index":index,"item_id":format!("{}_{id}_item",self.output.id),if custom{"input"}else{"arguments"}:if custom{item["input"].clone()}else{item["arguments"].clone()}}));
            items.insert(index, item.clone());
            self.event(
                bytes,
                "response.output_item.done",
                json!({"type":"response.output_item.done","output_index":index,"item":item}),
            );
            let _ = key;
        }
        Ok(items.into_values().collect())
    }
}
