//! The image of the private computer (#94, #102): where it comes from and how it is checked.
//!
//! Source (owner's decision 2026-11-25): **n8n Nod's own runtime**, unchanged (`n8n-io/nod`,
//! Apache-2.0).
//!
//! - **Windows:** `nod-Runtime-win32-x64.tar.gz`, a WSL2 root file system (Ubuntu 24.04 with the
//!   Docker Engine in `/usr/local/bin`, Windows drives and interop off) with Nod's container images
//!   preloaded under `/opt/nod-assets/images`.
//! - **Linux (#102):** as Nod does it on Linux, Lima with QEMU/KVM: Lima's own Linux build
//!   (`lima-<v>-Linux-x86_64.tar.gz`, the version and checksum Nod pins) and the guest parts of
//!   Nod's x86-64 runtime archive (`nod-Runtime-darwin-x64-….tar.gz`): the Ubuntu disk
//!   `guest.img`, Docker for the guest, the iptables packages, the workspace image and Nod's
//!   `manifest.json` with the image's id. Only the guest parts are unpacked; the macOS programs
//!   in the archive are never used.
//!
//! AIS.ChatCraft uses only the workspace image (Debian 12 with Python 3, LibreOffice, Tesseract,
//! qpdf, Ghostscript, Noto fonts) and starts it locked down (`container.rs`).
//!
//! Pinned like Nod pins it: every download's SHA-256 and size are fixed below; where the
//! publisher has a sums file (Nod's `SHA256SUMS-win32-x64`, Lima's `SHA256SUMS`) the line must
//! match too. Every file taken out of an archive is checked against its own pin. A wrong checksum
//! keeps nothing. Downloads arrive staged (`disk::Staged`), never in memory.
//!
//! Updates as in Nod: `.github/workflows/nod-runtime.yml` looks for a new Nod release every week,
//! verifies its signed release statement (sigstore, Nod's `desktop-release.yml`), and opens a
//! pull request that updates the Windows block below. An app built with new pins offers the new
//! runtime to computers that run an older one (`sandbox.update`, only after a yes); the
//! comparison is the installed downloads' checksums against the pinned ones, as Nod compares its
//! guest.

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

// ---- Linux (#102): Lima and the guest parts of Nod's x86-64 runtime ------------------------------
// Lima: Nod's `desktop/runtime/downloads.json` (`linux.x64.lima`), also in Lima's SHA256SUMS.
pub const LIMA_VERSION: &str = "2.2.0";
pub const LIMA_URL: &str = "https://github.com/lima-vm/lima/releases/download/v2.2.0/lima-2.2.0-Linux-x86_64.tar.gz";
pub const LIMA_SUMS_URL: &str = "https://github.com/lima-vm/lima/releases/download/v2.2.0/SHA256SUMS";
pub const LIMA_SHA256: &str = "a0ea1ccf6b7335a900adb5f8d2b8384457965fecb1ba72f09b4e3e46d12f424a";
pub const LIMA_BYTES: u64 = 23968157;
// Nod's runtime archive: `runtime-darwin-x64-<fingerprint>.json` of nod-v0.9.7 (covered by the
// release's signed statement) names it and every file in it.
pub const NOD_ARCHIVE_URL: &str = "https://github.com/n8n-io/nod/releases/download/nod-v0.9.5/nod-Runtime-darwin-x64-f1e9db832de9.tar.gz";
pub const NOD_ARCHIVE_SHA256: &str = "136d61be92ca405d1b3e091aa620a218a9d22704ab037d7ec269d5a936cb9efe";
pub const NOD_ARCHIVE_BYTES: u64 = 1743554674;
/// The files taken out of Nod's archive for the Linux guest, with their pinned checksums and sizes.
pub const LINUX_GUEST_FILES: &[(&str, &str, u64)] = &[
    ("manifest.json", "f410c50a0a1d5ac3f39bc89d6b09faab8f504f7a999c1329b77f4a95268fcbd3", 1402),
    ("guest.img", "7bdaa53cf909472929f353e19efc9b7bbe30fb7acc05c033ce7b8d46dd62d38d", 421224448),
    ("docker-guest.tar.gz", "5c0d19f31fece1accd0358bb8cff591fe25d7b6cba19f0fd412cbfdc07f75ff6", 82150845),
    ("guest-packages/iptables_1.8.11-2ubuntu3_amd64.deb", "187b16f9375f53ce6a1618a29de52bb177b620589ada29e4a3e25de5d594e66f", 381074),
    ("guest-packages/libip4tc2_1.8.11-2ubuntu3_amd64.deb", "be25148d60125f6866dfa7940f3f1371fff4b0e8a1a9500780ad50cb8aaa0895", 24182),
    ("guest-packages/libip6tc2_1.8.11-2ubuntu3_amd64.deb", "3d9fdb3171f1167ab3f5182de11db63bcd27aae71838618f3d46806b3134428e", 24446),
    ("guest-packages/libnetfilter-conntrack3_1.1.1-1_amd64.deb", "facfd45a7cf6bab8ef30adbe693d6486f22e5253263f9d53b4e70b53f768970e", 47520),
    ("guest-packages/libnfnetlink0_1.0.2-3build1_amd64.deb", "e76b638fef2c1ebf78260972a52c4716df5752b29a952aeddc0c091a74f179c2", 15078),
    ("guest-packages/libnftnl11_1.3.1-1_amd64.deb", "a2f4d4a4ce3fac7e473c8de60a3d8948a58e9850cd8e2ced17263aa73dda7199", 72288),
    ("images/workspace.tar.zst", "3bc8bae74df3582d49dc66bbdb59960c634ba8fef8a827f36c5ccbfd0e7bfd97", 653537446),
];
pub const LINUX_WORKSPACE_TAR_SHA256: &str = "3bc8bae74df3582d49dc66bbdb59960c634ba8fef8a827f36c5ccbfd0e7bfd97";
/// How people read the Linux version.
pub const LINUX_VERSION: &str = "nod-v0.9.5 (Linux, Lima 2.2.0)";
// ---- end of the Linux pins -----------------------------------------------------------------------

