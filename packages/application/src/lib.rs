//! Shared application use cases, ordinary configuration and lifecycle.
mod api;
pub mod config;
#[cfg(feature = "local-runtime")]
mod local;
#[cfg(not(feature = "local-runtime"))]
mod portable;
#[cfg(feature = "local-runtime")]
mod provider_import;
mod repository;
pub use api::*;
#[cfg(feature = "local-runtime")]
use local as implementation;
#[cfg(not(feature = "local-runtime"))]
use portable as implementation;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
pub use velune_conversation as conversation;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub home_directory: PathBuf,
    pub resources_directory: PathBuf,
    #[serde(default)]
    pub credential_resolver: Option<PathBuf>,
}
impl Options {
    fn validate(&self) -> Result<(), Error> {
        for path in [
            Some(&self.home_directory),
            Some(&self.resources_directory),
            self.credential_resolver.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !path.is_absolute() {
                return Err(Error::invalid("application paths must be absolute"));
            }
        }
        fs::create_dir_all(&self.home_directory)
            .map_err(|_| Error::invalid("application home directory"))?;
        if !self.resources_directory.is_dir() {
            return Err(Error::invalid("application resources directory"));
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "local-runtime")]
    #[error("{0}")]
    ProviderImport(#[from] velune_agent_runtime::provider_source::SourceReadError),
    #[error("invalid application operation: {0}")]
    Invalid(String),
    #[error("unsupported application operation: {0}")]
    Unsupported(String),
    #[error("application I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}
impl Error {
    pub(crate) fn invalid(message: &'static str) -> Self {
        Self::Invalid(message.into())
    }
}
pub struct Application {
    runtime: implementation::CoreRuntime,
}
impl Application {
    pub fn open(options: Options) -> Result<Self, Error> {
        Ok(Self {
            runtime: implementation::CoreRuntime::open(options)?,
        })
    }
    pub fn close(&mut self) -> Result<(), Error> {
        self.runtime.close_if_idle()
    }
}
