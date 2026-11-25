//! The optional H5P player (#62): H5P content shown as in the learning platform.
//!
//! The player is h5p-standalone (MIT) with the H5P core (GPL-3.0) and the content-type libraries
//! (MIT). Because of the GPL it is never part of the app package: it is a separate download,
//! built by `packaging/h5p-player/build.sh` and published on the release tag [`TAG`] with
//! `SHA256SUMS.txt`. Nothing is downloaded without the owner's explicit yes
//! (`h5p.player_install` with `yes: true`, which the UI sends only from the dialog that names
//! size, source and licence).
//!
//! Checksum: the zip must match its line in the release's `SHA256SUMS.txt`, which catches a
//! broken or cut-off download. A hash pinned in code ([`PINNED_SHA256`]) is stronger, because a
//! replaced release asset (and its sums file) would then be refused too; it can only be filled in
//! once the first bundle is published, so until then the sums file is what is checked, and
//! afterwards both. The zip is unpacked as hostile input (`ext::package::unzip`: no absolute
//! paths, no `..`, no links, caps on size and count), only known file types, into a staging
//! folder that replaces `<data>/h5p-player` only when everything checked out.
//!
//! "Im Player ansehen" (`h5p.play`) writes the content (`h5p.json`, `content/…`, and libraries
//! an imported package brings) into `<data>/open/h5p-<id>` and starts `aiscraft-preview --h5p`,
//! which serves player, libraries and content from its own protocol with a policy that allows
//! that protocol and nothing on the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::commands::{CommandSpec, req_str};
use crate::ext::package;
use crate::{Ctx, NoticeLevel};

/// The bundle format this app understands; the release tag and file carry it.
pub const BUNDLE_VERSION: u64 = 1;
/// The release that holds the bundle.
pub const TAG: &str = "h5p-player-v1";
/// The bundle file in that release.
pub const ASSET: &str = "aiscraft-h5p-player-1.zip";
/// The checksum file in that release (`sha256sum` format).
pub const SUMS: &str = "SHA256SUMS.txt";
/// What the dialog says about the size.
pub const APPROX_MB: u64 = 3;
pub const LICENSE: &str = "GPL-3.0 (H5P-Kern), MIT (h5p-standalone und Inhaltstypen)";
/// The bundle's SHA-256, once the first bundle is published (see the module docs).
pub const PINNED_SHA256: Option<&str> = Some("137cdb1b6110deae8e0406e6a0fc08a3c18200aae09201db028ea19cb9779615");
/// `manifest.json` names the bundle so.
const MANIFEST_NAME: &str = "aiscraft-h5p-player";
/// Files the preview needs from a bundle.
const REQUIRED: &[&str] = &["manifest.json", "player/main.bundle.js", "player/frame.bundle.js", "player/styles/h5p.css"];
/// What the app writes next to an install: checksum, size, time.
const INSTALLED: &str = "installed.json";
const MAX_BUNDLE: usize = 64 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;
/// File types a bundle may hold: code and styles of player and libraries, pictures, fonts,
/// texts (licences). Anything else is refused. Keep in step with `ALLOWED` in
/// `packaging/h5p-player/assemble.js`.
const BUNDLE_TYPES: &[&str] = &[
    "js", "css", "json", "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "woff", "woff2", "ttf", "eot", "otf", "txt", "md", "mp3", "m4a", "ogg", "wav",
    "webm", "mp4", "vtt", "xml",
];

/// Where the player comes from.
#[derive(Debug, Clone)]
pub struct Source {
    /// The folder holding [`SUMS`] and [`ASSET`].
    pub base: String,
    pub pinned: Option<String>,
}

impl Source {
    /// The release on GitHub, with the pinned hash once there is one.
    pub fn github() -> Self {
        Self { base: format!("https://github.com/{}/releases/download/{TAG}", crate::update::REPO), pinned: PINNED_SHA256.map(str::to_string) }
    }
}

/// What the UI and agents see (`State::h5p_player`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerState {
    pub installed: bool,
    /// The installed bundle's version (0 = none).
    pub version: u64,
    /// Content types and libraries in the installed bundle.
    pub libraries: usize,
    /// Size of the downloaded zip.
    pub bytes: u64,
    pub installing: bool,
    /// Can this engine download it (the real app; not engines in memory)?
    pub can_download: bool,
    /// Is the preview program (`aiscraft-preview`) there to show it?
    pub preview: bool,
    pub approx_mb: u64,
    pub license: String,
    /// The release page.
    pub source: String,
    /// The result of the last install or removal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn release_page() -> String {
    format!("https://github.com/{}/releases/tag/{TAG}", crate::update::REPO)
}

