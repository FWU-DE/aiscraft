//! Update check: asks GitHub (`FWU-DE/aiscraft` releases, via craft-update) whether a newer
//! published release exists and keeps the answer in [`UpdateState`]. One anonymous request; no
//! download, no install. Background checks run at start, at most once a day, and stay silent on
//! errors; the manual check (`app.check_update`) says what went wrong.
//!
//! Engines in memory (tests, the offscreen snapshot) never reach the network: their feed is off
//! unless a test hands them release JSON ([`crate::Engine::set_update_feed`]).

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Ctx, UpdateInfo};

/// Where releases are published. Only published releases count (CI makes drafts).
pub(crate) const REPO: &str = "FWU-DE/aiscraft";

/// The running version.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// At most one background check per day.
const DAY_MS: i64 = 24 * 3600 * 1000;

/// Release notes kept in the state (the full text is on the release page).
const MAX_NOTES: usize = 4000;

/// The first part of every release package's name (`aischatcraft-0.2.0-windows-x64.msi`).
pub(crate) const ASSET_STEM: &str = "aischatcraft";

/// AIS.ChatCraft as craft-update knows it; a later self-update looks for packages named after
/// [`ASSET_STEM`].
static APP: craft_update::App = craft_update::App {
    developer: "FWU",
    bundle_id_override: Some("ai.uds.aischatcraft"),
    ..craft_update::App::new(ASSET_STEM, "AIS.ChatCraft", "Native client for AIS.chat", REPO)
};

/// When this program was started as craft-update's restart helper (Windows: it waits for the
/// old copy to exit, runs the new MSI or unpacks the portable zip, then starts the app), does that
/// and returns its result; `None` for a normal start. `args` without the program name. Called first
/// thing in `main`; elsewhere and on other systems it is always `None`.
pub fn finish_update_helper(args: &[std::ffi::OsString]) -> Option<std::result::Result<(), String>> {
    craft_update::self_update::finish(&APP, args)
}

/// Where an engine gets its release list from.
#[derive(Debug, Clone, Default)]
pub(crate) enum Feed {
    /// No checks (engines in memory, `AISCRAFT_NO_UPDATE_CHECK`).
    #[default]
    Off,
    /// `https://api.github.com/repos/FWU-DE/aiscraft/releases` (the real app).
    GitHub,
    /// A release list (JSON as GitHub sends it) or the error a check should report (tests, demo).
    Scripted(std::result::Result<String, String>),
}

/// What is remembered between starts (store key `update`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    /// Last finished check (ms), successful or not.
    last_check: i64,
    /// The newest published release the last successful check found, if newer than the version
    /// that ran then.
    latest: Option<UpdateInfo>,
    /// "Diese Version überspringen": no hint for this version (or older).
    skipped: Option<String>,
}

fn saved(c: &Ctx) -> Saved {
    c.store(|st| Ok(st.setting("update")?.and_then(|v| serde_json::from_value(v).ok()))).ok().flatten().unwrap_or_default()
}

fn save(c: &Ctx, s: &Saved) -> Result<()> {
    c.store(|st| st.set_setting("update", &serde_json::to_value(s)?))
}

fn newer(version: &str, than: &str) -> bool {
    match (craft_update::parse_version(version), craft_update::parse_version(than)) {
        (Some(v), Some(t)) => craft_update::version::offers_update(&v, &t),
        _ => false,
    }
}

/// The hint to show for `latest`: newer than what runs and not skipped.
fn offer(latest: Option<UpdateInfo>, skipped: Option<&str>) -> Option<UpdateInfo> {
    latest.filter(|l| newer(&l.version, CURRENT_VERSION) && skipped.is_none_or(|s| newer(&l.version, s)))
}

