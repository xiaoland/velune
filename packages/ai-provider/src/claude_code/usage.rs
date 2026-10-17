//! Per-CLI-turn accounting across the HTTP segments parked on caller tools.
use serde_json::{Map, Value};
use std::collections::HashMap;

const FIELDS: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];
#[derive(Clone, Copy, Default)]
struct Counter {
    value: Option<u64>,
    invalid: bool,
}
impl Counter {
    fn observe(&mut self, value: &Value, growing: bool, field: &'static str) {
        let Some(number) = value.as_u64() else {
            self.invalid = true;
            diagnostic(field, "invalid");
            return;
        };
        if self.value.is_some_and(|prior| {
            if growing {
                number < prior
            } else {
                number != prior
            }
        }) {
            self.invalid = true;
            diagnostic(field, "inconsistent");
            return;
        }
        self.value = Some(number);
    }
    fn known(self) -> Option<u64> {
        if self.invalid { None } else { self.value }
    }
}
#[derive(Default)]
struct Step {
    counters: [Counter; 4],
    output_finished: bool,
}
#[derive(Default)]
pub(super) struct TurnUsage {
    steps: HashMap<String, Step>,
    current: Option<String>,
    unidentified: bool,
    delivered: [u64; 4],
    reset: bool,
}
impl TurnUsage {
    pub fn message(&mut self, message: &Value, stream_start: bool) {
        let id = message["id"].as_str().filter(|id| !id.is_empty());
        if stream_start {
            self.current = id.map(str::to_owned);
        }
        let Some(id) = id else {
            if message["usage"].is_object() {
                self.unidentified = true;
                diagnostic("message_id", "missing");
            }
            return;
        };
        let step = self.steps.entry(id.to_owned()).or_default();
        if let Some(usage) = message["usage"].as_object() {
            // Assistant output is the message_start placeholder, never the final response count.
            for index in [0, 2, 3] {
                if let Some(value) = usage.get(FIELDS[index]) {
                    step.counters[index].observe(value, false, FIELDS[index]);
                }
            }
        }
    }
    pub fn stream_delta(&mut self, event: &Value) {
        let Some(id) = &self.current else {
            return;
        };
        let Some(step) = self.steps.get_mut(id) else {
            return;
        };
        if let Some(value) = event["usage"].get("output_tokens") {
            step.counters[1].observe(value, true, FIELDS[1]);
        }
        if event["delta"]
            .get("stop_reason")
            .is_some_and(|reason| !reason.is_null())
        {
            step.output_finished = true;
        }
    }
    pub fn stream_stop(&mut self) {
        if let Some(id) = self.current.take()
            && let Some(step) = self.steps.get_mut(&id)
        {
            step.output_finished = true;
        }
    }
    pub fn reset_observed(&mut self) {
        self.reset = true;
        diagnostic("usage", "reset");
    }
    fn step_total(&self, index: usize) -> Result<Option<u64>, ()> {
        if self.reset || self.unidentified || self.steps.is_empty() {
            return Ok(None);
        }
        let mut total = 0u64;
        for step in self.steps.values() {
            if index == 1 && !step.output_finished {
                return Ok(None);
            }
            let Some(value) = step.counters[index].known() else {
                return Ok(None);
            };
            total = total
                .checked_add(value)
                .ok_or_else(|| diagnostic(FIELDS[index], "overflow"))?;
        }
        Ok(Some(total))
    }
    pub fn tool_segment(&mut self) -> Value {
        let mut usage = Map::new();
        for (index, field) in FIELDS.into_iter().enumerate() {
            if let Ok(Some(total)) = self.step_total(index) {
                self.emit(index, field, total, &mut usage);
            }
        }
        Value::Object(usage)
    }
    pub fn final_segment(&mut self, value: &Value) -> Value {
        let mut usage = Map::new();
        if self.reset {
            return Value::Object(usage);
        }
        let Some(cumulative) = value.as_object() else {
            diagnostic("usage", "missing");
            return Value::Object(usage);
        };
        for (index, field) in FIELDS.into_iter().enumerate() {
            match cumulative.get(field) {
                Some(value) => match value.as_u64() {
                    Some(total) => match self.step_total(index) {
                        Err(()) => {}
                        Ok(Some(observed)) if total < observed => diagnostic(field, "regressed"),
                        _ => self.emit(index, field, total, &mut usage),
                    },
                    None => diagnostic(field, "invalid"),
                },
                None => diagnostic(field, "missing"),
            }
        }
        Value::Object(usage)
    }
    fn emit(
        &mut self,
        index: usize,
        field: &'static str,
        total: u64,
        usage: &mut Map<String, Value>,
    ) {
        if let Some(delta) = total.checked_sub(self.delivered[index]) {
            usage.insert(field.to_owned(), Value::from(delta));
            self.delivered[index] = total;
        } else {
            diagnostic(field, "regressed");
        }
    }
}
fn diagnostic(field: &'static str, problem: &'static str) {
    tracing::warn!(field, problem, "claude_code_usage_unavailable");
}
