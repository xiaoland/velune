//! Provider-owned configuration and MiniMax adapter. Protocol/transport stay behind the
//! service-owned SamplingProvider contract. No environment/file/configuration reads here.
#![forbid(unsafe_code)]
pub mod config;
pub mod messages;
pub mod minimax;
pub mod openai;
pub mod responses;
pub mod translation;
