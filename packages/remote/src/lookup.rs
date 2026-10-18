//! Per-peer lookup hints resolve only that peer and never publish this device to contact URLs.
use crate::*;
use futures_util::{
    StreamExt,
    stream::{self, Stream},
};
use iroh::address_lookup::{AddressLookup, Error, Item, PkarrResolver};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{Arc, Mutex},
};

type Observations = Arc<Mutex<BTreeMap<String, FacilityReachability>>>;
#[derive(Clone, Default)]
pub(crate) struct LookupObservations(Observations);
impl LookupObservations {
    pub(crate) fn statuses(&self) -> Result<Vec<FacilityStatus>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .iter()
            .map(|(url, status)| FacilityStatus {
                kind: FacilityKind::Lookup,
                url: url.clone(),
                reachability: *status,
            })
            .collect())
    }
    fn record(&self, url: String, reachability: FacilityReachability) {
        if let Ok(mut observations) = self.0.lock() {
            if observations.len() >= 2048 && !observations.contains_key(&url) {
                observations.pop_first();
            }
            observations.insert(url, reachability);
        }
    }
    pub(crate) fn observe<T>(&self, url: String, inner: T) -> ObservedLookup<T> {
        if !self
            .0
            .lock()
            .is_ok_and(|observations| observations.contains_key(&url))
        {
            self.record(url.clone(), FacilityReachability::Unknown);
        }
        ObservedLookup {
            url,
            inner,
            observations: self.clone(),
        }
    }
}
pub(crate) struct ObservedLookup<T> {
    url: String,
    inner: T,
    observations: LookupObservations,
}
impl<T> std::fmt::Debug for ObservedLookup<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ObservedLookup")
    }
}
impl<T: AddressLookup> AddressLookup for ObservedLookup<T> {
    fn publish(&self, data: &iroh::address_lookup::EndpointData) {
        self.inner.publish(data);
    }
    fn resolve(
        &self,
        peer: iroh::EndpointId,
    ) -> Option<Pin<Box<dyn Stream<Item = std::result::Result<Item, Error>> + Send + 'static>>>
    {
        let stream = self.inner.resolve(peer)?;
        let url = self.url.clone();
        let observations = self.observations.clone();
        Some(Box::pin(stream.map(move |result| {
            observations.record(
                url.clone(),
                if result.is_ok() {
                    FacilityReachability::Reachable
                } else {
                    FacilityReachability::Unavailable
                },
            );
            result
        })))
    }
}
#[derive(Debug)]
pub(crate) struct ObservedResolverBuilder {
    pub url: String,
    pub observations: LookupObservations,
}
impl std::fmt::Debug for LookupObservations {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LookupObservations")
    }
}
impl iroh::address_lookup::AddressLookupBuilder for ObservedResolverBuilder {
    fn into_address_lookup(
        self,
        endpoint: &iroh::Endpoint,
    ) -> std::result::Result<impl AddressLookup, iroh::address_lookup::AddressLookupBuilderError>
    {
        let url: iroh::RelayUrl = self.url.parse().map_err(|error| {
            iroh::address_lookup::AddressLookupBuilderError::from_err("pkarr", error)
        })?;
        let resolver = PkarrResolver::builder(url.into()).into_address_lookup(endpoint)?;
        Ok(self.observations.observe(self.url, resolver))
    }
}

type ResolverMap = BTreeMap<iroh::EndpointId, Vec<Arc<dyn AddressLookup>>>;
#[derive(Clone, Default)]
pub(crate) struct PeerLookup(Arc<Mutex<ResolverMap>>);
impl std::fmt::Debug for PeerLookup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PeerLookup")
    }
}
impl PeerLookup {
    pub(crate) fn configure(
        &self,
        peer: &str,
        locator: &DeviceLocator,
        endpoint: &iroh::Endpoint,
        observations: &LookupObservations,
        withdrawn: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        locator.validate()?;
        let resolvers: Vec<Arc<dyn AddressLookup>> = locator
            .lookup_urls
            .iter()
            .filter(|url| {
                crate::endpoint::validate_relay_url(url)
                    .is_ok_and(|url| !withdrawn.contains(url.as_str()))
            })
            .map(|url| {
                let url = crate::endpoint::validate_relay_url(url)?;
                let canonical = url.to_string();
                Ok(Arc::new(
                    observations.observe(
                        canonical,
                        PkarrResolver::builder(url.into())
                            .dns_resolver(
                                endpoint
                                    .dns_resolver()
                                    .map_err(|_| RemoteError::Unavailable)?
                                    .clone(),
                            )
                            .build(endpoint.tls_config().clone()),
                    ),
                ) as Arc<dyn AddressLookup>)
            })
            .collect::<Result<_>>()?;
        let peer = crate::store::parse_peer(peer)?;
        let mut entries = self.0.lock().map_err(|_| RemoteError::Unavailable)?;
        if !entries.contains_key(&peer) && entries.len() >= 384 {
            return Err(RemoteError::Capacity);
        }
        entries.insert(peer, resolvers);
        Ok(())
    }
    pub(crate) fn clear(&self) {
        if let Ok(mut entries) = self.0.lock() {
            entries.clear();
        }
    }
}
impl AddressLookup for PeerLookup {
    fn resolve(
        &self,
        peer: iroh::EndpointId,
    ) -> Option<Pin<Box<dyn Stream<Item = std::result::Result<Item, Error>> + Send + 'static>>>
    {
        let entries = self.0.lock().ok()?;
        let resolvers = entries.get(&peer)?;
        let streams = resolvers
            .iter()
            .filter_map(|resolver| resolver.resolve(peer))
            .collect::<Vec<_>>();
        if streams.is_empty() {
            return None;
        }
        Some(Box::pin(stream::select_all(streams)))
    }
}
