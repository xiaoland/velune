use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use velune_ai::{
    Payload,
    ids::*,
    observation::{Quantity, Usage},
    provider::*,
    sampling::*,
};

fn selected(value: &Value, fields: &[&str]) -> Value {
    let mut result = serde_json::Map::new();
    for field in fields {
        if let Some(value) = value.get(*field) {
            result.insert((*field).into(), value.clone());
        }
    }
    Value::Object(result)
}
/// Projection happens before capture. Provider response IDs/timestamps/fingerprints, headers,
/// cookies, raw errors, and unrecognized field values are never passed to the capture sink.
pub(crate) fn project(raw: &Value) -> Value {
    let mut result = selected(raw, &["model"]);
    result["source_usage_present"] = json!(raw.get("usage").is_some_and(|v| !v.is_null()));
    result["provider_error_present"] = json!(
        raw.get("error").is_some_and(|v| !v.is_null())
            || raw
                .pointer("/base_resp/status_code")
                .and_then(Value::as_u64)
                .is_some_and(|v| v != 0)
    );
    if let Some(usage) = raw.get("usage").filter(|v| !v.is_null()) {
        result["usage"] = selected(
            usage,
            &["prompt_tokens", "completion_tokens", "total_tokens"],
        );
        result["unsupported_usage_fields_present"] = json!(usage.as_object().is_none_or(
            |object| object.keys().any(|key| {
                !["prompt_tokens", "completion_tokens", "total_tokens"].contains(&key.as_str())
            })
        ));
    }
    if let Some(choices) = raw.get("choices").and_then(Value::as_array) {
        result["choices"] = choices
            .iter()
            .map(|choice| {
                let mut mapped = selected(choice, &["index", "finish_reason"]);
                if let Some(delta) = choice.get("delta") {
                    let mut projected = selected(delta, &["role", "content"]);
                    projected["unsupported_fields_present"] =
                        json!(delta.as_object().is_none_or(|obj| obj.iter().any(|(k, v)| {
                            !["role", "content", "tool_calls"].contains(&k.as_str())
                                && !v.is_null()
                                && v != ""
                        })));
                    if let Some(tools) = delta.get("tool_calls") {
                        projected["tool_calls"] = match tools.as_array() {
                            Some(tools) => tools
                                .iter()
                                .map(|tool| {
                                    let mut t = selected(tool, &["index", "id", "type"]);
                                    if let Some(function) = tool.get("function") {
                                        t["function"] = selected(function, &["name", "arguments"]);
                                    }
                                    t
                                })
                                .collect(),
                            None => Value::Null,
                        };
                    }
                    mapped["delta"] = projected;
                }
                mapped
            })
            .collect();
    }
    result
}

