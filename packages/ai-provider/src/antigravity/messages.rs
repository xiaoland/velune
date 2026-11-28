//! Project Messages requests and ACP chunks across the native provider boundary.
use super::mcp;
use crate::claude_code::stream::Segment;
use serde_json::{Value, json};
use velune_ai::{
    NativeCompletion, NativeErrorKind as Error, NativeFailure, NativeOutput, NativeSink,
    NativeSuccess, Payload,
    http::{Header, ResponseBody, ResponseMeta},
};

pub(super) struct Request {
    history: Vec<Value>,
    pub tools: mcp::Tools,
    pub system: String,
    pub model: String,
}

impl Request {
    pub fn parse(body: Value, model: &str) -> Result<Self, Error> {
        if body["model"] != model {
            return Err(Error::InvalidInput);
        }
        let system = match &body["system"] {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            Value::Array(blocks) => {
                let blocks: Result<Vec<_>, _> = blocks.iter().map(text).collect();
                blocks?.join("\n")
            }
            _ => return Err(Error::InvalidInput),
        };
        let mut history = Vec::new();
        for message in body["messages"].as_array().ok_or(Error::InvalidInput)? {
            let role = message["role"]
                .as_str()
                .filter(|role| matches!(*role, "user" | "assistant"))
                .ok_or(Error::InvalidInput)?;
            history.push(json!({"role": role, "content": blocks(&message["content"])?}));
        }
        if history
            .last()
            .is_none_or(|message| message["role"] != "user")
        {
            return Err(Error::InvalidInput);
        }
        let mut tools = mcp::Tools::new();
        if let Some(entries) = body.get("tools") {
            for entry in entries.as_array().ok_or(Error::InvalidInput)? {
                let name = entry["name"]
                    .as_str()
                    .filter(|name| {
                        !name.is_empty()
                            && name.len() <= 64
                            && name
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                    })
                    .ok_or(Error::InvalidInput)?;
                if !entry["input_schema"].is_object() {
                    return Err(Error::Unsupported);
                }
                let tool = json!({
                    "name": name,
                    "description": entry["description"].as_str().unwrap_or(""),
                    "inputSchema": entry["input_schema"]
                });
                if tools.insert(name.to_owned(), tool).is_some() {
                    return Err(Error::InvalidInput);
                }
            }
        }
        if let Some(choice) = body.get("tool_choice").filter(|choice| !choice.is_null()) {
            match choice["type"].as_str() {
                Some("auto") => {}
                Some("none") => tools.clear(),
                // ACP doesn't expose constrained tool selection for the model.
                Some(_) => return Err(Error::Unsupported),
                None => return Err(Error::InvalidInput),
            }
        }
        Ok(Self {
            history,
            tools,
            system,
            model: model.to_owned(),
        })
    }

    pub fn is_tool_result(&self) -> bool {
        self.tail()
            .iter()
            .any(|block| block["type"] == "tool_result")
    }

    fn tail(&self) -> &[Value] {
        self.history.last().unwrap()["content"].as_array().unwrap()
    }

    pub fn tool_result(&self, id: &str) -> Result<Value, Error> {
        let [block] = self.tail() else {
            return Err(Error::InvalidInput);
        };
        if block["type"] != "tool_result" || block["tool_use_id"] != id {
            return Err(Error::InvalidInput);
        }
        let content = match &block["content"] {
            Value::Null => Vec::new(),
            value => blocks(value)?
                .iter()
                .map(media)
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(json!({"content": content,"isError": block["is_error"].as_bool().unwrap_or(false)}))
    }

    pub fn prompt(&self) -> Result<Vec<Value>, Error> {
        let (latest, history) = self.history.split_last().unwrap();
        let mut prompt = Vec::new();
        if !history.is_empty() {
            prompt.push(json!({
                "type": "text",
                "text": "Earlier caller conversation, in order. The final user message follows this history."
            }));
            for message in history {
                prompt.push(json!({
                    "type":"text",
                    "text": format!("\n{} message:", message["role"].as_str().unwrap())
                }));
                for block in message["content"].as_array().unwrap() {
                    match block["type"].as_str() {
                        Some("tool_use" | "tool_result") => {
                            // ACP has no prompt block for historical tool context.
                            prompt.push(json!({"type":"text","text":block.to_string()}));
                        }
                        Some("thinking" | "redacted_thinking") => {}
                        _ => prompt.push(media(block)?),
                    }
                }
            }
            prompt.push(json!({"type":"text","text":"\nLatest user message:"}));
        }
        for block in latest["content"].as_array().unwrap() {
            prompt.push(media(block)?);
        }
        if prompt.is_empty() {
            return Err(Error::InvalidInput);
        }
        Ok(prompt)
    }
}

fn blocks(value: &Value) -> Result<Vec<Value>, Error> {
    match value {
        Value::String(text) => Ok(vec![json!({"type": "text", "text": text})]),
        Value::Array(content) => Ok(content.clone()),
        _ => Err(Error::InvalidInput),
    }
}

fn text(block: &Value) -> Result<&str, Error> {
    if block["type"] != "text" {
        return Err(Error::Unsupported);
    }
    block["text"].as_str().ok_or(Error::InvalidInput)
}

fn media(block: &Value) -> Result<Value, Error> {
    match block["type"].as_str() {
        Some("text") => Ok(json!({"type": "text", "text": text(block)?})),
        Some("image") if block["source"]["type"] == "base64" => {
            let mime = block["source"]["media_type"]
                .as_str()
                .ok_or(Error::InvalidInput)?;
            let data = block["source"]["data"]
                .as_str()
                .ok_or(Error::InvalidInput)?;
            Ok(json!({"type":"image","mimeType":mime,"data":data}))
        }
        _ => Err(Error::Unsupported),
    }
}

pub(super) struct Reply {
    segment: Segment,
    content: Vec<Value>,
    stream: bool,
    model: String,
}

impl Reply {
    pub fn new(stream: bool, model: String) -> Self {
        Self {
            segment: Segment::new(stream, json!(model)),
            content: Vec::new(),
            stream,
            model,
        }
    }

