//! Safe projections of the Rust-owned remote state; response bodies use explicit query only.
use crate::*;
use velune_remote::{DeviceLocator, PairingPermission};

#[derive(Clone, uniffi::Record)]
pub struct BindingPairingOfferInfo {
    pub permission: PairingPermission,
    pub issuer: String,
    pub name: String,
    pub expires_at_unix_seconds: u64,
    pub locator: DeviceLocator,
}

/// Bounded offline parsing; no service, socket, identity, or token persistence is required.
#[uniffi::export]
pub fn decode_remote_offer(encoded: String) -> Result<BindingPairingOfferInfo, BindingError> {
    let offer = velune_remote::PairingOffer::decode(&encoded)?;
    Ok(BindingPairingOfferInfo {
        permission: offer.permission,
        issuer: offer.issuer,
        name: offer.name,
        expires_at_unix_seconds: offer.expires_at_unix_seconds,
        locator: offer.locator,
    })
}
