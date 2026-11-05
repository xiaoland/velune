//! Transport-neutral, typed Velune application contracts.
pub mod acp;
mod api_account;
mod billing;
pub use billing::*;
pub mod authentication;
mod catalog;
mod command;
mod composer;
pub use composer::*;
mod scheduled_tasks;
pub use scheduled_tasks::*;
pub mod config;
mod failure;
mod next_turn;
mod provider_template;
pub mod records;
pub mod resource_library;
pub mod runtime_execution;
mod runtime_start;
pub mod transcript_delivery;
pub use acp::*;
pub use api_account::{ApiAccountService, api_account_service};
pub use authentication::*;
pub use catalog::*;
pub use command::*;
pub use config::*;
pub use failure::*;
pub use next_turn::*;
pub use provider_template::{
    ProviderTemplate, ProviderTemplateKind, provider_subscription_template, provider_templates,
};
pub use records::*;
pub use resource_library::*;
pub use runtime_execution::*;
pub use runtime_management::*;
pub use runtime_start::*;
pub use transcript_delivery::*;
pub use velune_conversation as conversation;
pub use velune_conversation::bytes;
pub use velune_conversation::resources as runtime_management;

uniffi::setup_scaffolding!();

/// Ordered maps keep persisted configuration stable; foreign languages see dictionaries.
pub type StringMap = std::collections::BTreeMap<String, String>;
uniffi::custom_type!(StringMap, std::collections::HashMap<String, String>, {
    remote,
    lower: |map| map.into_iter().collect(),
    try_lift: |map| Ok(map.into_iter().collect()),
});
pub type WeightMap = std::collections::BTreeMap<String, u32>;
uniffi::custom_type!(WeightMap, std::collections::HashMap<String, u32>, {
    remote,
    lower: |map| map.into_iter().collect(),
    try_lift: |map| Ok(map.into_iter().collect()),
});
