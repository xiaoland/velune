//! Provider-side configuration boundary. No concrete network adapter or SDK is selected yet.
//! Future adapters implement velune_ai::provider::SamplingProvider and own protocol conversion.
#![forbid(unsafe_code)]
pub mod config;