/// The release's checksum file for Windows.
pub const SUMS: &str = "SHA256SUMS-win32-x64";
/// What the question says before the download, for this system.
pub const APPROX_MB: u64 = if cfg!(target_os = "linux") { (LIMA_BYTES + NOD_ARCHIVE_BYTES).div_ceil(1_000_000) } else { RUNTIME_BYTES.div_ceil(1_000_000) };
pub const DISK_MB: u64 = if cfg!(target_os = "linux") { 8000 } else { 5000 };
pub const LICENSE: &str = if cfg!(target_os = "linux") {
    "Lima (Apache-2.0); Laufzeit von n8n Nod (Apache-2.0): Ubuntu (freie Lizenzen, überwiegend GPL) mit Docker Engine (Apache-2.0); Arbeitsbereich: Debian 12 mit Python 3 (PSF), LibreOffice (MPL-2.0), Tesseract (Apache-2.0), ffmpeg (LGPL/GPL), qpdf (Apache-2.0), Ghostscript (AGPL-3.0), Noto-Schriften (OFL-1.1); QEMU vom System (GPL-2.0)"
} else {
    "Laufzeit von n8n Nod (Apache-2.0): Ubuntu 24.04 (freie Lizenzen, überwiegend GPL) mit Docker Engine (Apache-2.0); Arbeitsbereich: Debian 12 mit Python 3 (PSF), LibreOffice (MPL-2.0), Tesseract (Apache-2.0), ffmpeg (LGPL/GPL), qpdf (Apache-2.0), Ghostscript (AGPL-3.0), Noto-Schriften (OFL-1.1)"
};
pub const SOURCE_TEXT: &str = if cfg!(target_os = "linux") {
    "github.com/lima-vm/lima (Lima 2.2.0 für Linux) und github.com/n8n-io/nod (die Laufzeit von n8n Nod, davon nur der Linux-Rechner und der Arbeitsbereich), unverändert, mit festen Prüfsummen"
} else {
    "github.com/n8n-io/nod, die Windows-Laufzeit von n8n Nod (nod-Runtime-win32-x64.tar.gz), unverändert, mit fester Prüfsumme"
};
/// Largest download accepted.
const MAX_IMAGE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;

