//! Typed foreign-language entry point for the shared application unit.
//!
//! Each object owns one application session. Explicit close refuses a busy
//! runtime and leaves the object usable. Dropping a language wrapper is resource
//! cleanup, not a user-facing request to cancel an Agent turn.
mod types;
pub use types::*;

use serde::{Serialize, de::DeserializeOwned};
use std::sync::{Arc, Mutex, MutexGuard};
use velune_application::{Application, Error, Options};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BindingError {
    #[error("{detail}")]
    Invalid { detail: String },
    #[error("{detail}")]
    Unsupported { detail: String },
    #[error("{detail}")]
    Io { detail: String },
    #[error("{detail}")]
    Contract { detail: String },
    #[error("application has been closed")]
    Closed,
    #[error("application state is unavailable")]
    Unavailable,
}

impl From<Error> for BindingError {
    fn from(error: Error) -> Self {
        // The application error's typed variants are preserved at this boundary.
        match error {
            Error::Invalid(message) => Self::Invalid { detail: message },
            Error::Unsupported(message) => Self::Unsupported { detail: message },
            Error::Io(error) => Self::Io {
                detail: error.to_string(),
            },
            Error::Json(error) => Self::Contract {
                detail: error.to_string(),
            },
        }
    }
}

fn convert<I: Serialize, O: DeserializeOwned>(input: I) -> Result<O, BindingError> {
    let value = serde_json::to_value(input).map_err(|error| BindingError::Contract {
        detail: error.to_string(),
    })?;
    serde_json::from_value(value).map_err(|error| BindingError::Contract {
        detail: error.to_string(),
    })
}

#[derive(Clone, uniffi::Record)]
pub struct BindingOptions {
    pub home_directory: String,
    pub resources_directory: String,
    pub credential_resolver: Option<String>,
}

#[derive(uniffi::Object)]
pub struct VeluneApplication {
    application: Mutex<Option<Application>>,
}

impl VeluneApplication {
    fn lock(&self) -> Result<MutexGuard<'_, Option<Application>>, BindingError> {
        self.application
            .lock()
            .map_err(|_| BindingError::Unavailable)
    }
    fn with<T>(
        &self,
        operation: impl FnOnce(&mut Application) -> Result<T, Error>,
    ) -> Result<T, BindingError> {
        let mut state = self.lock()?;
        operation(state.as_mut().ok_or(BindingError::Closed)?).map_err(Into::into)
    }
}

#[uniffi::export]
impl VeluneApplication {
    #[uniffi::constructor]
    pub fn open(options: BindingOptions) -> Result<Arc<Self>, BindingError> {
        let application = Application::open(Options {
            home_directory: options.home_directory.into(),
            resources_directory: options.resources_directory.into(),
            credential_resolver: options.credential_resolver.map(Into::into),
        })?;
        Ok(Arc::new(Self {
            application: Mutex::new(Some(application)),
        }))
    }

    /// Close only when the application allows it. A failed close retains all
    /// resources and remains available for polling or an explicit Agent cancel.
    pub fn shutdown(&self) -> Result<(), BindingError> {
        let mut state = self.lock()?;
        if let Some(application) = state.as_mut() {
            application.close()?;
            *state = None;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with(|application| application.list())?)
    }

    pub fn upsert_gateway(
        &self,
        gateway: BindingGatewayConfig,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        let gateway = convert(gateway)?;
        convert(self.with(|application| application.upsert_gateway(gateway))?)
    }

    pub fn delete_gateway(&self, id: String) -> Result<BindingGatewayUpdate, BindingError> {
        convert(self.with(|application| application.delete_gateway(id))?)
    }

    pub fn upsert_runtime(
        &self,
        runtime: BindingRuntimeInstance,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        convert(self.with(|application| application.upsert_runtime(runtime))?)
    }

    pub fn delete_runtime(&self, id: String) -> Result<BindingRuntimeUpdate, BindingError> {
        convert(self.with(|application| application.delete_runtime(id))?)
    }

    pub fn connect_runtime(&self, id: String) -> Result<BindingConnectionResult, BindingError> {
        convert(self.with(|application| application.connect_runtime(id))?)
    }

    pub fn create_conversation(
        &self,
        runtime_id: String,
        cwd: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with(|application| application.create_conversation(runtime_id, cwd))?)
    }

    pub fn open_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(
            self.with(|application| application.open_conversation(runtime_id, conversation_id))?,
        )
    }

    pub fn snapshot(&self, runtime_id: String) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with(|application| application.snapshot(runtime_id))?)
    }

    pub fn send(
        &self,
        runtime_id: String,
        text: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with(|application| application.send(runtime_id, text))?)
    }

    pub fn cancel(&self, runtime_id: String) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with(|application| application.cancel(runtime_id))?)
    }

    pub fn select_model(
        &self,
        runtime_id: String,
        model_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with(|application| application.select_model(runtime_id, model_id))?)
    }

    pub fn preview_provider_import(
        &self,
        gateway_id: String,
        source: BindingProviderImportSource,
    ) -> Result<BindingProviderImportPreview, BindingError> {
        let source = convert(source)?;
        convert(self.with(|application| application.preview_provider_import(gateway_id, source))?)
    }

    pub fn apply_provider_import(
        &self,
        gateway_id: String,
        source: BindingProviderImportSource,
        preview_token: String,
        selections: Vec<BindingImportSelection>,
        replace_existing: bool,
    ) -> Result<BindingImportResult, BindingError> {
        let source = convert(source)?;
        let selections = convert(selections)?;
        convert(self.with(|application| {
            application.apply_provider_import(
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing,
            )
        })?)
    }

    pub fn authentication_inspect(
        &self,
        source: BindingCredentialSource,
    ) -> Result<BindingAuthenticationMetadata, BindingError> {
        let source = convert(source)?;
        convert(self.with(|application| application.authentication_inspect(source))?)
    }

    pub fn authentication_start(
        &self,
        source: BindingCredentialSource,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        let source = convert(source)?;
        convert(self.with(|application| application.authentication_start(source))?)
    }

    pub fn authentication_poll(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with(|application| application.authentication_poll())?)
    }

    pub fn authentication_reply(
        &self,
        prompt_id: String,
        value: String,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with(|application| application.authentication_reply(prompt_id, value))?)
    }

    pub fn authentication_cancel(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with(|application| application.authentication_cancel())?)
    }
}