fn source(c: &Ctx) -> Option<Source> {
    c.0.h5p_player_source.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The manifest of the bundle in `dir`, when it is a complete bundle of our format.
fn read_manifest(dir: &Path) -> Option<Value> {
    let path = dir.join("manifest.json");
    if std::fs::metadata(&path).ok()?.len() > MAX_MANIFEST {
        return None;
    }
    let m: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let ok = m.get("name").and_then(Value::as_str) == Some(MANIFEST_NAME)
        && m.get("version").and_then(Value::as_u64) == Some(BUNDLE_VERSION)
        && REQUIRED.iter().all(|f| dir.join(f).is_file());
    ok.then_some(m)
}

/// Is the player installed and complete?
pub(crate) fn installed(c: &Ctx) -> bool {
    read_manifest(&c.0.h5p_player_dir).is_some()
}

/// Reads what is on disk into the state (at start, after install and removal).
pub(crate) fn refresh(c: &Ctx) {
    let dir = &c.0.h5p_player_dir;
    let manifest = read_manifest(dir);
    let info: Value = std::fs::read(dir.join(INSTALLED)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let can_download = source(c).is_some();
    let preview = super::preview_program(c).is_some();
    c.update(|st| {
        let p = &mut st.h5p_player;
        p.installed = manifest.is_some();
        p.version = manifest.as_ref().and_then(|m| m.get("version")).and_then(Value::as_u64).unwrap_or(0);
        p.libraries = manifest.as_ref().and_then(|m| m.get("libraries")).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        p.bytes = info.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        p.can_download = can_download;
        p.preview = preview;
        p.approx_mb = APPROX_MB;
        p.license = LICENSE.into();
        p.source = release_page();
    });
}

/// The SHA-256 a `sha256sum` file gives for `file` (`<hex>  <name>` or `<hex> *<name>`).
pub(crate) fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().take(1000).find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Every file of an unpacked bundle has a known type, and the files the preview needs are there.
fn check_bundle(dir: &Path) -> Result<Value> {
    fn walk(dir: &Path, root: &Path, depth: usize, bad: &mut Option<String>) {
        if depth > 32 || bad.is_some() {
            return;
        }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, depth + 1, bad);
            } else {
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                if !BUNDLE_TYPES.contains(&ext.as_str()) {
                    *bad = Some(p.strip_prefix(root).unwrap_or(&p).display().to_string());
                }
            }
        }
    }
    let mut bad = None;
    walk(dir, dir, 0, &mut bad);
    if let Some(name) = bad {
        return Err(Error::invalid(format!("Der H5P-Player enthält eine Datei, die nicht dazugehört: {}", name.chars().take(120).collect::<String>())));
    }
    read_manifest(dir).ok_or_else(|| Error::invalid("Das ist kein vollständiger H5P-Player für diese Version von AIS.ChatCraft (manifest.json, player/…)"))
}

