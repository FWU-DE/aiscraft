//! Pictures of assistants, characters and learning scenarios. AIS.chat hands out signed,
//! short-lived storage URLs; they are fetched without the session cookie.

use std::time::Duration;

use aiscraft_core::{Error, Result};

use crate::http;

/// The largest picture fetched.
pub const PICTURE_CAP: usize = 2 * 1024 * 1024;

const TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Does `bytes` start like one of the formats in [`TYPES`]?
pub fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()))
}

/// Fetch a picture: at most [`PICTURE_CAP`] bytes, PNG, JPEG, WebP or GIF, within 20 seconds.
/// Returns the bytes and their content type.
pub async fn download(url: &str) -> Result<(Vec<u8>, String)> {
    let client = http::client(false)?;
    let resp = client.get(url).timeout(Duration::from_secs(20)).send().await.map_err(http::net_err)?;
    if !resp.status().is_success() {
        return Err(http::status_error(resp, "Bild").await);
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !TYPES.contains(&ctype.as_str()) {
        return Err(Error::protocol(format!("Kein Bild (PNG, JPEG, WebP oder GIF), sondern „{}“", if ctype.is_empty() { "ohne Typ" } else { &ctype })));
    }
    if resp.content_length().is_some_and(|n| n > PICTURE_CAP as u64) {
        return Err(Error::protocol("Das Bild ist größer als 2 MB"));
    }
    let bytes = http::read_bytes_capped(resp, PICTURE_CAP, "Das Bild").await?;
    if !looks_like_image(&bytes) {
        return Err(Error::protocol("Die Bilddatei ist beschädigt oder kein Bild"));
    }
    Ok((bytes, ctype))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_signatures() {
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(looks_like_image(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(looks_like_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"GIF89a"));
        assert!(!looks_like_image(b"<html>"));
        assert!(!looks_like_image(b"RIFF\0\0\0\0WAVE"));
        assert!(!looks_like_image(b""));
    }
}
