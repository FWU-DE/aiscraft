//! The image of the private computer (#94): where it comes from and how it is checked.
//!
//! Source (owner's decision 2026-11-25): **n8n Nod's own Windows runtime**, unchanged. Every Nod
//! release (`n8n-io/nod`, Apache-2.0) publishes `nod-Runtime-win32-x64.tar.gz`: a WSL2 root file
//! system (Ubuntu 24.04 with the Docker Engine in `/usr/local/bin`, Windows drives and interop
//! off) with Nod's container images preloaded under `/opt/nod-assets/images`; AIS.ChatCraft uses
//! only the workspace image (Debian 12 with Python 3, LibreOffice, Tesseract, qpdf, Ghostscript,
//! Noto fonts) and starts it locked down (`container.rs`).
//!
//! Pinned like Nod pins it: the release, the runtime's SHA-256 and size, and the workspace
//! image's id and tar checksum are fixed in the block below. The download must match the pin
//! and its line in the release's `SHA256SUMS-win32-x64`; a wrong checksum keeps nothing. The
//! file arrives staged (`disk::Staged`), never in memory.
//!
//! Updates as in Nod: `.github/workflows/nod-runtime.yml` looks for a new Nod release every week,
//! verifies its signed release statement (sigstore, Nod's `desktop-release.yml`), and opens a
//! pull request that updates this block. An app built with new pins offers the new runtime to
//! computers that run an older one (`sandbox.update`, again only after a yes); the comparison is
//! the installed runtime's checksum against the pinned one, as Nod compares its guest.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use sha2::{Digest as _, Sha256};

use crate::Ctx;

// ---- pinned Nod release: updated by .github/workflows/nod-runtime.yml, keep one value per line ---
pub const NOD_TAG: &str = "nod-v0.9.7";
pub const RUNTIME_ASSET: &str = "nod-Runtime-win32-x64.tar.gz";
pub const RUNTIME_SHA256: &str = "ae57683dab01aec443cb038ef94332c0e67fc725f4f08091e7ff21504142ca64";
pub const RUNTIME_BYTES: u64 = 1332058697;
pub const WORKSPACE_ID: &str = "sha256:8da90f7a2e66134dc6405f363529d8e0ffba0da0ef2c8798825697d98a2439cd";
pub const WORKSPACE_TAR: &str = "/opt/nod-assets/images/workspace.tar.zst";
pub const WORKSPACE_TAR_SHA256: &str = "6575a1360af55fc762a37befa23a90db4b50fc1e306873cc29ee7cc7d0f5f8cc";
// ---- end of the pinned Nod release ---------------------------------------------------------------

/// The release's checksum file for Windows.
pub const SUMS: &str = "SHA256SUMS-win32-x64";
/// What the question says before the download.
pub const APPROX_MB: u64 = RUNTIME_BYTES.div_ceil(1_000_000);
pub const DISK_MB: u64 = 5000;
pub const LICENSE: &str = "Laufzeit von n8n Nod (Apache-2.0): Ubuntu 24.04 (freie Lizenzen, überwiegend GPL) mit Docker Engine (Apache-2.0); Arbeitsbereich: Debian 12 mit Python 3 (PSF), LibreOffice (MPL-2.0), Tesseract (Apache-2.0), ffmpeg (LGPL/GPL), qpdf (Apache-2.0), Ghostscript (AGPL-3.0), Noto-Schriften (OFL-1.1)";
pub const SOURCE_TEXT: &str = "github.com/n8n-io/nod, die Windows-Laufzeit von n8n Nod (nod-Runtime-win32-x64.tar.gz), unverändert, mit fester Prüfsumme";
/// Largest image accepted.
const MAX_IMAGE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;

/// Where the image comes from.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    /// The folder with the sums file and the asset.
    pub base: String,
    pub asset: String,
    pub sums: String,
    pub pinned: Option<String>,
    /// The size the pin names; a download of another size is refused.
    pub bytes: Option<u64>,
    /// How people read the version („nod-v0.9.7“).
    pub version: String,
}

impl Source {
    /// The pinned Nod release on GitHub; `None` where Nod publishes no runtime (ARM).
    pub(crate) fn github() -> Option<Source> {
        cfg!(target_arch = "x86_64").then(|| Source {
            base: format!("https://github.com/n8n-io/nod/releases/download/{NOD_TAG}"),
            asset: RUNTIME_ASSET.into(),
            sums: SUMS.into(),
            pinned: Some(RUNTIME_SHA256.into()),
            bytes: Some(RUNTIME_BYTES),
            version: NOD_TAG.into(),
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
    let sums_resp = client.get(format!("{}/{}", src.base, src.sums)).send().await.map_err(net)?;
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
    let total = resp.content_length().or(src.bytes).unwrap_or(APPROX_MB * 1_000_000);
    if src.bytes.is_some_and(|b| b != total) {
        return Err(Error::protocol("Das Abbild hat nicht die eingetragene Größe; nichts geladen."));
    }
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
    if sum != expected || src.bytes.is_some_and(|b| b != got) {
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
    fn sums_lines_and_the_pinned_nod_release() {
        let a = "a".repeat(64);
        let sums = format!("{}  nod-Setup-x64.exe\n{a}  nod-Runtime-win32-x64.tar.gz\n", "b".repeat(64));
        assert_eq!(sum_for(&sums, RUNTIME_ASSET), Some(a));
        assert_eq!(sum_for(&sums, "nod-Setup-x64.exe"), Some("b".repeat(64)));
        assert_eq!(sum_for("kaputt", RUNTIME_ASSET), None);
        // Every pin is there and well formed (the workflow rewrites them line by line).
        let hexes = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        assert!(NOD_TAG.starts_with("nod-v") && hexes(RUNTIME_SHA256) && hexes(WORKSPACE_TAR_SHA256));
        assert!(WORKSPACE_ID.strip_prefix("sha256:").is_some_and(hexes));
        assert!(RUNTIME_BYTES > 100_000_000 && WORKSPACE_TAR.starts_with("/opt/nod-assets/images/"));
        if let Some(s) = Source::github() {
            assert_eq!(s.base, format!("https://github.com/n8n-io/nod/releases/download/{NOD_TAG}"));
            assert_eq!((s.pinned.as_deref(), s.bytes), (Some(RUNTIME_SHA256), Some(RUNTIME_BYTES)));
        }
    }
}
