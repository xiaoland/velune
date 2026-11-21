//! Inter-agent input is persisted by Codex but omitted from its ThreadItem API.
//! Supplement only native communications in turns that API still considers active
//! history; never infer an agent input from unmarked assistant prose.
use serde_json::{Value, json};
use std::collections::HashSet;

pub(super) fn supplement(items: &mut Vec<Value>, turns: &[Value], bytes: &[u8]) {
    let valid: HashSet<&str> = turns.iter().filter_map(|t| t["id"].as_str()).collect();
    let mut seen: HashSet<String> = items
        .iter()
        .filter_map(|v| v["item"]["id"].as_str().map(str::to_owned))
        .collect();
    let mut turn = None;
    let mut marked = false;
    let mut inputs = Vec::new();
    for (ordinal, line) in bytes.split(|b| *b == b'\n').enumerate() {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let payload = &record["payload"];
        let kind = record["type"].as_str().unwrap_or_default();
        if kind == "turn_context" || (kind == "event_msg" && payload["type"] == "task_started") {
            turn = payload["turn_id"].as_str().map(str::to_owned);
        }
        if kind == "inter_agent_communication_metadata" {
            marked = true;
            continue;
        }
        let communication = if kind == "inter_agent_communication"
            || (kind == "response_item" && payload["type"] == "agent_message")
        {
            Some(payload.clone())
        } else if kind == "response_item" && marked {
            payload["content"]
                .as_array()
                .filter(|p| p.len() == 1)
                .and_then(|p| p[0]["text"].as_str())
                .and_then(|t| serde_json::from_str::<Value>(t).ok())
        } else {
            None
        };
        if kind == "response_item" {
            marked = false;
        }
        let Some(communication) = communication else {
            continue;
        };
        if !communication["author"].is_string() || !communication["recipient"].is_string() {
            continue;
        }
        let text = communication["content"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                communication["content"].as_array().map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| {
                            (p["type"] == "input_text")
                                .then(|| p["text"].as_str())
                                .flatten()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            });
        let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let turn = payload["internal_chat_message_metadata_passthrough"]["turn_id"]
            .as_str()
            .or_else(|| {
                communication["internal_chat_message_metadata_passthrough"]["turn_id"].as_str()
            })
            .or(turn.as_deref());
        let Some(turn) = turn.filter(|t| valid.contains(t)) else {
            continue;
        };
        let id = payload["id"]
            .as_str()
            .or_else(|| communication["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("codex-agent-input:{turn}:{ordinal}"));
        if !seen.insert(id.clone()) {
            continue;
        }
        let timestamp = record["timestamp"].as_str().and_then(|value| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                .ok()
                .map(|at| (at.unix_timestamp_nanos() / 1_000_000) as i64)
        });
        inputs.push(json!({"turnId":turn,"startedAtMs":timestamp,"item":{"id":id,"type":"userMessage","content":[{"type":"text","text":text}]}}));
    }
    for input in inputs {
        let turn = &input["turnId"];
        let at = input["startedAtMs"].as_i64();
        let position = items
            .iter()
            .position(|item| {
                item["turnId"] == *turn
                    && at
                        .zip(item["startedAtMs"].as_i64())
                        .is_none_or(|(input, item)| item >= input)
            })
            .or_else(|| {
                items
                    .iter()
                    .rposition(|item| item["turnId"] == *turn)
                    .map(|i| i + 1)
            })
            .unwrap_or(items.len());
        items.insert(position, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_native_agent_inputs_in_surviving_turns_are_projected() {
        let mut items = vec![
            json!({"turnId":"kept","item":{"id":"answer","type":"agentMessage","text":"done"}}),
        ];
        let records = [
            json!({"type":"turn_context","payload":{"turn_id":"removed"}}),
            json!({"type":"inter_agent_communication","payload":{"id":"old","author":"/root","recipient":"/root/child","content":"removed input"}}),
            json!({"type":"turn_context","payload":{"turn_id":"kept"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"{\"author\":\"/root\",\"recipient\":\"/root/child\",\"content\":\"not marked\"}"}]}}),
            json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}),
            json!({"type":"response_item","payload":{"id":"input","type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":"parent task"}]}}),
            json!({"type":"response_item","payload":{"id":"input","type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":"parent task"}]}}),
        ];
        let bytes = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        supplement(&mut items, &[json!({"id":"kept"})], bytes.as_bytes());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["item"]["content"][0]["text"], "parent task");
        assert_eq!(items[1]["item"]["id"], "answer");
    }
}
