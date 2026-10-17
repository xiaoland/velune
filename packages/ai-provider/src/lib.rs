//! Provider-owned configuration and MiniMax adapter. Protocol/transport stay behind the
//! service-owned SamplingProvider contract. No environment/file/configuration reads here.
#![deny(unsafe_code)]
pub mod claude_code;
pub mod config;
pub mod messages;
pub mod metrics;
pub mod minimax;
pub mod openai;
pub mod responses;
pub mod translation;