#[derive(Default)]
pub struct Decoder {
    text: String,
    tools: BTreeMap<u32, PartialToolCall>,
    finish: Option<FinishReason>,
    usage: Usage,
    usage_seen: bool,
    failed: bool,
}
impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Consume the same projected record used by live capture. The replay entry point never
    /// constructs a client/provider/credential; it calls this exact mapper.
    pub fn push(
        &mut self,
        chunk: &Value,
        events: &mut ProviderSamplingSink,
    ) -> Result<(), SamplingErrorKind> {
        if self.failed {
            return Err(SamplingErrorKind::ProviderFailure);
        }
        let result = self.push_inner(chunk, events);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn push_inner(
        &mut self,
        chunk: &Value,
        events: &mut ProviderSamplingSink,
    ) -> Result<(), SamplingErrorKind> {
        let bad = SamplingErrorKind::ProviderFailure;
        if chunk["provider_error_present"] == true {
            return Err(bad);
        }
        if let Some(model) = chunk.get("model")
            && !model.is_string()
        {
            return Err(bad);
        }
        if let Some(usage) = chunk.get("usage") {
            if self.usage_seen {
                return Err(bad);
            }
            self.usage_seen = true;
            fn count(value: Option<&Value>) -> Result<Quantity<u64>, SamplingErrorKind> {
                match value {
                    None | Some(Value::Null) => Ok(Quantity::Unknown),
                    Some(v) => v
                        .as_u64()
                        .map(Quantity::Reported)
                        .ok_or(SamplingErrorKind::ProviderFailure),
                }
            }
            self.usage = Usage {
                input_tokens: count(usage.get("prompt_tokens"))?,
                output_tokens: count(usage.get("completion_tokens"))?,
            };
            events(SamplingDelta::Usage(self.usage));
        }
        let choices = chunk.get("choices").and_then(Value::as_array).ok_or(bad)?;
        if choices.len() > 1 {
            return Err(SamplingErrorKind::Unsupported);
        }
        for choice in choices {
            if choice["index"].as_u64() != Some(0) {
                return Err(SamplingErrorKind::Unsupported);
            }
            if let Some(delta) = choice.get("delta") {
                if delta["unsupported_fields_present"] == true {
                    return Err(SamplingErrorKind::Unsupported);
                }
                if let Some(role) = delta.get("role").filter(|v| !v.is_null())
                    && role != "assistant"
                {
                    return Err(bad);
                }
                if let Some(content) = delta.get("content").filter(|v| !v.is_null()) {
                    let text = content.as_str().ok_or(bad)?;
                    if !text.is_empty() {
                        if self.finish.is_some() || self.text.len() + text.len() > 65536 {
                            return Err(bad);
                        }
                        self.text.push_str(text);
                        events(SamplingDelta::Text(Payload::new(text.into())));
                    }
                }
                if let Some(tools) = delta.get("tool_calls").filter(|v| !v.is_null()) {
                    for tool in tools.as_array().ok_or(bad)? {
                        if self.finish.is_some() {
                            return Err(bad);
                        }
                        let index = tool["index"]
                            .as_u64()
                            .and_then(|i| u32::try_from(i).ok())
                            .ok_or(bad)?;
                        if index >= 16 {
                            return Err(SamplingErrorKind::Unsupported);
                        }
                        if let Some(kind) = tool.get("type")
                            && kind != "function"
                        {
                            return Err(SamplingErrorKind::Unsupported);
                        }
                        let state = self.tools.entry(index).or_insert_with(|| PartialToolCall {
                            index,
                            id: None,
                            name: None,
                            arguments: Payload::new(String::new()),
                        });
                        let id = tool
                            .get("id")
                            .filter(|v| !v.is_null())
                            .map(|v| ToolCallId::new(v.as_str().ok_or(bad)?).map_err(|_| bad))
                            .transpose()?;
                        let name = tool
                            .pointer("/function/name")
                            .filter(|v| !v.is_null())
                            .map(|v| ToolName::new(v.as_str().ok_or(bad)?).map_err(|_| bad))
                            .transpose()?;
                        if id
                            .as_ref()
                            .is_some_and(|id| state.id.as_ref().is_some_and(|old| old != id))
                            || name.as_ref().is_some_and(|name| {
                                state.name.as_ref().is_some_and(|old| old != name)
                            })
                        {
                            return Err(bad);
                        }
                        if id.is_some() || name.is_some() {
                            if id.is_some() {
                                state.id = id.clone();
                            }
                            if name.is_some() {
                                state.name = name.clone();
                            }
                            events(SamplingDelta::ToolIdentity { index, id, name });
                        }
                        if let Some(args) = tool.pointer("/function/arguments") {
                            let fragment = args.as_str().ok_or(bad)?;
                            if state.arguments.get().len() + fragment.len() > 65536 {
                                return Err(bad);
                            }
                            let mut joined = state.arguments.get().clone();
                            joined.push_str(fragment);
                            state.arguments = Payload::new(joined);
                            events(SamplingDelta::ToolArguments {
                                index,
                                fragment: Payload::new(fragment.into()),
                            });
                        }
                    }
                }
            }
            if let Some(finish) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                if self.finish.is_some() {
                    return Err(bad);
                }
                let reason = match finish.as_str() {
                    Some("stop") => FinishReason::Complete,
                    Some("length") => FinishReason::OutputLimit,
                    Some("tool_calls") => FinishReason::ToolCalls,
                    _ => return Err(SamplingErrorKind::Unsupported),
                };
                self.finish = Some(reason);
                events(SamplingDelta::Finish(reason));
            }
        }
        Ok(())
    }
    pub fn failure(
        self,
        kind: SamplingErrorKind,
        execution: ExecutionKnowledge,
        submitted: bool,
    ) -> ProviderSamplingOutcome {
        let partial =
            (!self.text.is_empty() || !self.tools.is_empty()).then(|| PartialSamplingOutput {
                text: (!self.text.is_empty()).then(|| Payload::new(self.text)),
                tool_calls: self.tools.into_values().collect(),
            });
        ProviderSamplingOutcome {
            result: Err(SamplingFailure {
                kind,
                execution,
                partial,
            }),
            usage: self.usage,
            submitted,
        }
    }
    /// MiniMax's SDK-compatible stream can close without a DONE sentinel. Accept only a clean
    /// transport EOF with no unfinished SSE framing, explicit finish, and reported token usage.
    /// An EOF/error alone is never completion. This does not manufacture a protocol event.
    pub fn clean_eof(self, framing: super::FramingEvidence) -> ProviderSamplingOutcome {
        if framing.done_lines != 0
            || framing.done_frames != 0
            || framing.unfinished_line_is_done
            || framing.unfinished_data_frame
            || framing.unfinished_line
            || self.finish.is_none()
            || !self.usage_seen
            || !matches!(
                (self.usage.input_tokens, self.usage.output_tokens),
                (Quantity::Reported(_), Quantity::Reported(_))
            )
        {
            return self.failure(
                SamplingErrorKind::ProviderFailure,
                ExecutionKnowledge::Accepted,
                true,
            );
        }
        self.complete()
    }
    pub fn complete(self) -> ProviderSamplingOutcome {
        let output = self.output();
        match output {
            Ok(output) => ProviderSamplingOutcome {
                result: Ok(output),
                usage: self.usage,
                submitted: true,
            },
            Err(kind) => self.failure(kind, ExecutionKnowledge::Accepted, true),
        }
    }
    fn output(&self) -> Result<SamplingOutput, SamplingErrorKind> {
        let bad = SamplingErrorKind::ProviderFailure;
        if self.failed {
            return Err(bad);
        }
        let finish = self.finish.ok_or(bad)?;
        if (finish == FinishReason::ToolCalls) != !self.tools.is_empty()
            && finish != FinishReason::OutputLimit
        {
            return Err(bad);
        }
        let mut tool_calls = Vec::new();
        let mut ids = HashSet::new();
        for tool in self.tools.values() {
            let id = tool.id.clone().ok_or(bad)?;
            if !ids.insert(id.clone()) {
                return Err(bad);
            }
            let name = tool.name.clone().ok_or(bad)?;
            let arguments: Value = serde_json::from_str(tool.arguments.get()).map_err(|_| bad)?;
            if !arguments.is_object() {
                return Err(bad);
            }
            tool_calls.push(ToolCall {
                id,
                name,
                arguments: Payload::new(arguments),
            });
        }
        Ok(SamplingOutput {
            text: (!self.text.is_empty()).then(|| Payload::new(self.text.clone())),
            tool_calls,
            finish,
        })
    }
}
