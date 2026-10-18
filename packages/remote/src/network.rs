use crate::{RemoteError, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkChangeReason {
    Available,
    Lost,
    LinkProperties,
    Capabilities,
    Foreground,
}

/// Platform facts may contain local addresses. They are not ordinary diagnostics or routing policy.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformNetworkSnapshot {
    pub sequence: u64,
    pub elapsed_realtime_ms: u64,
    pub reason: NetworkChangeReason,
    pub default_network_id: Option<String>,
    pub networks: Vec<PlatformNetwork>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformNetwork {
    pub id: String,
    pub wifi: bool,
    pub cellular: bool,
    pub internet: bool,
    pub validated: bool,
    pub interface_name: Option<String>,
    /// IP address and prefix length, without SSID or subscription metadata.
    pub addresses: Vec<String>,
    pub dns_servers: Vec<String>,
}

impl PlatformNetworkSnapshot {
    pub fn validate(&self) -> Result<()> {
        if self.networks.len() > 32
            || self
                .default_network_id
                .as_ref()
                .is_some_and(|id| !valid_label(id))
        {
            return Err(RemoteError::Configuration);
        }
        let mut ids = std::collections::HashSet::new();
        for network in &self.networks {
            if !valid_label(&network.id)
                || !ids.insert(&network.id)
                || network
                    .interface_name
                    .as_ref()
                    .is_some_and(|name| !valid_label(name))
                || network.addresses.len() > 64
                || network.dns_servers.len() > 16
            {
                return Err(RemoteError::Configuration);
            }
            for address in &network.addresses {
                let (ip, prefix) = address.split_once('/').ok_or(RemoteError::Configuration)?;
                let ip: IpAddr = ip.parse().map_err(|_| RemoteError::Configuration)?;
                let prefix: u8 = prefix.parse().map_err(|_| RemoteError::Configuration)?;
                if prefix > if ip.is_ipv4() { 32 } else { 128 } {
                    return Err(RemoteError::Configuration);
                }
            }
            if network
                .dns_servers
                .iter()
                .any(|ip| ip.parse::<IpAddr>().is_err())
            {
                return Err(RemoteError::Configuration);
            }
        }
        // A default callback can precede its capabilities/link-properties callback.
        // Missing detailed facts are retained as an observation, not fabricated.
        Ok(())
    }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}
