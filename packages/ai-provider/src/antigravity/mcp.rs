//! An ACP worker calls only the tools declared on the current Messages request.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::AbortHandle,
};
use velune_ai::NativeErrorKind;

pub(super) const CALL_ID_PREFIX: &str = "toolu_vagy_";

pub(super) type Tools = BTreeMap<String, Value>;

pub(super) struct Call {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub reply: oneshot::Sender<Value>,
}

pub(super) struct Bridge {
    pub endpoint: String,
    pub authorization: String,
    pub calls: mpsc::Receiver<Call>,
    server: AbortHandle,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.server.abort();
    }
}

struct Handler {
    authorization: String,
    tools: Tools,
    calls: mpsc::Sender<Call>,
}

impl Bridge {
    pub async fn start(tools: Tools) -> Result<Self, NativeErrorKind> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| NativeErrorKind::Transport)?;
        let address = listener
            .local_addr()
            .map_err(|_| NativeErrorKind::Transport)?;
        let authorization = format!("Bearer {}", super::nonce()?);
        let (requests, calls) = mpsc::channel(16);
        let handler = Handler {
            authorization: authorization.clone(),
            tools,
            calls: requests,
        };
        let app = Router::new()
            .route("/mcp", post(handle))
            .with_state(Arc::new(handler));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            endpoint: format!("http://{address}/mcp"),
            authorization,
            calls,
            server: server.abort_handle(),
        })
    }
}

async fn handle(
    State(state): State<Arc<Handler>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if headers
        .get(AUTHORIZATION)
        .is_none_or(|value| value.as_bytes() != state.authorization.as_bytes())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if request["jsonrpc"] != "2.0" || !request["method"].is_string() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(id) = request
        .get("id")
        .filter(|id| id.is_string() || id.is_number())
    else {
        return StatusCode::ACCEPTED.into_response();
    };
    let method = request["method"].as_str().unwrap();
    let result = match method {
        "initialize" => {
            // Echo a supported requested version; anything else gets the latest.
            const SUPPORTED: [&str; 3] = ["2025-03-26", "2025-06-18", "2025-11-25"];
            let version = request["params"]["protocolVersion"]
                .as_str()
                .filter(|version| SUPPORTED.contains(version))
                .unwrap_or("2025-11-25");
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "velune", "version": env!("CARGO_PKG_VERSION")}
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools": state.tools.values().collect::<Vec<_>>()})
        }
        "tools/call" => call(&state, &request["params"]).await,
        _ => {
            return Json(json!({
                "jsonrpc": "2.0","id": id,
                "error": {"code": -32601, "message": "Method not found"}
            }))
            .into_response();
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn call(state: &Handler, params: &Value) -> Value {
    const CLOSED: &str = "Caller tool session closed";
    let Some(name) = params["name"]
        .as_str()
        .filter(|name| state.tools.contains_key(*name))
    else {
        return error("Unknown caller tool");
    };
    let arguments = match params.get("arguments") {
        None => json!({}),
        Some(value) if value.is_object() => value.clone(),
        _ => return error("Tool arguments must be an object"),
    };
    let Ok(nonce) = super::nonce() else {
        return error(CLOSED);
    };
    let (reply, result) = oneshot::channel();
    let call = Call {
        id: format!("{CALL_ID_PREFIX}{nonce}"),
        name: name.to_owned(),
        arguments,
        reply,
    };
    if state.calls.try_send(call).is_err() {
        return error("Caller tool queue is unavailable");
    }
    result.await.unwrap_or_else(|_| error(CLOSED))
}

fn error(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}