/// At start: show what the last check found, then check again in the background when the
/// setting is on and the last check is a day old.
pub(crate) fn start(c: &Ctx) {
    let s = saved(c);
    let available = offer(s.latest.clone(), s.skipped.as_deref());
    c.update(|st| {
        st.update.current = CURRENT_VERSION.into();
        st.update.last_check = s.last_check;
        st.update.skipped = s.skipped.clone();
        st.update.available = available;
    });
    if matches!(feed(c), Feed::Off) {
        return;
    }
    let ctx = c.clone();
    c.spawn(async move {
        // Let the window come up first; the check is never urgent.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if let Err(e) = check(&ctx, true).await {
            log::debug!("Update-Prüfung: {e}");
        }
    });
}

fn feed(c: &Ctx) -> Feed {
    c.0.update_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The release list, as craft-update reads it: the newest published, stable release, if it is
/// newer than the running version.
async fn fetch(feed: Feed) -> std::result::Result<Option<craft_update::Release>, String> {
    match feed {
        Feed::Off => Ok(None),
        Feed::Scripted(Err(e)) => Err(e),
        Feed::Scripted(Ok(json)) => {
            let latest = craft_update::parse_github_release_list(&json, REPO, false)?;
            Ok(latest.filter(|r| r.version.as_ref().is_some_and(|v| newer(&v.to_string(), CURRENT_VERSION))))
        }
        Feed::GitHub => tokio::task::spawn_blocking(|| {
            let http = craft_update::Http::new(&format!("AIS.ChatCraft/{CURRENT_VERSION}"))?;
            craft_update::check_for_update(&APP, &http, CURRENT_VERSION)
        })
        .await
        .map_err(|e| format!("Prüfung abgebrochen ({e})"))?,
    }
}

/// A craft-update error in German, short enough for a line in the settings.
fn german(e: &str) -> String {
    if e.contains("request limit") || e.contains("refused") {
        "GitHub nimmt gerade keine Anfragen an (Anfragelimit). Versuch es später noch einmal.".into()
    } else if e.contains("couldn't reach") || e.contains("Prüfung abgebrochen") {
        "GitHub ist nicht erreichbar. Prüf die Internetverbindung und versuch es später noch einmal.".into()
    } else if e.contains("unreadable") || e.contains("unexpected") || e.contains("GitHub") {
        "GitHub hat keine lesbare Antwort geschickt. Versuch es später noch einmal.".into()
    } else {
        e.to_string()
    }
}

fn info(r: &craft_update::Release) -> UpdateInfo {
    let mut notes: String = r.notes.chars().take(MAX_NOTES).collect();
    if r.notes.chars().count() > MAX_NOTES {
        notes.push_str(" …");
    }
    UpdateInfo {
        version: r.version_label(),
        tag: r.tag.clone(),
        name: r.name.clone(),
        url: r.html_url.clone(),
        download: platform_asset(r).unwrap_or_else(|| r.html_url.clone()),
        published_at: r.published_at.map(|s| s.saturating_mul(1000)),
        notes,
    }
}

/// This computer's package in the release, named after craft-update's scheme
/// (`aischatcraft-<version>-<os>-<arch>.<ext>`, see packaging/): on Windows the MSI, else the
/// portable zip; on macOS the DMG; on Linux the AppImage, else deb, rpm, Flatpak or tarball.
/// craft-update only keeps assets whose link is the repository's own release download.
fn platform_asset(r: &craft_update::Release) -> Option<String> {
    let platform = craft_update::Platform::current()?;
    craft_update::pick_asset(r, &[ASSET_STEM], platform, platform.download_kinds()).map(|(a, _)| a.url.clone())
}

/// Check now. `background`: only when switched on and a day after the last check, and errors
/// are not shown. Returns what the settings show.
pub(crate) async fn check(c: &Ctx, background: bool) -> Result<Value> {
    let feed = feed(c);
    if matches!(feed, Feed::Off) {
        return Err(Error::unsupported("In dieser Umgebung prüft AIS.ChatCraft nicht nach Updates (Tests, Skripte, AISCRAFT_NO_UPDATE_CHECK)"));
    }
    let now = crate::now_ms();
    let mut s = saved(c);
    if background {
        let st = c.state();
        if !st.settings.update_check {
            return Ok(json!({"checked": false, "reason": "ausgeschaltet"}));
        }
        if now.saturating_sub(s.last_check) < DAY_MS && s.last_check <= now {
            return Ok(json!({"checked": false, "reason": "heute schon geprüft"}));
        }
    }
    let busy = c.update(|st| std::mem::replace(&mut st.update.checking, true));
    if busy {
        return Ok(json!({"checked": false, "reason": "läuft schon"}));
    }
    let result = fetch(feed).await;
    s.last_check = now;
    let outcome = match result {
        Ok(found) => {
            s.latest = found.as_ref().map(info);
            let available = offer(s.latest.clone(), s.skipped.as_deref());
            let message = match (&available, &s.latest) {
                (Some(a), _) => format!("Version {} ist verfügbar.", a.version),
                (None, Some(l)) => format!("Version {} ist verfügbar, du hast sie übersprungen.", l.version),
                (None, None) => format!("Du hast die neueste Version ({CURRENT_VERSION})."),
            };
            Ok((available, message))
        }
        Err(e) => Err(german(&e)),
    };
    let saved_ok = save(c, &s);
    c.update(|st| {
        st.update.checking = false;
        st.update.last_check = now;
        match &outcome {
            Ok((available, message)) => {
                // A dismissed hint stays away for this run unless an even newer version appears.
                if background {
                    let dismissed = st.update.dismissed.clone();
                    st.update.available = available.clone().filter(|a| dismissed.as_deref().is_none_or(|d| newer(&a.version, d)));
                } else {
                    st.update.available = available.clone();
                    st.update.dismissed = None;
                }
                st.update.message = (!background).then(|| message.clone());
            }
            Err(e) => {
                if !background {
                    st.update.message = Some(e.clone());
                }
            }
        }
    });
    saved_ok?;
    match outcome {
        Ok((available, message)) => Ok(json!({
            "checked": true,
            "current": CURRENT_VERSION,
            "available": available.is_some(),
            "version": available.as_ref().map(|a| a.version.clone()),
            "message": message,
        })),
        Err(e) if background => Ok(json!({"checked": false, "reason": e})),
        Err(e) => Err(Error::network(e)),
    }
}

/// "Diese Version überspringen".
pub(crate) fn skip(c: &Ctx, p: &Value) -> Result<Value> {
    let version = p
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| c.state().update.available.as_ref().map(|a| a.version.clone()))
        .ok_or_else(|| Error::invalid("Keine neue Version zum Überspringen"))?;
    if craft_update::parse_version(&version).is_none() {
        return Err(Error::invalid(format!("Keine Versionsnummer: {}", version.chars().take(40).collect::<String>())));
    }
    let mut s = saved(c);
    s.skipped = Some(version.clone());
    save(c, &s)?;
    c.update(|st| {
        st.update.skipped = Some(version.clone());
        st.update.available = offer(st.update.available.take(), Some(&version));
    });
    Ok(json!({"skipped": version}))
}

