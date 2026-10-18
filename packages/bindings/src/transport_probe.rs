//! Explicit diagnostic-only marker transport; never opens an application or identity file.
use crate::{remote::parse_addresses, *};
use std::{sync::Arc, time::Duration};
use velune_remote::{EndpointConfig, ProbeConnection, ProbeTarget, TransportProbe};

#[derive(Clone, uniffi::Record)]
pub struct BindingTransportProbeOptions {
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingTransportProbeTarget {
    pub endpoint_id: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
}
impl From<ProbeTarget> for BindingTransportProbeTarget {
    fn from(target: ProbeTarget) -> Self {
        Self {
            endpoint_id: target.endpoint_id,
            relay_urls: target.relay_urls,
            direct_addresses: target
                .direct_addresses
                .into_iter()
                .map(|address| address.to_string())
                .collect(),
        }
    }
}
impl TryFrom<BindingTransportProbeTarget> for ProbeTarget {
    type Error = BindingError;
    fn try_from(target: BindingTransportProbeTarget) -> Result<Self, Self::Error> {
        Ok(Self {
            endpoint_id: target.endpoint_id,
            relay_urls: target.relay_urls,
            direct_addresses: parse_addresses(target.direct_addresses)?,
        })
    }
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingProbeReply {
    pub sequence: u64,
    pub payload_matches: bool,
    pub elapsed_microseconds: u64,
    pub received_bytes: u64,
}
/// Explicit private evidence. Raw addresses must not enter ordinary logs or product UI.
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingPathDiagnostic {
    pub path_id: u64,
    pub local_address: Option<String>,
    pub remote_address: String,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingConnectionDiagnostics {
    pub status: BindingConnectionStatus,
    pub paths: Vec<BindingPathDiagnostic>,
}

/// Private mDNS evidence. Dropped counts ring loss since the previous drain, not upstream loss.
#[derive(Clone, uniffi::Enum)]
pub enum BindingProbeMdnsEvent {
    Discovered {
        sampled_at_micros: u64,
        endpoint_id: String,
        direct_addresses: Vec<String>,
    },
    Expired {
        sampled_at_micros: u64,
        endpoint_id: String,
        direct_addresses: Vec<String>,
    },
    Dropped {
        count: u64,
    },
}
impl From<velune_remote::ProbeMdnsEvent> for BindingProbeMdnsEvent {
    fn from(event: velune_remote::ProbeMdnsEvent) -> Self {
        match event {
            velune_remote::ProbeMdnsEvent::Discovered {
                sampled_at_micros,
                endpoint_id,
                direct_addresses,
            } => Self::Discovered {
                sampled_at_micros,
                endpoint_id,
                direct_addresses: direct_addresses
                    .into_iter()
                    .map(|address| address.to_string())
                    .collect(),
            },
            velune_remote::ProbeMdnsEvent::Expired {
                sampled_at_micros,
                endpoint_id,
                direct_addresses,
            } => Self::Expired {
                sampled_at_micros,
                endpoint_id,
                direct_addresses: direct_addresses
                    .into_iter()
                    .map(|address| address.to_string())
                    .collect(),
            },
            velune_remote::ProbeMdnsEvent::Dropped { count } => Self::Dropped { count },
        }
    }
}

#[derive(uniffi::Object)]
pub struct VeluneTransportProbe {
    probe: TransportProbe,
    runtime: Arc<tokio::runtime::Runtime>,
}
#[uniffi::export]
impl VeluneTransportProbe {
    #[uniffi::constructor]
    pub fn open(options: BindingTransportProbeOptions) -> Result<Arc<Self>, BindingError> {
        crate::transport_trace::install()?;
        let config = EndpointConfig {
            relay: match options.relay {
                BindingRelayConfig::Disabled => velune_remote::RelayConfig::Disabled,
                BindingRelayConfig::DevelopmentPublic => {
                    velune_remote::RelayConfig::DevelopmentPublic
                }
                BindingRelayConfig::Custom { urls } => velune_remote::RelayConfig::Custom { urls },
            },
            discovery: match options.discovery {
                BindingDiscoveryConfig::None => velune_remote::DiscoveryConfig::None,
                BindingDiscoveryConfig::DevelopmentPublic => {
                    velune_remote::DiscoveryConfig::DevelopmentPublic
                }
                BindingDiscoveryConfig::CustomPkarr { url } => {
                    velune_remote::DiscoveryConfig::CustomPkarr { url }
                }
            },
            path_policy: match options.path_policy {
                BindingRemotePathPolicy::Automatic => velune_remote::PathPolicy::Automatic,
                BindingRemotePathPolicy::RelayOnly => velune_remote::PathPolicy::RelayOnly,
            },
            bind_addresses: parse_addresses(options.bind_addresses)?,
            request_timeout: Duration::from_secs(30),
        };
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| BindingError::Unavailable)?,
        );
        let probe = runtime.block_on(TransportProbe::bind(config))?;
        Ok(Arc::new(Self { runtime, probe }))
    }
    pub fn target(&self) -> Result<BindingTransportProbeTarget, BindingError> {
        Ok(self.probe.descriptor().into())
    }
    pub fn connect(
        &self,
        target: BindingTransportProbeTarget,
    ) -> Result<Arc<VeluneTransportProbeConnection>, BindingError> {
        let connection = self
            .runtime
            .block_on(self.probe.connect(&target.try_into()?))?;
        Ok(Arc::new(VeluneTransportProbeConnection {
            runtime: self.runtime.clone(),
            connection,
        }))
    }
    pub fn report_network_change(
        &self,
        snapshot: BindingPlatformNetworkSnapshot,
    ) -> Result<u64, BindingError> {
        self.runtime
            .block_on(self.probe.report_network_change(snapshot.try_into()?))
            .map_err(Into::into)
    }
    pub fn connections(&self) -> Result<Vec<BindingConnectionDiagnostics>, BindingError> {
        convert(self.probe.connections())
    }
    pub fn enable_mdns(&self) -> Result<(), BindingError> {
        self.runtime
            .block_on(self.probe.enable_mdns())
            .map_err(Into::into)
    }
    /// Drain this probe's shared event queue, including loss since the previous drain.
    pub fn mdns_events(&self) -> Result<Vec<BindingProbeMdnsEvent>, BindingError> {
        Ok(self
            .probe
            .mdns_events()
            .into_iter()
            .map(Into::into)
            .collect())
    }
    pub fn mdns_resolve(
        &self,
        endpoint_id: String,
        timeout_ms: u64,
    ) -> Result<BindingTransportProbeTarget, BindingError> {
        let target = self.runtime.block_on(
            self.probe
                .mdns_resolve(endpoint_id, Duration::from_millis(timeout_ms)),
        )?;
        Ok(target.into())
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.runtime.block_on(self.probe.close());
        Ok(())
    }
}

#[derive(uniffi::Object)]
pub struct VeluneTransportProbeConnection {
    connection: ProbeConnection,
    runtime: Arc<tokio::runtime::Runtime>,
}
#[uniffi::export]
impl VeluneTransportProbeConnection {
    pub fn connection_status(&self) -> Result<BindingConnectionStatus, BindingError> {
        convert(self.connection.connection_status())
    }
    pub fn diagnostics(&self) -> Result<BindingConnectionDiagnostics, BindingError> {
        convert(self.connection.diagnostics())
    }
    pub fn observe_paths(&self) -> Result<Arc<VelunePathObserver>, BindingError> {
        Ok(VelunePathObserver::new(
            self.runtime.clone(),
            self.connection.observe_paths(),
        ))
    }
    pub fn round_trip(
        &self,
        sequence: u64,
        payload: Vec<u8>,
        timeout_ms: u64,
    ) -> Result<BindingProbeReply, BindingError> {
        let reply = self.runtime.block_on(self.connection.round_trip(
            sequence,
            payload,
            Duration::from_millis(timeout_ms),
        ))?;
        convert(reply)
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.connection.close();
        Ok(())
    }
}
