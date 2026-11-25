//! The image of the private computer (#94): where it comes from and how it is checked.
//!
//! Source (owner's decision 2026-11-25, „wie Nod“): an image this repository builds itself and
//! publishes, never a registry or a script that installs packages at first start. The workflow
//! `.github/workflows/sandbox-image.yml` builds `packaging/sandbox` (Ubuntu 24.04 pinned by
//! digest, Docker Engine from Ubuntu, the workspace image with LibreOffice, Python 3, qpdf,
//! Ghostscript and Noto fonts already loaded) and publishes the root file system as a release
//! asset on the tag [`TAG`] with `SHA256SUMS.txt`. Why built here: one checksum covers
//! everything that runs, the installation needs no package mirror (school proxies) and the
//! containers start without any network.
//!
//! Checking as for the H5P player (#62): the download must match its line in the release's sums
//! file and, once the first image is published, the SHA-256 pinned in code ([`PINNED_SHA256`]),
//! so a replaced release asset is refused too. The file arrives staged (`disk::Staged`), never
//! in memory; a wrong checksum keeps nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use sha2::{Digest as _, Sha256};

use crate::Ctx;

/// The release that holds the image.
pub const TAG: &str = "sandbox-v1";
/// The image for x86-64 (the only one built so far).
pub const ASSET_X86_64: &str = "aiscraft-sandbox-1-x86_64.tar.gz";
pub const SUMS: &str = "SHA256SUMS.txt";
/// The image's SHA-256 once the first one is published (fill in from the release's sums file).
pub const PINNED_SHA256: Option<&str> = None;
/// What the question says before the download.
pub const APPROX_MB: u64 = 900;
pub const DISK_MB: u64 = 4000;
pub const LICENSE: &str = "Ubuntu 24.04 (freie Lizenzen, überwiegend GPL), Docker Engine (Apache-2.0), LibreOffice (MPL-2.0), Python 3 (PSF), qpdf (Apache-2.0), Ghostscript (AGPL-3.0), Noto-Schriften (OFL-1.1)";
pub const SOURCE_TEXT: &str =
    "github.com/FWU-DE/aiscraft, Release „sandbox-v1“, gebaut vom Workflow sandbox-image.yml aus packaging/sandbox (Ubuntu 24.04 mit fester Prüfsumme)";
/// Largest image accepted.
const MAX_IMAGE: u64 = 3 * 1024 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;

/// Where the image comes from.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    /// The folder with [`SUMS`] and the asset.
    pub base: String,
    pub asset: String,
    pub pinned: Option<String>,
}

impl Source {
    /// The release on GitHub; `None` where no image is built (ARM).
    pub(crate) fn github() -> Option<Source> {
        cfg!(target_arch = "x86_64").then(|| Source {
            base: format!("https://github.com/{}/releases/download/{TAG}", crate::update::REPO),
            asset: ASSET_X86_64.into(),
            pinned: PINNED_SHA256.map(str::to_string),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The checksum of `asset` in a `sha256sum` file.
pub(crate) fn sum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let sum = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == asset && sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit())).then(|| sum.to_ascii_lowercase())
    })
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .user_agent(concat!("AIS.ChatCraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::network(e.to_string()))
}

/// Download the image into `dir` and check it. Returns the file and its checksum. On any error
/// nothing of it stays.
pub(crate) async fn fetch(c: &Ctx, src: &Source, dir: &Path) -> Result<(PathBuf, String)> {
    crate::disk::create_dir(dir)?;
    let client = client()?;
    let host = reqwest::Url::parse(&src.base).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let net = |e: reqwest::Error| Error::network(format!("Abbild von {host} nicht geladen: {e}. Internetverbindung prüfen und noch einmal versuchen."));
    let sums_resp = client.get(format!("{}/{SUMS}", src.base)).send().await.map_err(net)?;
    if !sums_resp.status().is_success() {
        return Err(Error::network(format!("Prüfsummen von {host} nicht geladen: Antwort {}.", sums_resp.status())));
    }
    let sums_bytes = sums_resp.bytes().await.map_err(net)?;
    if sums_bytes.len() > MAX_SUMS {
        return Err(Error::protocol("Die Prüfsummendatei ist zu groß; nichts geladen."));
    }
    let expected = sum_for(&String::from_utf8_lossy(&sums_bytes), &src.asset)
        .ok_or_else(|| Error::protocol(format!("{} fehlt in der Prüfsummendatei; nichts geladen.", src.asset)))?;
    if let Some(pin) = &src.pinned
        && pin != &expected
    {
        return Err(Error::protocol(
            "Die veröffentlichte Prüfsumme des Abbilds stimmt nicht mit der in AIS.ChatCraft eingetragenen überein. Nichts geladen; bitte AIS.ChatCraft aktualisieren.",
        ));
    }
    let target = dir.join("image.tar.gz");
    let mut resp = client.get(format!("{}/{}", src.base, src.asset)).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(Error::network(format!("Abbild von {host} nicht geladen: Antwort {}.", resp.status())));
    }
    let total = resp.content_length().unwrap_or(APPROX_MB * 1_000_000);
    if total > MAX_IMAGE {
        return Err(Error::protocol("Das Abbild ist größer als erwartet; nichts geladen."));
    }
    let mut staged = crate::disk::Staged::create(&target)?;
    let mut hash = Sha256::new();
    let (mut got, mut shown) = (0u64, 0u64);
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        got += chunk.len() as u64;
        if got > MAX_IMAGE {
            return Err(Error::protocol("Das Abbild ist größer als erwartet; nichts geladen."));
        }
        hash.update(&chunk);
        staged.write(&chunk)?;
        if got - shown >= 8_000_000 {
            shown = got;
            c.update(|s| s.sandbox.progress = Some((got, total.max(got))));
        }
    }
    let sum = hex(&hash.finalize());
    if sum != expected {
        return Err(Error::protocol(format!(
            "Die Prüfsumme des Abbilds stimmt nicht (erwartet {expected}, erhalten {sum}). Die Datei wurde verworfen, nichts eingerichtet."
        )));
    }
    staged.finish(&target)?;
    c.update(|s| s.sandbox.progress = Some((got, got)));
    Ok((target, sum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_lines_and_the_release() {
        let a = "a".repeat(64);
        let sums = format!("{a}  aiscraft-sandbox-1-x86_64.tar.gz\n{}  *andere.zip\n", "b".repeat(64));
        assert_eq!(sum_for(&sums, ASSET_X86_64), Some(a));
        assert_eq!(sum_for(&sums, "andere.zip"), Some("b".repeat(64)));
        assert_eq!(sum_for("kaputt", ASSET_X86_64), None);
        if let Some(s) = Source::github() {
            assert!(s.base.starts_with("https://github.com/") && s.base.ends_with(TAG));
        }
        assert!(PINNED_SHA256.is_none_or(|p| p.len() == 64));
    }
}