/// Hide the hint until the next start.
pub(crate) fn dismiss(c: &Ctx) -> Result<Value> {
    let hidden = c.update(|st| {
        let a = st.update.available.take();
        st.update.dismissed = a.as_ref().map(|a| a.version.clone());
        a.is_some()
    });
    Ok(json!({"dismissed": hidden}))
}

/// Open the release page ("Was ist neu") or this computer's download in the browser.
pub(crate) fn open(c: &Ctx, p: &Value) -> Result<Value> {
    let st = c.state();
    let latest = st.update.available.clone().or_else(|| saved(c).latest).ok_or_else(|| Error::not_found("Keine neue Version bekannt"))?;
    let url = match p.get("what").and_then(Value::as_str).unwrap_or("notes") {
        "notes" => latest.url,
        "download" => latest.download,
        other => return Err(Error::invalid(format!("what: notes oder download, nicht {}", other.chars().take(40).collect::<String>()))),
    };
    let prefix = format!("https://github.com/{REPO}/");
    if !url.starts_with(&prefix) || !craft_update::release::safe_web_link(&url) {
        return Err(Error::refused("Dieser Link führt nicht zu den Releases von AIS.ChatCraft"));
    }
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keinen Browser (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    webbrowser::open(&url).map_err(|e| Error::unsupported(format!("Browser ließ sich nicht öffnen: {e}")))?;
    Ok(json!({"url": url}))
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn versions_compare_like_releases() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
        assert!(!newer("kaputt", "0.1.0"));
        let i = |v: &str| UpdateInfo { version: v.into(), ..Default::default() };
        assert!(offer(Some(i("99.0.0")), None).is_some());
        assert!(offer(Some(i("99.0.0")), Some("99.0.0")).is_none(), "skipped");
        assert!(offer(Some(i("99.1.0")), Some("99.0.0")).is_some(), "newer than the skipped one");
        assert!(offer(Some(i("0.0.1")), None).is_none(), "older than this build");
    }

    #[test]
    fn packages_are_found_by_craft_update_names_only() {
        let release = |names: &[&str]| craft_update::Release {
            assets: names.iter().map(|n| craft_update::Asset { name: (*n).into(), size: 1, url: format!("https://x/{n}"), sha256: None }).collect(),
            ..Default::default()
        };
        // The names before #58 carry no version: no package, the hint links the release page.
        assert_eq!(platform_asset(&release(&["aischatcraft-linux-x86_64.tar.gz", "aischatcraft-windows-x86_64.zip", "aischatcraft-macos-arm64.tar.gz"])), None);
        let all = release(&[
            "aischatcraft-0.2.0-linux-x86_64.tar.gz",
            "aischatcraft-0.2.0-linux-x86_64.AppImage",
            "aischatcraft-0.2.0-linux-aarch64.AppImage",
            "aischatcraft-0.2.0-macos-universal.dmg",
            "aischatcraft-cli-0.2.0-macos-universal.zip",
            "aischatcraft-0.2.0-windows-x64-portable.zip",
            "aischatcraft-0.2.0-windows-x64.msi",
            "SHA256SUMS.txt",
        ]);
        let found = platform_asset(&all).unwrap_or_default();
        if cfg!(windows) {
            assert!(found.ends_with("windows-x64.msi"), "{found}");
        } else if cfg!(target_os = "macos") {
            assert!(found.ends_with("macos-universal.dmg"), "{found}");
        } else if cfg!(target_arch = "x86_64") {
            assert!(found.ends_with("linux-x86_64.AppImage"), "{found}");
        }
    }

    #[test]
    fn errors_read_as_german() {
        assert!(german("GitHub's request limit was reached").contains("Anfragelimit"));
        assert!(german("couldn't reach GitHub (dns)").contains("nicht erreichbar"));
    }

    /// The portable zip (#58): `portable.txt` beside the program keeps the data beside it too,
    /// in the folder craft-update uses for its own files.
    #[test]
    fn portable_txt_keeps_the_data_next_to_the_program() {
        let dir = std::env::temp_dir().join(format!("aiscraft-portable-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.as_path().join("aiscraft.exe");
        assert_eq!(crate::portable_data_dir(&exe), None);
        std::fs::write(dir.as_path().join("portable.txt"), "AIS.ChatCraft portable.").unwrap();
        assert_eq!(crate::portable_data_dir(&exe), Some(dir.as_path().join("AIS.ChatCraftData").join("aiscraft")));
        let paths = craft_update::Paths::for_exe(&APP, &exe).unwrap();
        assert!(paths.data.starts_with(dir.as_path().join("AIS.ChatCraftData")), "{:?}", paths.data);
        assert_eq!(crate::portable_data_dir(std::path::Path::new("")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
