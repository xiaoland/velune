//! Gemini GenerateContent transport and response facts. Requests and response
//! bytes remain native even when a caller also observes their terminal state.
use crate::{config::HttpEndpoint, metrics::ModelTerminal};
use serde_json::Value;
use std::collections::BTreeMap;
use velune_ai::{NativeErrorKind, ProviderModelId};

pub(crate) fn url(
    endpoint: &HttpEndpoint,
    model: &ProviderModelId,
    stream: bool,
) -> Result<String, NativeErrorKind> {
    let name = model
        .as_str()
        .strip_prefix("models/")
        .unwrap_or(model.as_str());
    // The model is a single resource name within the configured endpoint.
    // A path, query, percent escape, or verb cannot come from the model ID.
    if name.is_empty()
        || !name.bytes().any(|byte| byte.is_ascii_alphanumeric())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(NativeErrorKind::InvalidInput);
    }
    let operation = if stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    Ok(endpoint.url(&format!("/models/{name}:{operation}")))
}

#[derive(Default)]
pub(crate) struct ResponseObservation {
    candidates: BTreeMap<u64, Option<ModelTerminal>>,
    failed: bool,
}

impl ResponseObservation {
    pub(crate) fn observe(&mut self, value: &Value) {
        self.failed |= value.get("error").is_some_and(|error| !error.is_null())
            || value["type"] == "error"
            || value["promptFeedback"]["blockReason"]
                .as_str()
                .is_some_and(|reason| !matches!(reason, "" | "BLOCK_REASON_UNSPECIFIED"));
        for (index, candidate) in candidates(value).enumerate() {
            let index = candidate["index"].as_u64().unwrap_or(index as u64);
            let prior = self.candidates.entry(index).or_default();
            if let Some(reason) = candidate["finishReason"].as_str().and_then(finish_reason) {
                *prior = Some(prior.map_or(reason, |prior| strongest(prior, reason)));
            }
        }
    }

    pub(crate) fn terminal(&self) -> Option<ModelTerminal> {
        if self.failed {
            return Some(ModelTerminal::Failed);
        }
        if self.candidates.is_empty() {
            return None;
        }
        self.candidates
            .values()
            .try_fold(ModelTerminal::Completed, |outcome, candidate| {
                candidate.map(|terminal| strongest(outcome, terminal))
            })
    }
}

fn finish_reason(reason: &str) -> Option<ModelTerminal> {
    Some(match reason {
        "" | "FINISH_REASON_UNSPECIFIED" => return None,
        "STOP" => ModelTerminal::Completed,
        "MAX_TOKENS" | "CONTINUATION" => ModelTerminal::Incomplete,
        _ => ModelTerminal::Failed,
    })
}

fn strongest(left: ModelTerminal, right: ModelTerminal) -> ModelTerminal {
    match (left, right) {
        (ModelTerminal::Failed, _) | (_, ModelTerminal::Failed) => ModelTerminal::Failed,
        (ModelTerminal::Incomplete, _) | (_, ModelTerminal::Incomplete) => {
            ModelTerminal::Incomplete
        }
        _ => ModelTerminal::Completed,
    }
}

fn candidates(value: &Value) -> impl Iterator<Item = &Value> {
    value["candidates"].as_array().into_iter().flatten()
}

fn parts(candidate: &Value) -> impl Iterator<Item = &Value> {
    candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
}

pub(crate) fn has_output(value: &Value) -> bool {
    candidates(value).flat_map(parts).any(|part| {
        part["text"].as_str().is_some_and(|text| !text.is_empty())
            || part["functionCall"].is_object()
            || part["inlineData"].is_object()
            || part["executableCode"].is_object()
            || part["codeExecutionResult"].is_object()
    })
}

pub(crate) fn text(value: &Value) -> impl Iterator<Item = &str> {
    candidates(value)
        .enumerate()
        .find(|(index, candidate)| candidate["index"].as_u64().unwrap_or(*index as u64) == 0)
        .into_iter()
        .flat_map(|(_, candidate)| parts(candidate))
        .filter(|part| part["thought"] != true)
        .filter_map(|part| part["text"].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_resource_cannot_select_a_path_query_or_operation() {
        let endpoint = HttpEndpoint::parse("http://127.0.0.1:4242/v1beta").unwrap();
        let model = |value: &str| ProviderModelId::new(value).unwrap();
        assert_eq!(
            url(&endpoint, &model("models/gemini-3-flash"), true).unwrap(),
            "http://127.0.0.1:4242/v1beta/models/gemini-3-flash:streamGenerateContent?alt=sse"
        );
        for value in [
            "models/../gemini",
            "models/",
            "../gemini",
            "gemini%2Fother",
            "gemini?key=value",
            "gemini:predict",
        ] {
            assert_eq!(
                url(&endpoint, &model(value), false),
                Err(NativeErrorKind::InvalidInput)
            );
        }
    }

    #[test]
    fn usage_does_not_complete_an_unfinished_candidate() {
        let mut metrics = crate::metrics::ResponseMetrics::new(velune_ai::Protocol::Gemini);
        for record in [
            json!({
                "candidates":[{"index":0,"content":{"parts":[{"text":"first"}]}},
                              {"index":1,"content":{"parts":[{"text":"second"}]}}]
            }),
            json!({
                "candidates":[{"index":0,"finishReason":"STOP"}],
                "usageMetadata":{"promptTokenCount":12,"cachedContentTokenCount":8,"candidatesTokenCount":7}
            }),
            json!({"usageMetadata":{"thoughtsTokenCount":3,"toolUsePromptTokenCount":2}}),
        ] {
            metrics.stream_chunk(format!("data: {record}\n\n").as_bytes());
        }
        metrics.stream_end();
        assert!(metrics.first_output);
        assert_eq!(metrics.terminal, None);
        let usage = metrics.usage();
        assert_eq!(usage.input, Some(14));
        assert_eq!(usage.uncached_input, Some(6));
        assert_eq!(usage.cached_input, Some(8));
        assert_eq!(usage.output, Some(10));
        assert_eq!(usage.reasoning_output, Some(3));
    }

    #[test]
    fn blocked_prompt_is_a_failed_model_terminal() {
        let mut observed = ResponseObservation::default();
        observed.observe(&json!({
            "promptFeedback":{"blockReason":"SAFETY"},
            "usageMetadata":{"promptTokenCount":17}
        }));
        assert_eq!(observed.terminal(), Some(ModelTerminal::Failed));
    }
}