async fn install_from(c: &Ctx, src: &Source) -> Result<Value> {
    let sums = aiscraft_backends::download::fetch(&format!("{}/{SUMS}", src.base), MAX_SUMS, Duration::from_secs(60), "Die Prüfsummen des H5P-Players").await?;
    let expected = sum_for(&String::from_utf8_lossy(&sums), ASSET)
        .ok_or_else(|| Error::protocol(format!("{SUMS} der Veröffentlichung nennt den H5P-Player ({ASSET}) nicht")))?;
    if let Some(pin) = &src.pinned
        && !pin.eq_ignore_ascii_case(&expected)
    {
        return Err(Error::refused("Die veröffentlichte Prüfsumme des H5P-Players ist nicht die, die AIS.ChatCraft kennt. Nichts wurde installiert."));
    }
    let zip = aiscraft_backends::download::fetch(&format!("{}/{ASSET}", src.base), MAX_BUNDLE, Duration::from_secs(600), "Der H5P-Player").await?;
    let got = sha256_hex(&zip);
    if got != expected {
        return Err(Error::refused(format!(
            "Die Prüfsumme des geladenen H5P-Players stimmt nicht (erwartet {}…, erhalten {}…). Nichts wurde installiert; bitte später noch einmal versuchen.",
            expected.get(..12).unwrap_or(""),
            got.get(..12).unwrap_or("")
        )));
    }
    let dir = c.0.h5p_player_dir.clone();
    let staging = dir.with_extension("neu");
    package::remove(&staging);
    let checked = package::unzip(&zip, &staging).and_then(|()| check_bundle(&staging));
    let manifest = match checked {
        Ok(m) => m,
        Err(e) => {
            package::remove(&staging);
            return Err(e);
        }
    };
    let info = json!({"sha256": got, "bytes": zip.len(), "installed_at": crate::now_ms()});
    crate::disk::write(&staging.join(INSTALLED), info.to_string().as_bytes())?;
    package::remove(&dir);
    if let Err(e) = std::fs::rename(&staging, &dir) {
        package::remove(&staging);
        return Err(Error::io(format_args!("Ordner {}", dir.display()), &e));
    }
    Ok(json!({"installed": true, "sha256": got, "bytes": zip.len(), "version": manifest.get("version"), "path": dir.display().to_string()}))
}

async fn install(c: Ctx, p: Value) -> Result<Value> {
    if p.get("yes").and_then(Value::as_bool) != Some(true) {
        return Err(Error::refused(format!(
            "Der H5P-Player wird nur mit ausdrücklicher Zustimmung geladen (yes: true): ca. {APPROX_MB} MB von {}, Lizenz {LICENSE}.",
            release_page()
        )));
    }
    let src = source(&c).ok_or_else(|| Error::unsupported("In dieser Umgebung lädt AIS.ChatCraft nichts herunter (Tests, Skripte)"))?;
    let busy = c.update(|st| std::mem::replace(&mut st.h5p_player.installing, true));
    if busy {
        return Err(Error::busy("Der H5P-Player wird gerade schon geladen"));
    }
    let result = install_from(&c, &src).await;
    c.update(|st| {
        st.h5p_player.installing = false;
        st.h5p_player.message = Some(match &result {
            Ok(_) => "H5P-Player installiert.".into(),
            Err(e) => e.message.clone(),
        });
    });
    refresh(&c);
    if result.is_ok() {
        c.notice(NoticeLevel::Success, "H5P-Player installiert. „Im Player ansehen“ zeigt H5P-Inhalte jetzt wie in der Lernplattform.");
    }
    result
}

async fn remove(c: Ctx) -> Result<Value> {
    if c.state().h5p_player.installing {
        return Err(Error::busy("Der H5P-Player wird gerade geladen"));
    }
    let dir = c.0.h5p_player_dir.clone();
    let was = dir.exists();
    package::remove(&dir);
    package::remove(&dir.with_extension("neu"));
    if dir.exists() {
        return Err(Error::storage(format!("Der Ordner {} ließ sich nicht löschen", dir.display())));
    }
    c.update(|st| st.h5p_player.message = Some(if was { "H5P-Player entfernt.".into() } else { "Der H5P-Player war nicht installiert.".into() }));
    refresh(&c);
    Ok(json!({"removed": was}))
}

fn status(c: &Ctx) -> Value {
    refresh(c);
    let st = c.state();
    let mut v = serde_json::to_value(&st.h5p_player).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("path".into(), json!(c.0.h5p_player_dir.display().to_string()));
        o.insert("file".into(), json!(ASSET));
        o.insert("pinned".into(), json!(PINNED_SHA256.is_some()));
    }
    v
}

/// Where the content of artifact `id` is written for the player.
pub(crate) fn content_dir(c: &Ctx, id: &str) -> PathBuf {
    let short: String = id.chars().filter(char::is_ascii_alphanumeric).take(16).collect();
    c.0.open_dir.join(format!("h5p-{short}"))
}

