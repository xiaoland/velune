//! Resource choice is independent of delegation/harness selection. All dispatch is mock-only.
use crate::adapter::Harness;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Responses,
    Messages,
    PiProvider,
}
impl Protocol {
    pub fn for_harness(h: Harness) -> Self {
        match h {
            Harness::Codex => Self::Responses,
            Harness::ClaudeCode => Self::Messages,
            Harness::Pi => Self::PiProvider,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Resource {
    pub slot: String,
    pub account: String,
    pub model: String,
    pub provider: String,
    pub protocol: Protocol,
    pub authorized: bool,
    pub simulated: bool,
    pub available: bool,
}
impl Resource {
    pub fn fixture(h: Harness) -> Self {
        Self {
            slot: format!("mock-{}", h.name()),
            account: format!("synthetic-{}", h.name()),
            model: "deterministic-v1".into(),
            provider: "local-fixture-provider".into(),
            protocol: Protocol::for_harness(h),
            authorized: true,
            simulated: true,
            available: true,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Dispatch(usize),
    Wait,
    HandoffRequired,
    NeedsDecision,
}
pub struct Intent<'a> {
    pub harness: Harness,
    pub allowed_slots: &'a [String],
    pub bound_account: Option<&'a str>,
}
pub fn choose(intent: &Intent<'_>, resources: &[Resource]) -> Decision {
    let eligible: Vec<_> = resources
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.authorized
                && r.simulated
                && r.protocol == Protocol::for_harness(intent.harness)
                && intent.allowed_slots.contains(&r.slot)
        })
        .collect();
    if eligible.is_empty() {
        return Decision::NeedsDecision;
    }
    let compatible: Vec<_> = eligible
        .iter()
        .filter(|(_, r)| intent.bound_account.is_none_or(|a| a == r.account))
        .collect();
    if compatible.is_empty() {
        return Decision::HandoffRequired;
    }
    compatible
        .into_iter()
        .find(|(_, r)| r.available)
        .map_or(Decision::Wait, |(i, _)| Decision::Dispatch(*i))
}