/// One file to download.
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub url: String,
    /// The publisher's `sha256sum` file that must list the download too.
    pub sums_url: Option<String>,
    /// The download's name in that file.
    pub name: String,
    pub sha256: Option<String>,
    /// The size the pin names; a download of another size is refused.
    pub bytes: Option<u64>,
    /// The name it is kept under in the sandbox folder until the computer is created.
    pub file: String,
}

/// Where the image comes from.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    pub parts: Vec<Part>,
    /// How people read the version („nod-v0.9.7“).
    pub version: String,
}

impl Source {
    /// The pinned downloads for this system; `None` where there are none (ARM, macOS).
    pub(crate) fn github() -> Option<Source> {
        if !cfg!(target_arch = "x86_64") {
            return None;
        }
        if cfg!(windows) {
            Some(Source {
                parts: vec![Part {
                    url: format!("https://github.com/n8n-io/nod/releases/download/{NOD_TAG}/{RUNTIME_ASSET}"),
                    sums_url: Some(format!("https://github.com/n8n-io/nod/releases/download/{NOD_TAG}/{SUMS}")),
                    name: RUNTIME_ASSET.into(),
                    sha256: Some(RUNTIME_SHA256.into()),
                    bytes: Some(RUNTIME_BYTES),
                    file: "image.tar.gz".into(),
                }],
                version: NOD_TAG.into(),
            })
        } else if cfg!(target_os = "linux") {
            Some(Source { parts: linux_parts(), version: LINUX_VERSION.into() })
        } else {
            None
        }
    }

    /// What identifies the installed downloads: their pinned checksums (empty without pins).
    pub(crate) fn id(&self) -> Option<String> {
        let sums: Option<Vec<String>> = self.parts.iter().map(|p| p.sha256.clone()).collect();
        sums.map(|s| s.join("+"))
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.parts.iter().map(|p| p.bytes.unwrap_or(0)).sum()
    }
}

