use serde_json::{Value, json};
use velune_ai::{
    Payload,
    messages::{MessagesEvent, MessagesSink},
};

pub(super) struct Segment {
    pub id: String,
    enabled: bool,
    started: bool,
    open: Option<usize>,
    model: Value,
}
impl Segment {
    pub fn new(enabled: bool, model: Value) -> Self {
        Self {
            id: format!("msg_{}", super::token_id()),
            enabled,
            started: false,
            open: None,
            model,
        }
    }
    async fn frame(
        &mut self,
        sink: &mut MessagesSink,
        event: &str,
        value: Value,
    ) -> Result<(), ()> {
        if !self.enabled {
            return Ok(());
        }
        if !self.started {
            sink(MessagesEvent::Headers(super::meta()))
                .await
                .map_err(|_| ())?;
            self.started = true;
            let message = json!({"id":self.id,"type":"message","role":"assistant","model":self.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{}});
            let bytes = format!(
                "event: message_start\ndata: {}\n\n",
                json!({"type":"message_start","message":message})
            );
            sink(MessagesEvent::Body(Payload::new(bytes.into_bytes())))
                .await
                .map_err(|_| ())?;
        }
        let bytes = format!("event: {event}\ndata: {value}\n\n");
        sink(MessagesEvent::Body(Payload::new(bytes.into_bytes())))
            .await
            .map_err(|_| ())
    }
    pub async fn close(&mut self, sink: &mut MessagesSink) -> Result<(), ()> {
        if let Some(index) = self.open.take() {
            self.frame(
                sink,
                "content_block_stop",
                json!({"type":"content_block_stop","index":index}),
            )
            .await?;
        }
        Ok(())
    }
    pub async fn block(
        &mut self,
        sink: &mut MessagesSink,
        index: usize,
        block: &Value,
    ) -> Result<(), ()> {
        self.close(sink).await?;
        self.frame(
            sink,
            "content_block_start",
            json!({"type":"content_block_start","index":index,"content_block":block}),
        )
        .await?;
        self.open = Some(index);
        Ok(())
    }
    pub async fn delta(
        &mut self,
        sink: &mut MessagesSink,
        index: usize,
        delta: &Value,
    ) -> Result<(), ()> {
        self.frame(
            sink,
            "content_block_delta",
            json!({"type":"content_block_delta","index":index,"delta":delta}),
        )
        .await
    }
    pub async fn finish(&mut self, sink: &mut MessagesSink, output: &Value) -> Result<(), ()> {
        self.close(sink).await?;
        self.frame(sink,"message_delta",json!({"type":"message_delta","delta":{"stop_reason":output["stop_reason"],"stop_sequence":null},"usage":output["usage"]})).await?;
        self.frame(sink, "message_stop", json!({"type":"message_stop"}))
            .await
    }
}
