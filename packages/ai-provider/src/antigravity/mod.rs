//! Native Messages execution through the signed-in Antigravity ACP CLI.
//! Account selection and process preparation belong to the application.
mod acp;
mod mcp;
mod messages;
mod pool;
mod process;

use crate::claude_code::AbortOnDrop;
pub use acp::{Model, probe_models};
pub use pool::AntigravityPool;
use std::{process::Command, sync::Arc, time::Duration};
use velune_ai::{
    InvalidContract, NativeCompletion, NativeErrorKind as Error, NativeFailure, NativeProvider,
    NativeRequest, NativeSink, OperationFuture,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const SEGMENT_TIMEOUT: Duration = Duration::from_secs(1800);

/// Prepare a single account's ACP command and translate a loopback MCP endpoint
/// into the execution target's network namespace if needed.
pub trait Launch: Send + Sync {
    fn prepare(&self, loopback_mcp: String) -> OperationFuture<Result<Process, Error>>;
}

pub struct Process {
    pub command: Command,
    /// Optional target startup input, written before the first ACP frame.
    pub input_prefix: Vec<u8>,
    pub cwd: String,
    pub mcp_endpoint: String,
    /// Keeps the chosen execution target's private workspace and tunnels alive.
    pub guard: Option<Box<dyn Send>>,
}

#[derive(Clone)]
pub struct AntigravityConfig {
    pub launch: Arc<dyn Launch>,
    /// Caller namespace plus session ID. Anonymous callers cannot resolve tools.
    pub session: Option<String>,
    /// Account identity plus auth epoch. Never contains authentication material.
    pub binding: String,
}

pub struct Antigravity {
    pool: Arc<AntigravityPool>,
    config: AntigravityConfig,
}

impl Antigravity {
    pub fn new(
        pool: Arc<AntigravityPool>,
        config: AntigravityConfig,
    ) -> Result<Self, InvalidContract> {
        if config.binding.is_empty() || config.session.as_ref().is_some_and(String::is_empty) {
            return Err(InvalidContract(
                "Antigravity binding and explicit session must be nonempty",
            ));
        }
        Ok(Self { pool, config })
    }
}

impl NativeProvider for Antigravity {
    fn send(&self, request: NativeRequest, sink: NativeSink) -> OperationFuture<NativeCompletion> {
        let config = self.config.clone();
        let pool = self.pool.clone();
        let handle = pool.executor.handle.clone();
        Box::pin(async move {
            let mut task = AbortOnDrop(handle.spawn(execute(pool, config, request, sink)));
            (&mut task.0)
                .await
                .unwrap_or_else(|_| Err(NativeFailure::sent(Error::ProviderFailure, None)))
        })
    }
}

async fn execute(
    pool: Arc<AntigravityPool>,
    config: AntigravityConfig,
    request: NativeRequest,
    mut sink: NativeSink,
) -> NativeCompletion {
    let input = messages::Request::parse(request.body.into_inner(), request.model.as_str())
        .map_err(NativeFailure::unsent)?;
    if config.session.is_none() && !input.tools.is_empty() {
        return Err(NativeFailure::unsent(Error::Unsupported));
    }

    let mut lease = pool.claim(&config, &input)?;
    let mut stop = lease.stop();
    let (mut run, previous, prompt) = if let Some((pending, result)) = lease.previous.take() {
        (
            pending.run,
            Some((pending.prompt_id, pending.call.reply, result)),
            Vec::new(),
        )
    } else {
        let prompt = input.prompt().map_err(NativeFailure::unsent)?;
        let start = acp::Run::start(
            config.launch.as_ref(),
            input.tools.clone(),
            &input.system,
            &input.model,
            prompt.iter().any(|block| block["type"] == "image"),
        );
        let run = tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => return Err(lease.failure(Error::Cancelled)),
            run = tokio::time::timeout(STARTUP_TIMEOUT, start) => {
                run.map_err(|_| NativeFailure::unsent(Error::Timeout))?.map_err(NativeFailure::unsent)?
            }
        };
        (run, None, prompt)
    };

    let mut reply = messages::Reply::new(request.stream, input.model.clone());
    let advance = async {
        let prompt_id = if let Some((id, reply, result)) = previous {
            reply.send(result).map_err(|_| Error::Transport)?;
            id
        } else {
            // A failed write may have handed a partial or complete prompt to the CLI.
            lease.submitted = true;
            run.prompt(prompt).await?
        };
        loop {
            match run.event(prompt_id).await? {
                acp::Event::Content(update) => reply.chunk(&update, &mut sink).await?,
                acp::Event::Tool(call) => {
                    if call.reply.is_closed() {
                        continue;
                    }
                    reply.tool(&call, &mut sink).await?;
                    return Ok(Terminal::Tool(prompt_id, call));
                }
                acp::Event::Stop(reason) => {
                    let reason = match reason.as_str() {
                        "end_turn" => "end_turn",
                        "max_tokens" | "max_turn_requests" => "max_tokens",
                        "refusal" => "refusal",
                        "cancelled" => return Err(Error::Cancelled),
                        _ => return Err(Error::ProviderFailure),
                    };
                    return Ok(Terminal::Stop(reason));
                }
            }
        }
    };
    let terminal = tokio::select! {
        biased;
        _ = stop.wait_for(|stopped| *stopped) => Err(Error::Cancelled),
        result = tokio::time::timeout(SEGMENT_TIMEOUT, advance) => result.unwrap_or(Err(Error::Timeout)),
    };
    match terminal {
        Err(kind) => {
            if lease.submitted {
                run.cancel().await;
            }
            Err(lease.failure(kind))
        }
        Ok(Terminal::Tool(prompt_id, call)) => {
            lease
                .park(pool::Parked {
                    run,
                    prompt_id,
                    call,
                    model: input.model,
                    tools: input.tools,
                    system: input.system,
                })
                .map_err(|kind| lease.failure(kind))?;
            let result = tokio::select! {
                biased;
                _ = stop.wait_for(|stopped| *stopped) => Err(lease.failure(Error::Cancelled)),
                reply = reply.finish("tool_use", &mut sink) => reply,
            }?;
            lease.keep_parked();
            Ok(result)
        }
        Ok(Terminal::Stop(reason)) => {
            drop(run);
            let result = tokio::select! {
                biased;
                _ = stop.wait_for(|stopped| *stopped) => Err(lease.failure(Error::Cancelled)),
                reply = reply.finish(reason, &mut sink) => reply,
            }?;
            lease.finish().map_err(|kind| lease.failure(kind))?;
            Ok(result)
        }
    }
}

enum Terminal {
    Tool(u64, mcp::Call),
    Stop(&'static str),
}

fn nonce() -> Result<String, Error> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| Error::Transport)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
