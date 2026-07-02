use std::time::Duration;

use lazy_static::lazy_static;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};

lazy_static! {
    // One client for all registry/tarball downloads, so connections and TLS
    // sessions are pooled instead of being rebuilt on every request.
    static ref SHARED_CLIENT: ClientWithMiddleware = build_client();
}

fn build_client() -> ClientWithMiddleware {
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);

    let client_builder = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(120))
        .deflate(true)
        .gzip(true)
        .brotli(true);
    let base_client = client_builder
        .build()
        .expect("reqwest::ClientBuilder::build()");

    ClientBuilder::new(base_client)
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build()
}

pub fn get_client() -> ClientWithMiddleware {
    // ClientWithMiddleware clones share the underlying connection pool.
    SHARED_CLIENT.clone()
}
