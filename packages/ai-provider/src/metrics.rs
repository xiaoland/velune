//! Read-only facts from original protocol responses, before any translation.
//! Missing numeric values remain unknown; this observer never changes delivery.
use crate::translation::LlmProtocol;
use eventsource_stream::EventStream;
use futures_util::{FutureExt, Stream, StreamExt};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub uncached_input: Option<u64>,
    pub output: Option<u64>,
    pub reasoning_output: Option<u64>,
    pub cached_input: Option<u64>,
    pub cache_read_input: Option<u64>,
    pub cache_creation_input: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTerminal {
    Completed,
    Incomplete,
    Failed,
}

#[derive(Default)]
struct Feed {
    bytes: VecDeque<Vec<u8>>,
    ended: bool,
}
struct Input(Arc<Mutex<Feed>>);
impl Stream for Input {
    type Item = Result<Vec<u8>, Infallible>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut feed = self.0.lock().expect("owned metrics feed");
        match feed.bytes.pop_front() {
            Some(bytes) => Poll::Ready(Some(Ok(bytes))),
            None if feed.ended => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}
/// Synchronously drains the standard SSE parser after each received chunk.
/// The source is polled only here, so it does not need to schedule a waker.
pub struct ResponseMetrics {
    protocol: LlmProtocol,
    feed: Arc<Mutex<Feed>>,
    events: Pin<Box<EventStream<Input>>>,
    observed_bytes: usize,
    usage: TokenUsage,
    messages_input: Option<u64>,
    choices: BTreeMap<u64, bool>,
    pub reported: bool,
    pub first_output: bool,
    pub terminal: Option<ModelTerminal>,
    pub degraded: bool,
}
impl ResponseMetrics {
    pub fn new(protocol: LlmProtocol) -> Self {
        let feed = Arc::new(Mutex::new(Feed::default()));
        Self {
            protocol,
            events: Box::pin(EventStream::new(Input(feed.clone()))),
            feed,
            observed_bytes: 0,
            usage: TokenUsage::default(),
            messages_input: None,
            choices: BTreeMap::new(),
            reported: false,
            first_output: false,
            terminal: None,
            degraded: false,
        }
    }
    pub fn usage(&self) -> TokenUsage {
        self.usage.clone()
    }
    pub fn stream_chunk(&mut self, bytes: &[u8]) {
        // Match the existing conversion body's bound without limiting native delivery.
        if self.degraded {
            return;
        }
        self.observed_bytes = self.observed_bytes.saturating_add(bytes.len());
        if self.observed_bytes > 16 * 1024 * 1024 {
            self.degraded = true;
            return;
        }
        self.feed
            .lock()
            .expect("owned metrics feed")
            .bytes
            .push_back(bytes.to_vec());
        self.drain();
    }
    pub fn stream_end(&mut self) {
        self.feed.lock().expect("owned metrics feed").ended = true;
        self.drain();
    }
    fn drain(&mut self) {
        loop {
            match self.events.next().now_or_never() {
                Some(Some(Ok(event))) if event.data == "[DONE]" => {
                    if self.protocol == LlmProtocol::ChatCompletions
                        && !self.choices.is_empty()
                        && self.choices.values().all(|done| *done)
                    {
                        self.terminal.get_or_insert(ModelTerminal::Completed);
                    }
                }
                Some(Some(Ok(event))) => match serde_json::from_str::<Value>(&event.data) {
                    Ok(value) => self.observe(&value, true),
                    Err(_) => {
                        self.degraded = true;
                        break;
                    }
                },
                Some(Some(Err(_))) => {
                    self.degraded = true;
                    break;
                }
                _ => break,
            }
        }
    }
    pub fn json(&mut self, bytes: &[u8]) {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => self.observe(&value, false),
            Err(_) => self.degraded = true,
        }
    }
    fn observe(&mut self, value: &Value, streaming: bool) {
        if value.get("error").is_some_and(|error| !error.is_null()) || value["type"] == "error" {
            self.terminal = Some(ModelTerminal::Failed);
        }
        match self.protocol {
            LlmProtocol::ChatCompletions => {
                self.merge_usage(&value["usage"]);
                if let Some(choices) = value["choices"].as_array() {
                    for choice in choices {
                        let index = choice["index"].as_u64().unwrap_or(0);
                        let done = choice.get("finish_reason").is_some_and(|v| !v.is_null());
                        let prior = self.choices.entry(index).or_default();
                        *prior |= done;
                        if streaming {
                            self.first_output |= chat_output(&choice["delta"]);
                        }
                    }
                    if !streaming
                        && !self.choices.is_empty()
                        && self.choices.values().all(|done| *done)
                        && self.terminal != Some(ModelTerminal::Failed)
                    {
                        self.terminal = Some(ModelTerminal::Completed);
                    }
                }
            }
            LlmProtocol::Responses => {
                let response = if streaming { &value["response"] } else { value };
                self.merge_usage(&response["usage"]);
                let terminal = match (value["type"].as_str(), response["status"].as_str()) {
                    (Some("response.completed"), _) | (_, Some("completed")) => {
                        Some(ModelTerminal::Completed)
                    }
                    (Some("response.incomplete"), _) | (_, Some("incomplete")) => {
                        Some(ModelTerminal::Incomplete)
                    }
                    (Some("response.failed"), _) | (_, Some("failed")) => {
                        Some(ModelTerminal::Failed)
                    }
                    _ => None,
                };
                if terminal.is_some() {
                    self.terminal = terminal;
                }
                if streaming
                    && matches!(
                        value["type"].as_str(),
                        Some(
                            "response.output_text.delta"
                                | "response.reasoning_text.delta"
                                | "response.reasoning_summary_text.delta"
                                | "response.function_call_arguments.delta"
                                | "response.custom_tool_call_input.delta"
                        )
                    )
                {
                    self.first_output |= nonempty(&value["delta"]);
                }
            }
            LlmProtocol::Messages => {
                self.merge_usage(&value["message"]["usage"]);
                self.merge_usage(&value["usage"]);
                if ((!streaming && nonempty(&value["stop_reason"]))
                    || value["type"] == "message_stop")
                    && self.terminal != Some(ModelTerminal::Failed)
                {
                    self.terminal = Some(ModelTerminal::Completed);
                }
                if streaming {
                    let delta = &value["delta"];
                    self.first_output |= nonempty(&delta["text"])
                        || nonempty(&delta["thinking"])
                        || nonempty(&delta["partial_json"])
                        || nonempty(&value["content_block"]["text"])
                        || nonempty(&value["content_block"]["thinking"]);
                }
            }
        }
    }
    fn merge_usage(&mut self, value: &Value) {
        let Some(usage) = value.as_object() else {
            return;
        };
        let (input, output) = if self.protocol == LlmProtocol::ChatCompletions {
            ("prompt_tokens", "completion_tokens")
        } else {
            ("input_tokens", "output_tokens")
        };
        let a = usage.get(input).and_then(Value::as_u64);
        let b = usage.get(output).and_then(Value::as_u64);
        self.reported |= a.is_some() || b.is_some();
        if let Some(b) = b {
            self.usage.output = Some(b);
        }
        match self.protocol {
            LlmProtocol::ChatCompletions | LlmProtocol::Responses => {
                if a.is_some() {
                    self.usage.input = a;
                }
                let input_details = if self.protocol == LlmProtocol::Responses {
                    "input_tokens_details"
                } else {
                    "prompt_tokens_details"
                };
                let output_details = if self.protocol == LlmProtocol::Responses {
                    "output_tokens_details"
                } else {
                    "completion_tokens_details"
                };
                merge(
                    &mut self.usage.cached_input,
                    value[input_details]["cached_tokens"].as_u64(),
                );
                merge(
                    &mut self.usage.reasoning_output,
                    value[output_details]["reasoning_tokens"].as_u64(),
                );
            }
            LlmProtocol::Messages => {
                merge(&mut self.messages_input, a);
                self.usage.uncached_input = self.messages_input;
                merge(
                    &mut self.usage.cache_read_input,
                    value["cache_read_input_tokens"].as_u64(),
                );
                merge(
                    &mut self.usage.cache_creation_input,
                    value["cache_creation_input_tokens"].as_u64(),
                );
                self.usage.cached_input = self.usage.cache_read_input;
                // No fabricated cache zero: partial Messages input remains unknown.
                self.usage.input = self
                    .messages_input
                    .zip(self.usage.cache_read_input)
                    .zip(self.usage.cache_creation_input)
                    .and_then(|((base, read), write)| base.checked_add(read)?.checked_add(write));
            }
        }
    }
}
fn merge(target: &mut Option<u64>, value: Option<u64>) {
    if value.is_some() {
        *target = value;
    }
}
fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|s| !s.is_empty())
}
fn chat_output(delta: &Value) -> bool {
    nonempty(&delta["content"])
        || nonempty(&delta["reasoning_content"])
        || nonempty(&delta["reasoning"])
        || delta["tool_calls"].as_array().is_some_and(|calls| {
            calls
                .iter()
                .any(|call| nonempty(&call["function"]["arguments"]))
        })
}
