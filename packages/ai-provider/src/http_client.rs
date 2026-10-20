//! A shared connection pool with explicit, consumer-owned retry boundaries.
use reqwest::{Client, ClientBuilder};
/// Never redirects credentials or silently repeats model requests. Builder TLS,
/// proxy and timeout settings are retained; redirect/retry policy is enforced last.
#[derive(Clone)]
pub struct HttpClient {
    pub(crate) inner: Client,
}
impl HttpClient {
    pub fn new(builder: ClientBuilder) -> Result<Self, reqwest::Error> {
        builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map(|inner| Self { inner })
    }
}
