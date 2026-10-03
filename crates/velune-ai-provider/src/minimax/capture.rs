//! Final artifact guard: inspect decoded JSON strings and joined protocol payload fragments.
//! No values, lengths, hashes or error details escape this function.
use serde_json::Value;
use std::collections::BTreeMap;

fn contains(value: &Value, credential: &str, depth: usize) -> bool {
    if depth > 16 {
        return true;
    } // fail closed on pathological nested encodings
    match value {
        Value::String(text) => {
            if text.contains(credential) {
                return true;
            }
            // Tool argument strings contain embedded JSON, possibly incomplete on failure.
            // Decode each quoted JSON string, including complete literals inside partial objects.
            let bytes = text.as_bytes();
            let mut start = None;
            let mut escaped = false;
            for (i, byte) in bytes.iter().enumerate() {
                if let Some(begin) = start {
                    if escaped {
                        escaped = false;
                    } else if *byte == b'\\' {
                        escaped = true;
                    } else if *byte == b'"' {
                        if let Ok(decoded) = serde_json::from_str::<String>(&text[begin..=i])
                            && contains(&Value::String(decoded), credential, depth + 1)
                        {
                            return true;
                        }
                        start = None;
                    }
                } else if *byte == b'"' {
                    start = Some(i);
                }
            }
            if let Some(begin) = start {
                let mut literal = text[begin..].to_owned();
                literal.push('"');
                if let Ok(decoded) = serde_json::from_str::<String>(&literal)
                    && contains(&Value::String(decoded), credential, depth + 1)
                {
                    return true;
                }
            }
            false
        }
        Value::Array(values) => values.iter().any(|v| contains(v, credential, depth + 1)),
        Value::Object(values) => values
            .iter()
            .any(|(k, v)| k.contains(credential) || contains(v, credential, depth + 1)),
        _ => false,
    }
}
/// Call immediately before writing a buffered synthetic fixture. Incremental capture callbacks
/// must stay in memory until this guard accepts the entire artifact. Service payload access is
/// explicit; this is a capture guard, not a promise to redact all model content automatically.
pub fn fixture_is_safe(fixture: &Value, credential: &str) -> bool {
    if credential.is_empty() || contains(fixture, credential, 0) {
        return false;
    }
    let mut text = String::new();
    let mut tools: BTreeMap<u64, String> = BTreeMap::new();
    for record in fixture["records"].as_array().into_iter().flatten() {
        if record["kind"] != "chunk" {
            continue;
        }
        for choice in record["body"]["choices"].as_array().into_iter().flatten() {
            if let Some(fragment) = choice["delta"]["content"].as_str() {
                text.push_str(fragment);
            }
            for tool in choice["delta"]["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if let (Some(index), Some(fragment)) = (
                    tool["index"].as_u64(),
                    tool["function"]["arguments"].as_str(),
                ) {
                    tools.entry(index).or_default().push_str(fragment);
                }
            }
        }
    }
    !contains(&Value::String(text), credential, 0)
        && tools
            .into_values()
            .all(|args| !contains(&Value::String(args), credential, 0))
}
