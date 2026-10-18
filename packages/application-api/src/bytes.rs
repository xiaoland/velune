//! Binary content stays compact on the JSON wire; no filesystem location is encoded here.
use base64::Engine;
use serde::{Deserialize, Deserializer, Serializer, de::Error};
pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
}
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let encoded = String::deserialize(deserializer)?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(D::Error::custom)
}
