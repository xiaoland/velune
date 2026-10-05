//! Explicit manual acceptance entry, not a test runner. Two fixed synthetic cases plus one explicitly authorized text diagnostic.
//! replay constructs no HTTP client and never reads credential/proxy environment variables.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use velune_ai::{
    OperationFuture, Payload, SamplingService, direct::DirectSamplingService, ids::*,
    observation::*, provider::*, sampling::*,
};
use velune_ai_provider::{
    config::*,
    minimax::{self, Decoder, MiniMax, ProtocolRecord},
};

type Result<T> = std::result::Result<T, &'static str>;
const ROOT: &str = "fixtures/ai/minimax-m3";
fn checked<T>(value: std::result::Result<T, impl std::fmt::Debug>) -> Result<T> {
    value.map_err(|_| "operation failed; raw details suppressed")
}
fn input(case: &str) -> Result<SamplingInput> {
    let (prompt, tools, max) = match case {
        "text" | "text-diagnostic" => ("Reply exactly: SYNTHETIC_OK", vec![], 64),
        "tool" => (
            "Call the synthetic add_numbers function exactly once with a=2 and b=3. Do not calculate or explain; only return the function call.",
            vec![checked(ToolDefinition::new(
                checked(ToolName::new("add_numbers"))?,
                Payload::new(
                    "Synthetic arithmetic fixture; caller will not execute this tool.".into(),
                ),
                json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"],"additionalProperties":false}),
            ))?],
            256,
        ),
        _ => return Err("case must be text, text-diagnostic or tool"),
    };
    checked(SamplingInput::new(
        None,
        vec![Message::User(Payload::new(prompt.into()))],
        tools,
        max,
    ))
}
fn provider_id() -> ProviderId {
    ProviderId::new("minimax-cn").expect("constant")
}
fn model_id() -> ProviderModelId {
    ProviderModelId::new(minimax::MODEL).expect("constant")
}
fn revision() -> ConfigRevision {
    ConfigRevision::new(1).expect("constant")
}
fn request(case: &str) -> Result<SamplingRequest> {
    Ok(SamplingRequest {
        call_id: checked(CallId::new(format!("synthetic-{case}-call-1")))?,
        provider: provider_id(),
        model: model_id(),
        input: input(case)?,
    })
}
fn context_json(context: &AttemptContext) -> Value {
    json!({"call":context.call.as_str(),"attempt":context.attempt.as_str(),"provider":context.provider.as_str(),"model":context.model.as_str(),"revision":context.revision.get()})
}
fn quantity(value: Quantity<u64>) -> Value {
    match value {
        Quantity::Unknown => json!({"kind":"unknown"}),
        Quantity::Reported(value) => json!({"kind":"reported","value":value}),
        Quantity::Estimated(value) => json!({"kind":"estimated","value":value}),
    }
}
fn usage_json(usage: Usage) -> Value {
    json!({"input_tokens":quantity(usage.input_tokens),"output_tokens":quantity(usage.output_tokens)})
}
fn event_json(event: SamplingEvent) -> Value {
    let body = match event.kind {
        SamplingEventKind::Delta(delta) => match delta {
            SamplingDelta::Text(text) => json!({"kind":"text","fragment":text.get()}),
            SamplingDelta::ToolIdentity { index, id, name } => {
                json!({"kind":"tool_identity","index":index,"id":id.as_ref().map(ToolCallId::as_str),"name":name.as_ref().map(ToolName::as_str)})
            }
            SamplingDelta::ToolArguments { index, fragment } => {
                json!({"kind":"tool_arguments","index":index,"fragment":fragment.get()})
            }
            SamplingDelta::Finish(reason) => {
                json!({"kind":"finish","reason":format!("{reason:?}")})
            }
            SamplingDelta::Usage(usage) => json!({"kind":"usage","usage":usage_json(usage)}),
        },
        SamplingEventKind::Terminal { error } => {
            json!({"kind":"terminal","error":error.map(|e| format!("{e:?}"))})
        }
    };
    json!({"context":context_json(&event.context),"sequence":event.sequence,"event":body})
}
fn result_json(completion: &SamplingCompletion) -> Value {
    let outcome = match &completion.outcome.result {
        Ok(output) => {
            json!({"ok":true,"finish":format!("{:?}",output.finish),"text":output.text.as_ref().map(Payload::get),"tools":output.tool_calls.iter().map(|tool| json!({"id":tool.id.as_str(),"name":tool.name.as_str(),"arguments":tool.arguments.get()})).collect::<Vec<_>>()})
        }
        Err(failure) => {
            json!({"ok":false,"kind":format!("{:?}",failure.kind),"execution":format!("{:?}",failure.execution),"partial":failure.partial.as_ref().map(|partial| json!({"text":partial.text.as_ref().map(Payload::get),"tools":partial.tool_calls.iter().map(|t| json!({"index":t.index,"id":t.id.as_ref().map(ToolCallId::as_str),"name":t.name.as_ref().map(ToolName::as_str),"arguments_fragment":t.arguments.get()})).collect::<Vec<_>>()}))})
        }
    };
    json!({"call":completion.call.call.as_str(),"revision":completion.call.revision.get(),"usage":usage_json(completion.outcome.usage),"outcome":outcome})
}
fn record_json(record: ProtocolRecord) -> Value {
    match record {
        ProtocolRecord::Request(body) => json!({"kind":"request","body":body.get()}),
        ProtocolRecord::Status(status) => json!({"kind":"status","status":status}),
        ProtocolRecord::Chunk(body) => json!({"kind":"chunk","body":body.get()}),
        ProtocolRecord::Done => json!({"kind":"done"}),
        ProtocolRecord::StreamEnd {
            classification,
            framing,
        } => {
            json!({"kind":"stream_end","classification":format!("{classification:?}"),"framing":{"done_lines":framing.done_lines,"done_frames":framing.done_frames,"unfinished_data_frame":framing.unfinished_data_frame,"unfinished_line":framing.unfinished_line,"unfinished_line_is_done":framing.unfinished_line_is_done}})
        }
    }
}
struct Replay {
    records: Vec<Value>,
}
impl SamplingProvider for Replay {
    fn sampling(
        &self,
        request: ProviderSamplingRequest,
        mut events: ProviderSamplingSink,
    ) -> OperationFuture<ProviderSamplingOutcome> {
        let records = self.records.clone();
        Box::pin(async move {
            let mut decoder = Decoder::new();
            let mut done = false;
            let mut request_seen = false;
            let mut status_seen = false;
            let mut termination = None;
            for record in records {
                let execution = if status_seen {
                    ExecutionKnowledge::Accepted
                } else {
                    ExecutionKnowledge::NotSent
                };
                if termination.is_some() {
                    return decoder.failure(SamplingErrorKind::ProviderFailure, execution, false);
                }
                match record["kind"].as_str() {
                    Some("request") if !request_seen && !status_seen => {
                        request_seen = true;
                        if !minimax::request_json(&request.input).is_ok_and(|v| v == record["body"])
                        {
                            return decoder.failure(
                                SamplingErrorKind::InvalidInput,
                                execution,
                                false,
                            );
                        }
                    }
                    Some("status") if request_seen && !status_seen => {
                        if record["status"] != 200 {
                            let kind = match record["status"].as_u64() {
                                Some(401) => SamplingErrorKind::Authentication,
                                Some(403) => SamplingErrorKind::Permission,
                                Some(429) => SamplingErrorKind::RateLimited,
                                _ => SamplingErrorKind::ProviderFailure,
                            };
                            return decoder.failure(kind, ExecutionKnowledge::Unknown, false);
                        }
                        status_seen = true;
                    }
                    Some("chunk") if status_seen && !done => {
                        if let Err(kind) = decoder.push(&record["body"], &mut events) {
                            return decoder.failure(kind, ExecutionKnowledge::Accepted, false);
                        }
                    }
                    Some("done") if status_seen && !done => {
                        done = true;
                    }
                    Some("stream_end") if status_seen => {
                        termination = Some(record);
                    }
                    _ => {
                        return decoder.failure(
                            SamplingErrorKind::ProviderFailure,
                            execution,
                            false,
                        );
                    }
                }
            }
            let mut result = match termination.as_ref().map(|r| r["classification"].as_str()) {
                Some(Some("CleanEofWithoutDone")) if !done => {
                    let frame = &termination.as_ref().expect("present")["framing"];
                    let framing = minimax::FramingEvidence {
                        done_lines: frame["done_lines"]
                            .as_u64()
                            .and_then(|v| v.try_into().ok())
                            .unwrap_or(u32::MAX),
                        done_frames: frame["done_frames"]
                            .as_u64()
                            .and_then(|v| v.try_into().ok())
                            .unwrap_or(u32::MAX),
                        unfinished_data_frame: frame["unfinished_data_frame"]
                            .as_bool()
                            .unwrap_or(true),
                        unfinished_line: frame["unfinished_line"].as_bool().unwrap_or(true),
                        unfinished_line_is_done: frame["unfinished_line_is_done"]
                            .as_bool()
                            .unwrap_or(true),
                    };
                    decoder.clean_eof(framing)
                }
                Some(Some("DoneDecoded")) if done => decoder.complete(),
                None if done => decoder.complete(),
                Some(Some("Timeout")) => decoder.failure(
                    SamplingErrorKind::Timeout,
                    ExecutionKnowledge::Accepted,
                    false,
                ),
                Some(Some(
                    "Transport" | "Body" | "Decode" | "SizeLimit" | "SseParser" | "Utf8",
                )) => decoder.failure(
                    SamplingErrorKind::Transport,
                    ExecutionKnowledge::Accepted,
                    false,
                ),
                None => decoder.failure(
                    SamplingErrorKind::Transport,
                    if status_seen {
                        ExecutionKnowledge::Accepted
                    } else {
                        ExecutionKnowledge::NotSent
                    },
                    false,
                ),
                _ => decoder.failure(
                    SamplingErrorKind::ProviderFailure,
                    if status_seen {
                        ExecutionKnowledge::Accepted
                    } else {
                        ExecutionKnowledge::NotSent
                    },
                    false,
                ),
            };
            result.submitted = false; // replay has no actual transport submission
            result
        })
    }
}
async fn run(
    case: &str,
    provider: Arc<dyn SamplingProvider>,
) -> Result<(SamplingCompletion, Vec<Value>)> {
    let binding = checked(ProviderBinding::new(
        provider_id(),
        revision(),
        vec![model_id()],
        provider,
    ))?;
    let service = DirectSamplingService::new(Arc::new(binding));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let future = checked(service.sampling(
        request(case)?,
        checked(AttemptId::new(format!("synthetic-{case}-attempt-1")))?,
        Box::new(move |event| sink.lock().expect("sink").push(event_json(event))),
    ))?;
    let result = future.await;
    let events = events.lock().expect("sink").clone();
    Ok((result, events))
}
fn write_json(path: &str, value: &Value) -> Result<()> {
    checked(fs::write(path, checked(serde_json::to_vec_pretty(value))?))
}
async fn live(case: &str, source: &str) -> Result<()> {
    if source.len() != 40 || !source.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("source commit must be a full SHA");
    }
    if case == "tool"
        && !std::path::Path::new(&format!("{ROOT}/text-diagnostic.live.json")).exists()
    {
        return Err("reviewed diagnostic required before tool capture");
    }
    let body = checked(minimax::request_json(&input(case)?))?;
    if body.to_string().len() > 4096 {
        return Err("request budget bound exceeded");
    }
    checked(fs::create_dir_all(ROOT))?;
    let mut reserved_milli_cny = 900_u64; // historical 401: 800, prior auth: 100
    for prior in ["text", "text-diagnostic", "tool"] {
        let prior_path = format!("{ROOT}/{prior}.live.json");
        if std::path::Path::new(&prior_path).exists() {
            let old: Value = checked(serde_json::from_slice(&checked(fs::read(prior_path))?))?;
            reserved_milli_cny += 500; // never release on estimated cost
            let approved_diagnostic_predecessor = prior == "text"
                && ["text-diagnostic", "tool"].contains(&case)
                && old["source_commit"] == "abd05a2cea3f0a7545005778c030bc78b5b9890f"
                && old["mapped"]["outcome"]["kind"] == "Transport"
                && old["mapped"]["outcome"]["execution"] == "Accepted";
            let reviewed_diagnostic = if prior == "text-diagnostic" && case == "tool" {
                let expected: Value = checked(serde_json::from_slice(&checked(fs::read(
                    format!("{ROOT}/text-diagnostic.expected.json"),
                ))?))?;
                let (completion, events) = run(
                    "text-diagnostic",
                    Arc::new(Replay {
                        records: old["records"]
                            .as_array()
                            .ok_or("missing diagnostic records")?
                            .clone(),
                    }),
                )
                .await?;
                old["source_commit"] == "d7439bdd67b6a2810d25e8f7e16c6632011eab99"
                    && completion.outcome.result.is_ok()
                    && result_json(&completion) == expected["mapped"]
                    && json!(events) == expected["events"]
            } else {
                false
            };
            if old["admission"] != "completed"
                || (old["mapped"]["outcome"]["ok"] != true
                    && !approved_diagnostic_predecessor
                    && !reviewed_diagnostic)
                || old["mapped"]["usage"]["input_tokens"]["kind"] != "reported"
                || old["mapped"]["usage"]["output_tokens"]["kind"] != "reported"
            {
                return Err("previous attempt incomplete/unknown; stop new dispatch");
            }
        }
    }
    if reserved_milli_cny + 500 > 5000 {
        return Err("global reservation budget exhausted");
    }

    // Three fixed admissions, create_new before credential resolution or dispatch. A pending/crashed
    // admission remains charged at its entire reservation. No overwrite, reset, retry, or resume.
    let path = format!("{ROOT}/{case}.live.json");
    let mut file = checked(OpenOptions::new().write(true).create_new(true).open(&path))?;
    let capture_time = checked(SystemTime::now().duration_since(UNIX_EPOCH))?.as_secs();
    let mut fixture = json!({"schema":1,"source":"live","protocol":"minimax-chat-completions-sse","adapter_version":"0.1.0","requested_model":minimax::MODEL,"source_commit":source,"baseline_commit":"7c6f98267a56101d2117311adeaf10b50b01d8f2","capture_unix_seconds":capture_time,"case":case,"admission":"pending_unknown","budget":{"currency":"CNY","global_limit":5.0,"historical_401_usage":"unknown","historical_401_reserve":0.8,"prior_auth_tokens":{"input":165,"output":2},"prior_auth_reserve":0.1,"this_attempt_reserve":0.5,"reservation_total_after_admission_milli_cny":reserved_milli_cny+500,"standard_price_cny_per_million":{"input":2.1,"output":8.4},"reservation_rate_cny_per_million":{"input":8.4,"output":33.6},"input_token_allowance_estimated":8192,"output_token_bound":1024,"authorized_new_attempt_limit":8,"entry_point_live_attempt_limit":3,"maximum_reserved_by_this_entry":2.4},"redaction":{"mode":"allowlist-before-write","removed":["headers","cookies","credentials","response_id","created","system_fingerprint","raw_errors","unrecognized_field_values"],"retained":["synthetic_request","model","choices.index","delta.role","delta.content","delta.tool_calls","finish_reason","prompt_tokens","completion_tokens","total_tokens","usage_presence","unsupported_usage_presence","SSE_done"],"usage_unmapped":"additional fields represented by presence only; no invented zeros"}});
    checked(file.write_all(&checked(serde_json::to_vec_pretty(&fixture))?))?;
    checked(file.sync_all())?;
    drop(file);
    // The only credential read: explicitly authorized Networksecret placeholder, in app main.
    let credential = checked(std::env::var("MINIMAX_API_KEY"))?;
    let client = checked(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http1_only()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .connection_verbose(false)
            .build(),
    )?;
    let config = checked(ProviderConfig::new(
        provider_id(),
        revision(),
        ProtocolConfig::ChatCompletions(ChatCompletionsConfig {
            endpoint: checked(HttpEndpoint::new(
                Transport::Https,
                "api.minimax.cn",
                443,
                "/v1",
            ))?,
        }),
        checked(CredentialRef::new("MINIMAX_API_KEY"))?,
    ))?;
    let records = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&records);
    let provider = checked(MiniMax::new(
        config,
        client,
        Payload::new(credential.clone()),
        Some(Arc::new(move |record| {
            sink.lock().expect("capture").push(record_json(record))
        })),
    ))?;
    let (completion, events) = run(case, Arc::new(provider)).await?;
    fixture["admission"] = json!("completed");
    fixture["actual_http_attempts"] = json!(completion.call.attempts);
    fixture["records"] = json!(*records.lock().expect("capture"));
    fixture["mapped"] = result_json(&completion);
    fixture["events"] = json!(events);
    fixture["attempt"] = completion.attempt.as_ref().map(|a| json!({"context":context_json(&a.context),"execution":format!("{:?}",a.execution),"error":a.error.map(|e| format!("{e:?}")),"usage":usage_json(a.usage)})).unwrap_or(Value::Null);
    if !minimax::fixture_is_safe(&fixture, &credential) {
        // Persist fixed metadata and typed counts only. Never serialize the rejected fixture,
        // its fragments, tool identities, decoded strings, or raw errors on this path.
        write_json(
            &path,
            &json!({
                "schema":1,"source":"live","case":case,"source_commit":source,
                "capture_unix_seconds":capture_time,"admission":"capture_rejected",
                "diagnostic":"content_unsafe_or_undecodable",
                "actual_http_attempts":completion.call.attempts,
                "provider_result":if completion.outcome.result.is_ok() { "completed" } else { "failed" },
                "usage":usage_json(completion.outcome.usage),"budget":fixture["budget"]
            }),
        )?;
        return Err("capture rejected: unsafe or undecodable content; reservation retained");
    }
    write_json(&path, &fixture)?;
    println!(
        "{}",
        json!({"source":"live","case":case,"attempts":completion.call.attempts,"ok":completion.outcome.result.is_ok(),"usage":usage_json(completion.outcome.usage),"fixture":path})
    );
    if completion.outcome.result.is_err() {
        return Err("live attempt failed; stop, no retry");
    }
    if !matches!(
        (
            completion.outcome.usage.input_tokens,
            completion.outcome.usage.output_tokens
        ),
        (Quantity::Reported(_), Quantity::Reported(_))
    ) {
        return Err("usage incomplete; retain reservation and stop");
    }
    Ok(())
}
async fn replay(case: &str) -> Result<()> {
    let fixture: Value = checked(serde_json::from_slice(&checked(fs::read(format!(
        "{ROOT}/{case}.live.json"
    )))?))?;
    let expected: Value = checked(serde_json::from_slice(&checked(fs::read(format!(
        "{ROOT}/{case}.expected.json"
    )))?))?;
    let records = fixture["records"]
        .as_array()
        .ok_or("missing records")?
        .clone();
    let (completion, events) = run(case, Arc::new(Replay { records })).await?;
    let mapped = result_json(&completion);
    if completion.call.attempts != 0 || completion.attempt.is_some() {
        return Err("replay must have no HTTP attempt");
    }
    if mapped != expected["mapped"] || json!(events) != expected["events"] {
        return Err("independently reviewed mapping mismatch");
    }
    println!(
        "{}",
        json!({"source":"replay","case":case,"network_attempts":0,"credential_reads":0,"events":events.len(),"mapping_matches_reviewed_expected":true,"usage":usage_json(completion.outcome.usage)})
    );
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("live") if args.len() == 4 => live(&args[2], &args[3]).await,
        Some("replay")
            if args.len() == 3
                && ["text", "text-diagnostic", "tool"].contains(&args[2].as_str()) =>
        {
            replay(&args[2]).await
        }
        _ => Err(
            "usage: minimax_manual live text|text-diagnostic|tool SOURCE_COMMIT | replay text|text-diagnostic|tool",
        ),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
