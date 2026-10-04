//! Native adapters are deliberately unavailable. This boundary carries no credentials.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Harness {
    Codex,
    ClaudeCode,
    Pi,
}
impl Harness {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "codex" => Ok(Self::Codex),
            "claude-code" => Ok(Self::ClaudeCode),
            "pi" => Ok(Self::Pi),
            _ => Err(Error::Invalid("unknown harness")),
        }
    }
    pub fn boundary(self) -> Boundary {
        match self {
            Self::Codex => Boundary {
                control: "app-server thread/turn",
                model: "Responses gateway; turn/start model",
                completion: "turn/completed",
                native_verified: false,
            },
            Self::ClaudeCode => Boundary {
                control: "official Agent SDK bridge (not private CLI envelope)",
                model: "Messages gateway; SDK setModel",
                completion: "result plus background-work reconciliation",
                native_verified: false,
            },
            Self::Pi => Boundary {
                control: "Pi v1.0.2 line-delimited RPC (not JSON-RPC 2.0)",
                model: "Pi-native provider; startup flags or set_model",
                completion: "agent_settled (not agent_end)",
                native_verified: false,
            },
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Boundary {
    pub control: &'static str,
    pub model: &'static str,
    pub completion: &'static str,
    pub native_verified: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Fault {
    None,
    Busy,
    BeforeSend,
    AfterAccept,
}
#[derive(Debug)]
pub enum Submission {
    Busy,
    NotSent,
    Unknown,
    Consumed(String),
}

/// Called only at an idle/safe message boundary. Consumed requires execution evidence,
/// not merely transport acceptance. No production implementation ships in this crate.
pub trait HarnessAdapter {
    fn harness(&self) -> Harness;
    fn submit(&mut self, delivery_id: i64, input: &str, is_result: bool) -> Submission;
}

pub struct SimHarness {
    pub kind: Harness,
    pub fault: Fault,
    pub consumed: Vec<i64>,
}
impl SimHarness {
    pub fn new(kind: Harness) -> Self {
        Self {
            kind,
            fault: Fault::None,
            consumed: vec![],
        }
    }
}
impl HarnessAdapter for SimHarness {
    fn harness(&self) -> Harness {
        self.kind
    }
    fn submit(&mut self, id: i64, input: &str, is_result: bool) -> Submission {
        match self.fault {
            Fault::Busy => Submission::Busy,
            Fault::BeforeSend => Submission::NotSent,
            Fault::AfterAccept => {
                self.consumed.push(id);
                Submission::Unknown
            }
            Fault::None => {
                self.consumed.push(id);
                if is_result {
                    Submission::Consumed(format!("SIMULATED: received {input}"))
                } else if input == "2 + 3 = 5" {
                    Submission::Consumed("SIMULATED: arithmetic verified".into())
                } else {
                    Submission::Consumed("SIMULATED: fixture rejected".into())
                }
            }
        }
    }
}

pub struct NativeUnavailable(pub Harness);
impl NativeUnavailable {
    pub fn start(&self) -> Result<()> {
        Err(Error::Unsupported(
            "native transport/login not implemented or authorized",
        ))
    }
}
