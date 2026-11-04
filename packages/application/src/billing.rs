//! Optional account reporting, independent of model execution and quota admission.
use crate::{
    BillingAuthenticationEdit, BillingConfiguration, BillingReport, ProviderBillingStatus,
    QuotaAuthenticationDescription,
    config::*,
    query_credential::{QueryCredential, valid_secret},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "billing_http.rs"]
mod http;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Configuration {
    pub settings: Option<BillingConfiguration>,
    pub credential: Option<QueryCredential>,
    pub confirmed_model_generation: Option<u64>,
}
impl Configuration {
    pub(crate) fn validate(&self, provider: &ProviderDefinition) -> Result<(), &'static str> {
        match (&self.settings, &self.credential) {
            (None, None) if self.confirmed_model_generation.is_none() => Ok(()),
            (Some(settings), Some(QueryCredential::ApiKey { value })) => {
                settings.validate()?;
                if supported(provider) != Some(settings.service)
                    || !valid_secret(value)
                    || self
                        .confirmed_model_generation
                        .is_some_and(|g| g > provider.authentication.generation)
                {
                    return Err("账单来源、认证或关联配置无效");
                }
                Ok(())
            }
            _ => Err("账单来源与查询认证必须一起配置"),
        }
    }
    fn description(&self, generation: u64) -> QuotaAuthenticationDescription {
        QuotaAuthenticationDescription {
            configured: self.credential.is_some(),
            binding_confirmed: self.credential.is_some()
                && self.confirmed_model_generation == Some(generation),
        }
    }
    pub(crate) fn apply(
        &mut self,
        provider: &ProviderDefinition,
        settings: Option<BillingConfiguration>,
        edit: BillingAuthenticationEdit,
        confirmed: bool,
    ) -> Result<(), crate::Error> {
        let Some(settings) = settings else {
            *self = Self::default();
            return Ok(());
        };
        settings.validate().map_err(crate::Error::invalid)?;
        if supported(provider) != Some(settings.service) {
            return Err(crate::Error::invalid(
                "账单查询只支持以 API key 直连 OpenAI／Anthropic 官方地址",
            ));
        }
        match edit {
            BillingAuthenticationEdit::Keep => {}
            BillingAuthenticationEdit::SetApiKey { value } => {
                if !valid_secret(&value) {
                    return Err(crate::Error::invalid("查询 API key 无效"));
                }
                self.credential = Some(QueryCredential::ApiKey { value });
            }
            BillingAuthenticationEdit::Clear => {
                *self = Self::default();
                return Ok(());
            }
        }
        if self.credential.is_none() {
            return Err(crate::Error::invalid("请填写组织查询 API key"));
        }
        if !confirmed {
            return Err(crate::Error::invalid(
                "请确认查询组织与此提供商模型认证的关联",
            ));
        }
        if !matches!(&provider.authentication.material, crate::provider_authentication::AuthenticationMaterial::ApiKey { value } if !value.is_empty())
        {
            return Err(crate::Error::invalid(
                "请先保存模型 API key，再配置账单查询",
            ));
        }
        self.settings = Some(settings);
        self.confirmed_model_generation = Some(provider.authentication.generation);
        Ok(())
    }
}
pub(crate) fn supported(provider: &ProviderDefinition) -> Option<crate::BillingService> {
    if provider.execution_backend != ExecutionBackend::Http
        || provider.request_profile != RequestProfile::Native
        || provider.authentication.source() != crate::ProviderAuthenticationSource::ApiKey
    {
        return None;
    }
    velune_application_api::billing_service(&provider.endpoint)
}
#[derive(Clone)]
struct Capture {
    settings: BillingConfiguration,
    key: String,
}
impl Capture {
    fn identity(&self, month: i64) -> String {
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&self.settings).expect("billing scope serialization"));
        digest.update([0]);
        digest.update(self.key.as_bytes());
        digest.update(month.to_le_bytes());
        format!("{:x}", digest.finalize())
    }
}
struct Entry {
    report: Option<BillingReport>,
    problem: Option<String>,
    running: bool,
    fetched: Option<Instant>,
    retry_at: Instant,
    cancel: tokio::sync::watch::Sender<bool>,
}
#[derive(Default)]
struct State {
    bindings: BTreeMap<(String, String), Option<Capture>>,
    entries: BTreeMap<String, Entry>,
    stopped: bool,
    inflight: usize,
}
#[derive(Default)]
pub(crate) struct Billing {
    state: Mutex<State>,
}
const TTL: Duration = Duration::from_secs(15 * 60);
impl Billing {
    pub(crate) fn configure(&self, gateways: &[GatewayConfig]) {
        let mut state = self.state.lock().expect("billing state");
        state.bindings = gateways
            .iter()
            .flat_map(|g| {
                g.providers.iter().map(move |p| {
                    let capture = if p
                        .billing
                        .description(p.authentication.generation)
                        .binding_confirmed
                    {
                        p.billing
                            .settings
                            .clone()
                            .zip(
                                p.billing
                                    .credential
                                    .as_ref()
                                    .and_then(QueryCredential::api_key),
                            )
                            .map(|(settings, key)| Capture {
                                settings,
                                key: key.into(),
                            })
                    } else {
                        None
                    };
                    ((g.id.clone(), p.id.clone()), capture)
                })
            })
            .collect();
        let month = month_start();
        let live: BTreeSet<_> = state
            .bindings
            .values()
            .flatten()
            .map(|c| c.identity(month))
            .collect();
        state.entries.retain(|key, entry| {
            if live.contains(key) {
                true
            } else {
                entry.cancel.send_replace(true);
                false
            }
        });
    }
    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("billing state");
        state.stopped = true;
        for entry in state.entries.values() {
            entry.cancel.send_replace(true);
        }
        state.entries.clear();
        state.bindings.clear();
    }
    pub(crate) fn query(
        self: &Arc<Self>,
        gateway: &str,
        provider: &ProviderDefinition,
        force: bool,
    ) -> ProviderBillingStatus {
        let mut status = ProviderBillingStatus {
            configuration: provider.billing.settings.clone(),
            authentication: provider
                .billing
                .description(provider.authentication.generation),
            report: None,
            refreshing: false,
            stale: false,
            problem: None,
        };
        if status.configuration.is_none() {
            return status;
        }
        if !status.authentication.binding_confirmed {
            status.problem = Some("模型认证已变化，请重新确认查询组织的关联".into());
            return status;
        }
        let month = month_start();
        let mut state = self.state.lock().expect("billing state");
        if state.stopped {
            status.problem = Some("账单查询已停止".into());
            return status;
        }
        let Some(capture) = state
            .bindings
            .get(&(gateway.into(), provider.id.clone()))
            .cloned()
            .flatten()
        else {
            return status;
        };
        let identity = capture.identity(month);
        // Retire old calendar periods without retaining secrets or unbounded report caches.
        let live: BTreeSet<_> = state
            .bindings
            .values()
            .flatten()
            .map(|c| c.identity(month))
            .collect();
        state.entries.retain(|key, entry| {
            if live.contains(key) {
                true
            } else {
                entry.cancel.send_replace(true);
                false
            }
        });
        let capacity = state.inflight < 4;
        let entry = state
            .entries
            .entry(identity.clone())
            .or_insert_with(|| Entry {
                report: None,
                problem: None,
                running: false,
                fetched: None,
                retry_at: Instant::now(),
                cancel: tokio::sync::watch::channel(false).0,
            });
        let stale = entry.fetched.is_none_or(|time| time.elapsed() >= TTL);
        let start =
            !entry.running && capacity && Instant::now() >= entry.retry_at && (force || stale);
        if start {
            entry.running = true;
            entry.problem = None;
            entry.retry_at = Instant::now() + Duration::from_secs(30);
        }
        status.report = entry.report.clone();
        status.stale = stale && status.report.is_some();
        status.refreshing = entry.running;
        status.problem = entry.problem.clone();
        let cancel = entry.cancel.subscribe();
        let job_cancel = entry.cancel.clone();
        if !start {
            return status;
        }
        state.inflight += 1;
        drop(state);
        let service = self.clone();
        std::thread::spawn(move || {
            let result = collect(capture, month, cancel);
            service.finish(&identity, &job_cancel, result);
        });
        status
    }
    /// A canceled entry may have been recreated under the same scope and credential.
    fn finish(
        &self,
        identity: &str,
        job_cancel: &tokio::sync::watch::Sender<bool>,
        result: Result<BillingReport, String>,
    ) {
        let mut state = self.state.lock().expect("billing state");
        state.inflight -= 1;
        if *job_cancel.borrow() {
            return;
        }
        if let Some(entry) = state.entries.get_mut(identity) {
            entry.running = false;
            match result {
                Ok(report) => {
                    entry.fetched = Some(Instant::now());
                    entry.report = Some(report);
                    entry.problem = None;
                }
                Err(problem) => {
                    entry.problem = Some(problem);
                    entry.fetched = None;
                    entry.retry_at = Instant::now() + Duration::from_secs(60);
                }
            }
        }
    }
}
fn month_start() -> i64 {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("calendar month")
        .timestamp_millis()
}
fn collect(
    capture: Capture,
    starting: i64,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<BillingReport, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "账单查询无法启动")?;
    runtime.block_on(async {
        if *cancel.borrow() { return Err("账单查询已取消".into()); }
        tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(45),http::collect(capture.settings,&capture.key,starting)) => result.map_err(|_|"账单查询超时".to_owned())?,
            _=cancel.changed()=>Err("账单查询已取消".into()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_identity_separates_permissions_scope_and_month() {
        let mut capture = Capture {
            settings: BillingConfiguration {
                service: crate::BillingService::OpenAi,
                organization_id: "org_test".into(),
                scope: crate::BillingScope::Organization,
            },
            key: "synthetic-admin".into(),
        };
        let identity = capture.identity(1);
        assert_ne!(identity, capture.identity(2));
        capture.key = "different-permission".into();
        assert_ne!(identity, capture.identity(1));
        capture.key = "synthetic-admin".into();
        capture.settings.scope = crate::BillingScope::Project {
            id: "proj_test".into(),
        };
        assert_ne!(identity, capture.identity(1));
    }
    #[test]
    fn canceled_job_cannot_write_into_recreated_entry() {
        let billing = Billing::default();
        let old = tokio::sync::watch::channel(false).0;
        old.send_replace(true);
        billing.state.lock().unwrap().entries.insert(
            "scope".into(),
            Entry {
                report: None,
                problem: None,
                running: true,
                fetched: None,
                retry_at: Instant::now(),
                cancel: tokio::sync::watch::channel(false).0,
            },
        );
        billing.state.lock().unwrap().inflight = 1;
        billing.finish("scope", &old, Err("late failure".into()));
        let state = billing.state.lock().unwrap();
        assert!(state.entries["scope"].running);
        assert!(state.entries["scope"].problem.is_none());
        assert_eq!(state.inflight, 0);
    }
}