/// The Linux downloads (#102): Lima, then Nod's runtime archive.
pub(crate) fn linux_parts() -> Vec<Part> {
    vec![
        Part {
            url: LIMA_URL.into(),
            sums_url: Some(LIMA_SUMS_URL.into()),
            name: LIMA_URL.rsplit('/').next().unwrap_or("").into(),
            sha256: Some(LIMA_SHA256.into()),
            bytes: Some(LIMA_BYTES),
            file: "lima.tar.gz".into(),
        },
        Part {
            url: NOD_ARCHIVE_URL.into(),
            sums_url: None,
            name: NOD_ARCHIVE_URL.rsplit('/').next().unwrap_or("").into(),
            sha256: Some(NOD_ARCHIVE_SHA256.into()),
            bytes: Some(NOD_ARCHIVE_BYTES),
            file: "nod-runtime.tar.gz".into(),
        },
    ]
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of a file on disk, read in pieces.
pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok(hex(&h.finalize()))
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

/// Download every part into `dir` and check it. Returns the files and what identifies them (the
/// checksums joined). On any error nothing of the failed part stays.
pub(crate) async fn fetch(c: &Ctx, src: &Source, dir: &Path) -> Result<(Vec<PathBuf>, String)> {
    crate::disk::create_dir(dir)?;
    let client = client()?;
    let total = src.bytes();
    let mut before = 0u64;
    let mut files = Vec::new();
    let mut sums = Vec::new();
    for part in &src.parts {
        let (file, sum, got) = fetch_part(c, &client, part, dir, before, total).await?;
        before += got;
        files.push(file);
        sums.push(sum);
    }
    c.update(|s| s.sandbox.progress = Some((before, before)));
    Ok((files, sums.join("+")))
}

async fn fetch_part(c: &Ctx, client: &reqwest::Client, part: &Part, dir: &Path, before: u64, total: u64) -> Result<(PathBuf, String, u64)> {
    let host = reqwest::Url::parse(&part.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let net = |e: reqwest::Error| Error::network(format!("{} von {host} nicht geladen: {e}. Internetverbindung prüfen und noch einmal versuchen.", part.name));
    let mut expected = part.sha256.clone();
    if let Some(sums_url) = &part.sums_url {
        let sums_resp = client.get(sums_url).send().await.map_err(net)?;
        if !sums_resp.status().is_success() {
            return Err(Error::network(format!("Prüfsummen von {host} nicht geladen: Antwort {}.", sums_resp.status())));
        }
        // Read capped while it arrives: a server (or a redirect target) cannot fill the memory.
        let sums_bytes = aiscraft_backends::http::read_bytes_capped(sums_resp, MAX_SUMS, "Die Prüfsummendatei").await.map_err(|e| {
            if e.kind == aiscraft_core::ErrorKind::Protocol { Error::protocol("Die Prüfsummendatei ist zu groß; nichts geladen.") } else { e }
        })?;
        let listed = sum_for(&String::from_utf8_lossy(&sums_bytes), &part.name)
            .ok_or_else(|| Error::protocol(format!("{} fehlt in der Prüfsummendatei; nichts geladen.", part.name)))?;
        if expected.as_ref().is_some_and(|pin| pin != &listed) {
            return Err(Error::protocol(
                "Die veröffentlichte Prüfsumme des Abbilds stimmt nicht mit der in AIS.ChatCraft eingetragenen überein. Nichts geladen; bitte AIS.ChatCraft aktualisieren.",
            ));
        }
        expected = Some(listed);
    }
    let expected = expected.ok_or_else(|| Error::protocol(format!("Für {} ist keine Prüfsumme eingetragen; nichts geladen.", part.name)))?;
    let target = dir.join(&part.file);
    let mut resp = client.get(&part.url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(Error::network(format!("{} von {host} nicht geladen: Antwort {}.", part.name, resp.status())));
    }
    let size = resp.content_length().or(part.bytes).unwrap_or(0);
    if part.bytes.is_some_and(|b| b != size) {
        return Err(Error::protocol("Das Abbild hat nicht die eingetragene Größe; nichts geladen."));
    }
    if size > MAX_IMAGE {
        return Err(Error::protocol("Das Abbild ist größer als erwartet; nichts geladen."));
    }
    let total = total.max(before + size);
    let mut staged = crate::disk::Staged::create(&target)?;
    let mut hash = Sha256::new();
    let (mut got, mut shown) = (0u64, 0u64);
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        got += chunk.len() as u64;
        if got > MAX_IMAGE || part.bytes.is_some_and(|b| got > b) {
            return Err(Error::protocol("Das Abbild ist größer als erwartet; nichts geladen."));
        }
        hash.update(&chunk);
        staged.write(&chunk)?;
        if got - shown >= 8_000_000 {
            shown = got;
            c.update(|s| s.sandbox.progress = Some((before + got, total.max(before + got))));
        }
    }
    let sum = hex(&hash.finalize());
    if sum != expected || part.bytes.is_some_and(|b| b != got) {
        return Err(Error::protocol(format!(
            "Die Prüfsumme von {} stimmt nicht (erwartet {expected}, erhalten {sum}). Die Datei wurde verworfen, nichts eingerichtet.",
            part.name
        )));
    }
    staged.finish(&target)?;
    Ok((target, sum, got))
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
            assert!(s.parts.iter().all(|p| p.url.starts_with("https://github.com/") && p.sha256.as_deref().is_some_and(hexes) && p.bytes.is_some()));
            assert_eq!(s.id().map(|i| i.split('+').count()), Some(s.parts.len()));
        }
    }

    #[test]
    fn linux_pins_are_complete() {
        let hexes = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        assert!(hexes(LIMA_SHA256) && hexes(NOD_ARCHIVE_SHA256) && LIMA_URL.contains(LIMA_VERSION));
        for (path, sha, bytes) in LINUX_GUEST_FILES {
            assert!(hexes(sha) && *bytes > 0 && !path.starts_with('/') && !path.contains(".."), "{path}");
        }
        for need in ["manifest.json", "guest.img", "docker-guest.tar.gz", "images/workspace.tar.zst"] {
            assert!(LINUX_GUEST_FILES.iter().any(|(p, ..)| *p == need), "{need}");
        }
        assert!(LINUX_GUEST_FILES.iter().any(|(p, s, _)| *p == "images/workspace.tar.zst" && *s == LINUX_WORKSPACE_TAR_SHA256));
        let parts = linux_parts();
        assert_eq!(parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["lima-2.2.0-Linux-x86_64.tar.gz", "nod-Runtime-darwin-x64-f1e9db832de9.tar.gz"]);
    }
}
