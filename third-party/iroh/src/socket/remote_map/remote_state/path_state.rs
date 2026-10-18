//! The state kept for each network path to a remote endpoint.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use n0_error::e;
use n0_future::time::Instant;
use rustc_hash::FxHashMap;
use tokio::sync::oneshot;
use tracing::trace;

use super::{Source, TransportAddrInfo, TransportAddrUsage};
use crate::{address_lookup::AddressLookupFailed, metrics::SocketMetrics, socket::transports};

/// Maximum number of non-relay paths we keep around per endpoint.
pub(super) const MAX_NON_RELAY_PATHS: usize = 30;

/// Maximum number of inactive non-relay paths we keep around per endpoint.
///
/// These are paths that at one point been opened and are now closed.
pub(super) const MAX_INACTIVE_NON_RELAY_PATHS: usize = 10;

/// Map of all paths that we are aware of for a remote endpoint.
///
/// Also stores a list of resolve requests which are triggered once at least one path is known,
/// or once this struct is notified of a failed Address Lookup run.
#[derive(Debug)]
pub(super) struct RemotePathState {
    /// All possible paths we are aware of.
    ///
    /// These paths might be entirely impossible to use, since they are added by Address Lookup
    /// mechanisms. The are only potentially usable.
    paths: FxHashMap<transports::Addr, PathState>,
    /// Pending resolve requests from [`Self::resolve_remote`].
    pending_resolve_requests: VecDeque<oneshot::Sender<Result<(), AddressLookupFailed>>>,
    metrics: Arc<SocketMetrics>,
}

/// Describes the usability of this path, i.e. whether it has ever been opened,
/// when it was closed, or if it has never been usable.
#[derive(Debug, Default)]
pub(super) enum PathStatus {
    /// This path is open and active.
    Open,
    /// This path was once opened, but was abandoned at the given [`Instant`].
    Inactive(Instant),
    /// This path was never usable (we attempted holepunching and it didn't work).
    Unusable,
    /// We have not yet attempted holepunching, or holepunching is currently in
    /// progress, so we do not know the usability of this path.
    #[default]
    Unknown,
}

impl RemotePathState {
    pub(super) fn new(metrics: Arc<SocketMetrics>) -> Self {
        Self {
            paths: Default::default(),
            pending_resolve_requests: Default::default(),
            metrics,
        }
    }

    pub(super) fn to_remote_addrs(&self) -> Vec<TransportAddrInfo> {
        self.paths
            .iter()
            .flat_map(|(addr, state)| {
                let usage = match state.status {
                    PathStatus::Open => TransportAddrUsage::Active,
                    PathStatus::Inactive(_) | PathStatus::Unusable | PathStatus::Unknown => {
                        TransportAddrUsage::Inactive
                    }
                };
                Some(TransportAddrInfo {
                    addr: addr.clone().into(),
                    usage,
                })
            })
            .collect()
    }

    /// Insert a new address of an open path into our list of paths.
    ///
    /// This will emit pending resolve requests and trigger pruning paths.
    pub(super) fn insert_open_path(&mut self, addr: transports::Addr, source: Source) {
        match addr {
            transports::Addr::Ip(_) => self.metrics.transport_ip_paths_added.inc(),
            transports::Addr::Relay(_, _) => self.metrics.transport_relay_paths_added.inc(),
            transports::Addr::Custom(_) => self.metrics.transport_custom_paths_added.inc(),
        };
        let state = self.paths.entry(addr).or_default();
        state.status = PathStatus::Open;
        state.sources.insert(source.clone(), Instant::now());
        self.emit_pending_resolve_requests(None);
        self.prune_paths();
    }

    /// Mark a path as abandoned.
    ///
    /// If this path does not exist, it does nothing to the
    /// `RemotePathState`
    pub(super) fn abandoned_path(&mut self, addr: &transports::Addr) {
        if let Some(state) = self.paths.get_mut(addr) {
            if matches!(state.status, PathStatus::Open) {
                match addr {
                    transports::Addr::Ip(_) => self.metrics.transport_ip_paths_removed.inc(),
                    transports::Addr::Relay(_, _) => {
                        self.metrics.transport_relay_paths_removed.inc()
                    }
                    transports::Addr::Custom(_) => {
                        self.metrics.transport_custom_paths_removed.inc()
                    }
                };
            }
            match state.status {
                PathStatus::Open | PathStatus::Inactive(_) => {
                    state.status = PathStatus::Inactive(Instant::now());
                }
                PathStatus::Unusable | PathStatus::Unknown => {
                    state.status = PathStatus::Unusable;
                }
            }
        }
    }

    /// Inserts multiple addresses of unknown status into our list of potential paths.
    ///
    /// If this caused the path set to transition from empty to non-empty, any
    /// pending resolve requests are woken with `Ok(())`. Inserts that add no
    /// new paths (empty iterator, or only duplicates) are a no-op: waking
    /// pending requests while the path set is still empty would send a bogus
    /// `AddressLookupFailed::NoResults` while an address lookup is in flight.
    pub(super) fn insert_multiple(
        &mut self,
        addrs: impl Iterator<Item = transports::Addr>,
        source: Source,
    ) {
        let now = Instant::now();
        let was_empty = self.paths.is_empty();
        for addr in addrs {
            self.paths
                .entry(addr)
                .or_default()
                .sources
                .insert(source.clone(), now);
        }
        trace!("added addressing information");
        if was_empty && !self.paths.is_empty() {
            self.emit_pending_resolve_requests(None);
        }
        self.prune_paths();
    }

