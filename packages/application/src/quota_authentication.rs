//! Query credentials are provider-private and bound to a confirmed model revision.
use crate::{Error, QuotaAuthenticationDescription, QuotaAuthenticationEdit};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QuotaAuthentication {
    pub access_key: Option<AccessKey>,
    pub generation: u64,
    pub confirmed_model_generation: Option<u64>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccessKey {
    pub id: String,
    pub secret: String,
}
impl std::fmt::Debug for QuotaAuthentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaAuthentication")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl QuotaAuthentication {
    pub(crate) fn valid(&self, model_generation: u64) -> bool {
        self.confirmed_model_generation
            .is_none_or(|g| g <= model_generation)
            && self
                .access_key
                .as_ref()
                .map_or(self.confirmed_model_generation.is_none(), |key| {
                    valid_id(&key.id) && valid_secret(&key.secret)
                })
    }
    pub(crate) fn description(&self, model_generation: u64) -> QuotaAuthenticationDescription {
        QuotaAuthenticationDescription {
            configured: self.access_key.is_some(),
            binding_confirmed: self.access_key.is_some()
                && self.confirmed_model_generation == Some(model_generation),
        }
    }
    pub(crate) fn apply(
        &mut self,
        edit: Option<QuotaAuthenticationEdit>,
        model_generation: u64,
        model_configured: bool,
        target_changed: bool,
    ) -> Result<(), Error> {
        let before = self.clone();
        if target_changed
            || self
                .confirmed_model_generation
                .is_some_and(|g| g != model_generation)
        {
            self.confirmed_model_generation = None;
        }
        match edit {
            None => {}
            Some(QuotaAuthenticationEdit::Clear) => {
                self.access_key = None;
                self.confirmed_model_generation = None;
            }
            Some(QuotaAuthenticationEdit::ConfirmBinding) => {
                if self.access_key.is_none() || !model_configured {
                    return Err(Error::invalid(
                        "先配置模型 API key 与额度查询凭据，再确认关联",
                    ));
                }
                self.confirmed_model_generation = Some(model_generation);
            }
            Some(QuotaAuthenticationEdit::SetAccessKey {
                access_key_id,
                secret_access_key,
                confirmed,
            }) => {
                if !valid_id(&access_key_id)
                    || !valid_secret(&secret_access_key)
                    || (confirmed && !model_configured)
                {
                    return Err(Error::invalid("额度查询凭据或关联确认无效"));
                }
                self.access_key = Some(AccessKey {
                    id: access_key_id,
                    secret: secret_access_key,
                });
                self.confirmed_model_generation = confirmed.then_some(model_generation);
            }
        }
        if *self != before {
            self.generation = before
                .generation
                .checked_add(1)
                .ok_or_else(|| Error::invalid("quota authentication revision overflow"))?;
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
}
