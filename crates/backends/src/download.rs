//! Downloading a file the person asked for (the optional H5P player, #62): GitHub release
//! downloads redirect to their storage host, so redirects are followed (at most five). The body
//! is capped; a larger answer is an error, never a full memory.

use std::time::Duration;

use aiscraft_core::{Error, Result};

use crate::http;

/// Fetch `url` whole: at most `cap` bytes, within `timeout`. `what` names it in errors.
pub async fn fetch(url: &str, cap: usize, timeout: Duration, what: &str) -> Result<Vec<u8>> {
    let client = http::client(true)?;
    let resp = client.get(url).timeout(timeout).send().await.map_err(http::net_err)?;
    if !resp.status().is_success() {
        return Err(http::status_error(resp, what).await);
    }
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(Error::protocol(format!("{what} ist größer als {} MB", cap / (1024 * 1024))));
    }
    http::read_bytes_capped(resp, cap, what).await
}
