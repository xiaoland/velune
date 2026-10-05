//! Small protocol-neutral HTTP result types shared by native operations.
//! No HTTP client or header implementation is exposed from the domain crate.

use crate::Payload;

#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Header")
            .field("name", &self.name)
            .field("value", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMeta {
    pub status: u16,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseBody {
    pub meta: ResponseMeta,
    pub body: Payload<Vec<u8>>,
}