    /// Sends back on `tx` once a possible path to the remote is known.
    ///
    /// If there already is a known path, `Ok(())` is returned immediately. Otherwise an
    /// address lookup is performed and the result is sent back once that
    /// completes. [`AddressLookupFailed`] is sent if there are no known paths.
    pub(super) fn resolve_remote(&mut self, tx: oneshot::Sender<Result<(), AddressLookupFailed>>) {
        if !self.paths.is_empty() {
            tx.send(Ok(())).ok();
        } else {
            self.pending_resolve_requests.push_back(tx);
        }
    }

    /// Returns `true` if there are any queued resolve requests from [`Self::resolve_remote`].
    pub(super) fn resolve_requests_is_empty(&self) -> bool {
        self.pending_resolve_requests.is_empty()
    }

    /// Notifies that a Address Lookup run has finished.
    ///
    /// This will emit pending resolve requests.
    pub(super) fn address_lookup_finished(&mut self, result: Result<(), AddressLookupFailed>) {
        self.emit_pending_resolve_requests(result.err());
    }

    /// Returns an iterator over the addresses of all paths.
    pub(super) fn addrs(&self) -> impl Iterator<Item = &transports::Addr> {
        self.paths.keys()
    }

    /// Returns whether this stores any addresses.
    pub(super) fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Replies to all pending resolve requests.
    ///
    /// This is a no-op if no requests are queued. Replies `Ok` if we have any known paths,
    /// otherwise with the provided `address_lookup_error` or with [`AddressLookupFailed::NoResults`].
    fn emit_pending_resolve_requests(&mut self, address_lookup_error: Option<AddressLookupFailed>) {
        if self.pending_resolve_requests.is_empty() {
            return;
        }
        let result = match (self.paths.is_empty(), address_lookup_error) {
            (false, _) => Ok(()),
            (true, Some(err)) => Err(err),
            (true, None) => Err(e!(AddressLookupFailed::NoResults { errors: Vec::new() })),
        };
        for tx in self.pending_resolve_requests.drain(..) {
            tx.send(result.clone()).ok();
        }
    }

    /// Prune paths.
    ///
    /// Should be invoked any time we insert a new path.
    ///
    /// We currently only prune non-relay paths. For more information on the
    /// criteria for when and which paths we prune, look at the [`prune_non_relay_paths`] function.
    pub(super) fn prune_paths(&mut self) {
        prune_non_relay_paths(&mut self.paths);
    }
}

/// The state of a single path to the remote endpoint.
///
/// Each path is identified by the destination [`transports::Addr`] and they are stored in
/// the [`RemotePathState`] map in [`RemoteStateActor`].
///
/// [`RemoteStateActor`]: super::RemoteStateActor
#[derive(Debug, Default)]
pub(super) struct PathState {
    /// How we learned about this path, and when.
    ///
    /// We keep track of only the latest [`Instant`] for each [`Source`], keeping the size
    /// of the map of sources down to one entry per type of source.
    pub(super) sources: HashMap<Source, Instant>,
    /// The usability status of this path.
    pub(super) status: PathStatus,
}

/// Prunes the non-relay paths in the paths HashMap.
///
/// Only prunes if the number of non-relay paths is above [`MAX_NON_RELAY_PATHS`].
///
/// Keeps paths that are open or of unknown status.
///
/// Always prunes paths that have unsuccessfully holepunched.
///
/// Keeps [`MAX_INACTIVE_NON_RELAY_PATHS`] of the most recently closed paths
/// that are not currently being used but have successfully been
/// holepunched previously.
///
/// This all ensures that:
///
/// - We do not have unbounded growth of paths.
/// - If we have many paths for this remote, we prune the paths that cannot hole punch.
/// - We do not prune holepunched paths that are currently not in use too quickly. For
///   example, if a large number of untested paths are added at once, we will not
///   immediately prune all of the unused, but valid, paths at once.
fn prune_non_relay_paths(paths: &mut FxHashMap<transports::Addr, PathState>) {
    // if the total number of paths is less than the max, bail early
    if paths.len() < MAX_NON_RELAY_PATHS {
        return;
    }

    let primary_paths: Vec<_> = paths.iter().filter(|(addr, _)| !addr.is_relay()).collect();

    // if the total number of non-relay paths is less than the max, bail early
    if primary_paths.len() < MAX_NON_RELAY_PATHS {
        return;
    }

    // paths that were opened at one point but have previously been closed
    let mut inactive = Vec::with_capacity(primary_paths.len());
    // paths where we attempted hole punching but it not successful
    let mut failed = Vec::with_capacity(primary_paths.len());

    for (addr, state) in primary_paths {
        match state.status {
            PathStatus::Inactive(t) => {
                // paths where holepunching succeeded at one point, but the path was closed.
                inactive.push((addr.clone(), t));
            }
            PathStatus::Unusable => {
                // paths where holepunching has been attempted and failed.
                failed.push(addr.clone());
            }
            _ => {
                // ignore paths that are open or the status is unknown
            }
        }
    }

    // All paths are bad, don't prune all of them.
    //
    // This implies that `inactive` is empty.
    if failed.len() == paths.len() {
        // leave the max number of non-relay paths
        failed.truncate(paths.len().saturating_sub(MAX_NON_RELAY_PATHS));
    }

    // sort the potentially prunable from most recently closed to least recently closed
    inactive.sort_by_key(|b| std::cmp::Reverse(b.1));

    // Prune the "oldest" closed paths.
    let old_inactive =
        inactive.split_off(inactive.len().saturating_sub(MAX_INACTIVE_NON_RELAY_PATHS));

    // collect all the paths that should be pruned
    let must_prune: HashSet<_> = failed
        .into_iter()
        .chain(old_inactive.into_iter().map(|(addr, _)| addr))
        .collect();

    paths.retain(|addr, _| !must_prune.contains(addr));
}