    pub async fn chunk(&mut self, update: &Value, sink: &mut NativeSink) -> Result<(), Error> {
        // ACP thoughts have no Messages signatures to return or replay.
        if update["sessionUpdate"] == "agent_thought_chunk" {
            return Ok(());
        }
        let block = &update["content"];
        if block["type"] == "image" {
            let mime = block["mimeType"].as_str().ok_or(Error::ProviderFailure)?;
            let data = block["data"].as_str().ok_or(Error::ProviderFailure)?;
            let image =
                json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}});
            self.segment
                .block(sink, self.content.len(), &image)
                .await
                .map_err(|_| Error::Cancelled)?;
            self.content.push(image);
            return Ok(());
        }
        let text = text(block).map_err(|_| Error::Unsupported)?;
        if text.is_empty() {
            return Ok(());
        }
        if self
            .content
            .last()
            .is_none_or(|block| block["type"] != "text")
        {
            let block = json!({"type": "text", "text": ""});
            self.segment
                .block(sink, self.content.len(), &block)
                .await
                .map_err(|_| Error::Cancelled)?;
            self.content.push(block);
        }
        let index = self.content.len() - 1;
        let mut previous = std::mem::take(&mut self.content[index]["text"]);
        let Value::String(ref mut full_text) = previous else {
            unreachable!()
        };
        full_text.push_str(text);
        self.content[index]["text"] = previous;
        self.segment
            .delta(sink, index, &json!({"type":"text_delta","text":text}))
            .await
            .map_err(|_| Error::Cancelled)
    }

    pub async fn tool(&mut self, call: &mcp::Call, sink: &mut NativeSink) -> Result<(), Error> {
        let index = self.content.len();
        let mut block = json!({"type":"tool_use","id":call.id,"name":call.name,"input":{}});
        self.segment
            .block(sink, index, &block)
            .await
            .map_err(|_| Error::Cancelled)?;
        self.segment
            .delta(
                sink,
                index,
                &json!({"type":"input_json_delta","partial_json":call.arguments.to_string()}),
            )
            .await
            .map_err(|_| Error::Cancelled)?;
        block["input"] = call.arguments.clone();
        self.content.push(block);
        Ok(())
    }

    pub async fn finish(mut self, stop: &str, sink: &mut NativeSink) -> NativeCompletion {
        // ACP's cumulative context use is not per-request token accounting.
        let message = json!({
            "id": self.segment.id, "type": "message", "role": "assistant", "model": self.model,
            "content": self.content, "stop_reason": stop, "stop_sequence": null, "usage": {}
        });
        let content_type = if self.stream {
            "text/event-stream"
        } else {
            "application/json"
        };
        let meta = ResponseMeta {
            status: 200,
            headers: vec![Header {
                name: "content-type".into(),
                value: content_type.into(),
            }],
        };
        let output = if self.stream {
            self.segment
                .finish(sink, &message)
                .await
                .map_err(|_| NativeFailure::sent(Error::Cancelled, None))?;
            NativeOutput::Stream(meta)
        } else {
            NativeOutput::Json(ResponseBody {
                meta,
                body: Payload::new(serde_json::to_vec(&message).unwrap()),
            })
        };
        Ok(NativeSuccess {
            output,
            completed: stop == "end_turn" && !self.content.is_empty(),
        })
    }
}