/// Writes a package for the player: `h5p.json`, `content/…`, and the libraries it brings under
/// `libraries/`. The package is checked first like an opened `.h5p` file.
pub(crate) fn unpack_content(bytes: &[u8], dir: &Path) -> Result<usize> {
    super::h5p::inspect(bytes)?;
    let zerr = |e: craft_zip::Error| Error::invalid(format!("Das H5P-Paket ist nicht lesbar ({e})"));
    let mut zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: super::h5p::MAX_ENTRIES, max_total_bytes: super::h5p::MAX_UNPACKED_BYTES })
        .map_err(zerr)?;
    package::remove(dir);
    crate::disk::create_dir(dir)?;
    let mut files = 0;
    for i in 0..zip.len() {
        let name = zip.entry_name(i).map_err(zerr)?.unwrap_or_default();
        if zip.entry_is_dir(i).map_err(zerr)?.unwrap_or(false) || name.split('/').any(|p| p.starts_with('.') || p == "__MACOSX") {
            continue;
        }
        // Names passed `inspect`: relative, no `..`, no backslashes, known types.
        let target = if name == "h5p.json" || name.starts_with("content/") { dir.join(&name) } else { dir.join("libraries").join(&name) };
        let data = zip.read_index(i, super::h5p::MAX_PACKAGE_BYTES).map_err(zerr)?.unwrap_or_default();
        if let Some(parent) = target.parent() {
            crate::disk::create_dir(parent)?;
        }
        crate::disk::write(&target, &data)?;
        files += 1;
    }
    Ok(files)
}

async fn play(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (a, h, _) = super::h5p::h5p_of(&c, &id)?;
    if !installed(&c) {
        return Err(Error::not_found(format!(
            "Der H5P-Player ist nicht installiert. „H5P-Player laden“ lädt ihn (ca. {APPROX_MB} MB, {LICENSE}); bis dahin zeigt „Ausprobieren“ die Aufgaben."
        )));
    }
    let note = crate::ai_label::note_for_artifact(&c, &a).map(|n| n.plain());
    let (bytes, _) = super::h5p::package_bytes(&c, &h, note.as_deref())?;
    let dir = content_dir(&c, &a.id);
    let files = unpack_content(&bytes, &dir)?;
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keine Fenster (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    if super::preview_program(&c).is_none() {
        return Err(Error::unsupported(
            "Zu dieser Installation gehört keine Vorschau (aiscraft-preview), die den H5P-Player zeigen kann. „Ausprobieren“ zeigt die Aufgaben trotzdem.",
        ));
    }
    let player = c.0.h5p_player_dir.clone();
    let args = [std::ffi::OsStr::new("--h5p"), dir.as_os_str(), std::ffi::OsStr::new("--player"), player.as_os_str()];
    if !super::spawn_preview(&c, &args, &a.title) {
        return Err(Error::unsupported("Die Vorschau (aiscraft-preview) ließ sich nicht starten"));
    }
    Ok(json!({"shown_in": "preview", "dir": dir.display().to_string(), "files": files}))
}

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "h5p.player_status",
        label: "H5P-Player: Stand",
        group: "H5P",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, _p| Box::pin(async move { Ok(status(&c)) }),
    },
    CommandSpec {
        id: "h5p.player_install",
        label: "H5P-Player laden (GPL-3.0)",
        group: "H5P",
        shortcut: None,
        params: "yes: bool (ausdrückliche Zustimmung zum Download: Größe, Quelle und Lizenz wurden genannt)",
        palette: false,
        run: |c, p| Box::pin(install(c, p)),
    },
    CommandSpec {
        id: "h5p.player_remove",
        label: "H5P-Player entfernen",
        group: "H5P",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, _p| Box::pin(remove(c)),
    },
    CommandSpec {
        id: "h5p.play",
        label: "Im H5P-Player ansehen",
        group: "H5P",
        shortcut: None,
        params: "id: string",
        palette: false,
        run: |c, p| Box::pin(play(c, p)),
    },
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sums_files_are_read_like_sha256sum() {
        let h = "a".repeat(64);
        let sums = format!("{h}  {ASSET}\n{}  other.zip\n", "b".repeat(64));
        assert_eq!(sum_for(&sums, ASSET), Some(h.clone()));
        assert_eq!(sum_for(&format!("{}  *{ASSET}", h.to_uppercase()), ASSET), Some(h.clone()));
        assert_eq!(sum_for(&sums, "missing.zip"), None);
        assert_eq!(sum_for(&format!("abc  {ASSET}"), ASSET), None, "short hash");
        assert_eq!(sum_for(&format!("{}  {ASSET}", "g".repeat(64)), ASSET), None, "not hex");
        assert_eq!(sum_for(&format!("{h}  {ASSET}.evil"), ASSET), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
