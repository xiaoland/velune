//! Explicit diagnostic-only marker transport; never opens an application or identity file.
use crate::{
    remote::{RemoteRuntime, parse_addresses},
    *,
};
use std::{sync::Arc, time::Duration};
use velune_remote::{
    ConnectionDiagnostics, ConnectionStatus, DiscoveryConfig, PathPolicy, PlatformNetworkSnapshot,
    ProbeMdnsEvent, ProbeReply, RelayConfig,
};
use velune_remote::{EndpointConfig, ProbeConnection, ProbeTarget, TransportProbe};

#[derive(Clone, uniffi::Record)]
pub struct BindingTransportProbeOptions {
    pub relay: RelayConfig,
    pub discovery: DiscoveryConfig,
    pub path_policy: PathPolicy,
    pub bind_addresses: Vec<String>,
}

#[derive(uniffi::Object)]
pub struct VeluneTransportProbe {
    probe: TransportProbe,
    runtime: Arc<RemoteRuntime>,
}
#[uniffi::export]
impl VeluneTransportProbe {
    #[uniffi::constructor]
    pub fn open(options: BindingTransportProbeOptions) -> Result<Arc<Self>, BindingError> {
        crate::transport_trace::install()?;
        let config = EndpointConfig {
            local_discovery: false,
            relay_credentials: Vec::new(),
            relay: options.relay,
            discovery: options.discovery,
            path_policy: options.path_policy,
            bind_addresses: parse_addresses(options.bind_addresses)?,
            request_timeout: Duration::from_secs(30),
        };
        let runtime = RemoteRuntime::new()?;
        let probe = runtime.block_on(TransportProbe::bind(config))?;
        Ok(Arc::new(Self { runtime, probe }))
    }
    pub fn target(&self) -> Result<ProbeTarget, BindingError> {
        Ok(self.probe.descriptor())
    }
    pub fn connect(
        &self,
        target: ProbeTarget,
    ) -> Result<Arc<VeluneTransportProbeConnection>, BindingError> {
        let connection = self.runtime.block_on(self.probe.connect(&target))?;
        Ok(Arc::new(VeluneTransportProbeConnection {
            runtime: self.runtime.clone(),
            connection,
        }))
    }
    pub fn report_network_change(
        &self,
        snapshot: PlatformNetworkSnapshot,
    ) -> Result<u64, BindingError> {
        snapshot.validate()?;
        self.runtime
            .block_on(self.probe.report_network_change(snapshot))
            .map_err(Into::into)
    }
    pub fn connections(&self) -> Result<Vec<ConnectionDiagnostics>, BindingError> {
        Ok(self.probe.connections())
    }
    pub fn enable_mdns(&self) -> Result<(), BindingError> {
        self.runtime
            .block_on(self.probe.enable_mdns())
            .map_err(Into::into)
    }
    pub fn refresh_mdns_interfaces(&self) -> Result<u64, BindingError> {
        self.runtime
            .block_on(self.probe.refresh_mdns_interfaces())
            .map_err(Into::into)
    }
    /// Drain this probe's shared event queue, including loss since the previous drain.
    pub fn mdns_events(&self) -> Result<Vec<ProbeMdnsEvent>, BindingError> {
        Ok(self.probe.mdns_events())
    }
    pub fn mdns_resolve(
        &self,
        endpoint_id: String,
        timeout_ms: u64,
    ) -> Result<ProbeTarget, BindingError> {
        let target = self.runtime.block_on(
            self.probe
                .mdns_resolve(endpoint_id, Duration::from_millis(timeout_ms)),
        )?;
        Ok(target)
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.runtime.block_on(self.probe.close())?;
        Ok(())
    }
}

#[derive(uniffi::Object)]
pub struct VeluneTransportProbeConnection {
    connection: ProbeConnection,
    runtime: Arc<RemoteRuntime>,
}
#[uniffi::export]
impl VeluneTransportProbeConnection {
    pub fn connection_status(&self) -> Result<ConnectionStatus, BindingError> {
        Ok(self.connection.connection_status())
    }
    pub fn diagnostics(&self) -> Result<ConnectionDiagnostics, BindingError> {
        Ok(self.connection.diagnostics())
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
    ) -> Result<ProbeReply, BindingError> {
        let reply = self.runtime.block_on(self.connection.round_trip(
            sequence,
            payload,
            Duration::from_millis(timeout_ms),
        ))?;
        Ok(reply)
    }
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.connection.close();
        Ok(())
    }
}
