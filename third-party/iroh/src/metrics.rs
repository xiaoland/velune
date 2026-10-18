//! Co-locating all of the iroh metrics structs
use std::sync::Arc;

use iroh_metrics::MetricsGroupSet;

use serde::{Deserialize, Serialize};

pub use crate::{
    address_lookup::Metrics as AddressLookupMetrics, net_report::Metrics as NetReportMetrics,
    socket::Metrics as SocketMetrics,
};

/// Metrics collected by an [`crate::endpoint::Endpoint`].
///
/// See [`crate::endpoint::Endpoint::metrics`] for details.
#[derive(Default, Debug, Clone, Serialize, Deserialize, MetricsGroupSet)]
#[metrics(name = "endpoint")]
#[non_exhaustive]
pub struct EndpointMetrics {
    /// Metrics collected by the endpoint's socket.
    pub socket: Arc<SocketMetrics>,
    /// Metrics collected by net reports.
    pub net_report: Arc<NetReportMetrics>,
    /// Metrics collected by address lookup.
    pub address_lookup: Arc<AddressLookupMetrics>,
}
