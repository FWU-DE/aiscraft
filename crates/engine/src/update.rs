//! Updates: asks GitHub (`FWU-DE/aiscraft` releases, via craft-update) whether a newer published
//! release exists, keeps the answer in [`UpdateState`](crate::UpdateState), and after the
//! person's yes („Jetzt aktualisieren“, `app.update_now`) lets craft-update download this
//! computer's package, check it against the release's `SHA256SUMS.txt` (on Windows also the
//! signature), put it in place and start the restart helper.
//!
//! Background checks run at start and then at most once an hour, and stay silent on errors; the
//! manual check (`app.check_update`) says what went wrong. Nothing is downloaded or installed
//! without the yes. „Später“ (`app.update_later`) hides the hint for some hours, or until the
//! next start.
//!
//! Engines in memory (tests, the offscreen snapshot) never reach the network: their feed is off
//! unless a test hands them release JSON ([`crate::Engine::set_update_feed`]), and they never
//! install or restart; tests point the download at a local server and replace the restart
//! (`use_test_install`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use aiscraft_core::{Error, Result};
use craft_update::self_update::{self, Install};
use craft_update::{JobPhase, Paths, Platform, Release, Stop};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{Ctx, NoticeLevel, UpdateInfo, UpdatePhase, UpdateProgress};

/// Where releases are published. Only published releases count (CI makes drafts).
pub(crate) const REPO: &str = "FWU-DE/aiscraft";

/// The running version.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// At most one background request to the release list per hour.
const HOUR_MS: i64 = 3600 * 1000;

/// How often the background loop looks whether an hour has passed or „Später“ ran out. It asks
/// nothing by itself; [`check`] decides whether a request goes out.
const TICK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// „Später“ without hours, and its bounds (a week at most).
const LATER_HOURS: f64 = 4.0;
const LATER_MAX_HOURS: f64 = 168.0;

/// Release notes kept in the state (the full text is on the release page).
const MAX_NOTES: usize = 4000;

/// The composer text kept across the update's restart.
const MAX_DRAFT: usize = 100_000;

/// Store key of the composer text kept across the restart.
const DRAFT_KEY: &str = "update_draft";

/// The first part of every release package's name (`aischatcraft-0.2.0-windows-x64.msi`).
pub(crate) const ASSET_STEM: &str = "aischatcraft";

/// The desktop app's program (`aiscraft`, `aiscraft.exe`). Only it replaces itself; the CLI and
/// other programs that open an engine never do.
const GUI_PROGRAM: &str = "aiscraft";

/// What `last` answers in tests instead of letting craft-update install: the download is
/// checked, nothing is replaced and no helper starts.
const TEST_READY: &str = "aiscraft-test: bereit, kein Austausch";

/// AIS.ChatCraft as craft-update knows it; the self-update looks for packages named after
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

/// The update's moving parts that are not state: the release found, the running download, the
/// prepared restart. One per engine.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Requests to the release list so far (the gates count them).
    pub(crate) requests: AtomicUsize,
    /// Restarts that would have run (tests; the real app ends instead).
    pub(crate) restarts: AtomicUsize,
    /// The release the last check found, with its packages, for „Jetzt aktualisieren“.
    release: Mutex<Option<Release>>,
    /// The running download, so „Abbrechen“ can stop it.
    stop: Mutex<Option<Arc<Stop>>>,
    /// The checked update, waiting for the restart.
    restart: Mutex<Option<Pending>>,
    /// Tests: packages come from this local server and nothing is installed.
    test: Mutex<Option<TestInstall>>,
}

/// An update ready for the restart.
enum Pending {
    /// craft-update's restart helper, started when the window closes.
    Helper(self_update::Restart),
    /// Tests: the restart only counts.
    Test,
}

#[derive(Debug, Clone)]
struct TestInstall {
    /// `http://127.0.0.1:<port>`: each package of the release is fetched from `<mirror>/<name>`.
    mirror: String,
    /// craft-update's folders (downloads) for this test.
    dir: std::path::PathBuf,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Tests: fetch packages from `mirror` (a local server) into `dir`, as if this copy were an
/// AppImage on Linux x86-64, and never install or restart. Only for engines in memory.
#[cfg(test)]
pub(crate) fn use_test_install(c: &Ctx, mirror: &str, dir: std::path::PathBuf) -> Result<()> {
    if c.0.pictures_online {
        return Err(Error::refused("Nur ohne Datenordner (Tests, Vorschau) erlaubt"));
    }
    *lock(&c.0.update_job.test) = Some(TestInstall { mirror: mirror.trim_end_matches('/').to_string(), dir });
    c.update(|st| st.update.can_install = true);
    Ok(())
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

/// At start: show what the last check found, bring back a draft kept across an update's restart,
/// say how the last update ended, then check in the background now and every hour while the
/// setting is on.
pub(crate) fn start(c: &Ctx) {
    let s = saved(c);
    let available = offer(s.latest.clone(), s.skipped.as_deref());
    c.update(|st| {
        st.update.current = CURRENT_VERSION.into();
        st.update.last_check = s.last_check;
        st.update.skipped = s.skipped.clone();
        st.update.available = available;
    });
    restore_draft(c);
    if !matches!(feed(c), Feed::GitHub) {
        return;
    }
    let ctx = c.clone();
    c.spawn(async move {
        // Whether this copy can replace itself: craft-update knows how it was installed (and on
        // Windows checks the signature), so it runs off the window's thread.
        let probe = ctx.clone();
        let can = tokio::task::spawn_blocking(move || {
            report_last_update(&probe);
            this_copy_updates_itself()
        })
        .await
        .unwrap_or(false);
        ctx.update(|st| st.update.can_install = can);
        // Let the window come up first; the check is never urgent.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        loop {
            if let Err(e) = check(&ctx, true).await {
                log::debug!("Update-Prüfung: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Whether the running program is the desktop app in an installation craft-update can replace
/// (AppImage, Flatpak bundle, macOS app, …). Anything else gets the download link, as before.
fn this_copy_updates_itself() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    if exe.file_stem().and_then(|s| s.to_str()) != Some(GUI_PROGRAM) {
        return false;
    }
    // craft-update's Windows helper only finishes a program named after the release stem
    // (`aischatcraft.exe`); ours is `aiscraft.exe`, and the packages are not signed yet (#58).
    // Until both are settled, Windows gets the download link.
    if cfg!(windows) {
        return false;
    }
    self_update::is_self_updating(&APP)
}

/// After the restart into a new version: say how the update ended (craft-update's records).
fn report_last_update(c: &Ctx) {
    let Ok(paths) = Paths::for_app(&APP) else { return };
    match self_update::take_outcome(&paths, CURRENT_VERSION) {
        Some(self_update::Outcome::Installed(v)) => {
            c.notice(NoticeLevel::Info, format!("AIS.ChatCraft ist jetzt auf Version {v} aktualisiert."));
        }
        Some(self_update::Outcome::Failed(why)) => {
            c.notice(NoticeLevel::Warning, format!("Das letzte Update ließ sich nicht installieren: {}", install_german(&why)));
        }
        None => {}
    }
}

/// The composer text the update's restart kept, back into the composer (once).
fn restore_draft(c: &Ctx) {
    let draft = c.store(|st| st.setting(DRAFT_KEY)).ok().flatten();
    let Some(text) = draft.as_ref().and_then(Value::as_str).map(str::to_string) else { return };
    if c.store(|st| st.delete_setting(DRAFT_KEY)).is_ok() && !text.trim().is_empty() {
        crate::templates::fill_composer(c, text, true);
    }
}

/// „Später“ ran out: show the hint again, from what the last check found (no request).
fn reoffer(c: &Ctx, now: i64) {
    let remind_at = c.state().update.remind_at;
    if remind_at == 0 || now < remind_at {
        return;
    }
    let s = saved(c);
    c.update(|st| {
        st.update.remind_at = 0;
        let dismissed = st.update.dismissed.clone();
        st.update.available = offer(s.latest.clone(), s.skipped.as_deref()).filter(|a| dismissed.as_deref().is_none_or(|d| newer(&a.version, d)));
    });
}

fn feed(c: &Ctx) -> Feed {
    lock(&c.0.update_feed).clone()
}

/// The release list, as craft-update reads it: the newest published, stable release, if it is
/// newer than the running version. Counts the request.
async fn fetch(c: &Ctx, feed: Feed) -> std::result::Result<Option<Release>, String> {
    if !matches!(feed, Feed::Off) {
        c.0.update_job.requests.fetch_add(1, Ordering::SeqCst);
    }
    match feed {
        Feed::Off => Ok(None),
        Feed::Scripted(Err(e)) => Err(e),
        Feed::Scripted(Ok(json)) => {
            let latest = craft_update::parse_github_release_list(&json, REPO, false)?;
            Ok(latest.filter(|r| r.version.as_ref().is_some_and(|v| newer(&v.to_string(), CURRENT_VERSION))))
        }
        Feed::GitHub => tokio::task::spawn_blocking(|| {
            let http = craft_update::Http::new(&agent())?;
            let found = craft_update::check_for_update(&APP, &http, CURRENT_VERSION)?;
            // Just updated to it (a release whose program says another version): don't loop.
            let paths = Paths::for_app(&APP).ok();
            Ok(found.filter(|r| paths.as_ref().is_none_or(|p| !self_update::just_updated_to(p, r))))
        })
        .await
        .map_err(|e| format!("Prüfung abgebrochen ({e})"))?,
    }
}

fn agent() -> String {
    format!("AIS.ChatCraft/{CURRENT_VERSION}")
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

/// A craft-update install error in German, with the next step. craft-update reports errors as
/// English text without a kind, so this reads the text (see docs/craft-libs.md).
fn install_german(e: &str) -> String {
    let e = e.trim();
    if e == craft_update::protocol::CANCELLED {
        "Update abgebrochen. Es wurde nichts installiert.".into()
    } else if e == craft_update::protocol::PAUSED {
        "Die Verbindung ist abgebrochen. Versuch es später noch einmal; was schon geladen ist, bleibt erhalten.".into()
    } else if e.contains("doesn't match its published checksum") {
        "Die heruntergeladene Datei passt nicht zur veröffentlichten Prüfsumme (SHA256SUMS.txt). Sie wurde gelöscht, es wurde nichts installiert.".into()
    } else if e.contains("publishes no checksum") || e.contains("no valid checksum") {
        "Das Release nennt keine Prüfsumme für dieses Paket. Ohne Prüfsumme installiert AIS.ChatCraft nichts.".into()
    } else if e.contains("checksums") {
        "Die Prüfsummen des Releases ließen sich nicht laden. Versuch es später noch einmal.".into()
    } else if e.contains("isn't signed") || e.contains("signed by") {
        "Das Paket ist nicht vom Herausgeber von AIS.ChatCraft signiert und wurde nicht installiert.".into()
    } else if e.contains("download link isn't") || e.contains("does not match the advertised version") || e.contains("does not match selected asset") {
        "Der Download gehört nicht zu diesem Release von AIS.ChatCraft; es wurde nichts geladen.".into()
    } else if e.contains("can't update itself") || e.contains("only finishes") {
        "Diese Installation kann sich nicht selbst aktualisieren. Lade die neue Version über „Herunterladen“.".into()
    } else if e.contains(" has no ") {
        "Dieses Release enthält kein Paket für diesen Rechner. Lade die neue Version über „Herunterladen“.".into()
    } else if e.contains("not newer") {
        "Diese Version ist nicht neuer als die laufende.".into()
    } else if e.contains("no longer on the release page") {
        "Das Paket ist nicht mehr auf der Release-Seite. Prüf nach Updates und versuch es noch einmal.".into()
    } else if e.contains("Is the disk full") {
        format!("{} ({})", aiscraft_core::DISK_FULL, e.chars().take(200).collect::<String>())
    } else if e.contains("couldn't connect") || e.contains("couldn't reach") || e.contains("HTTP ") || e.contains("connection") {
        "Der Download-Server ist nicht erreichbar. Prüf die Internetverbindung und versuch es noch einmal.".into()
    } else {
        format!("Das Update ist fehlgeschlagen: {}", e.chars().take(300).collect::<String>())
    }
}

fn info(r: &Release) -> UpdateInfo {
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
fn platform_asset(r: &Release) -> Option<String> {
    let platform = craft_update::Platform::current()?;
    craft_update::pick_asset(r, &[ASSET_STEM], platform, platform.download_kinds()).map(|(a, _)| a.url.clone())
}

/// Check now. `background`: only when switched on and an hour after the last check, and errors
/// are not shown. Returns what the settings show.
pub(crate) async fn check(c: &Ctx, background: bool) -> Result<Value> {
    let feed = feed(c);
    if matches!(feed, Feed::Off) {
        return Err(Error::unsupported("In dieser Umgebung prüft AIS.ChatCraft nicht nach Updates (Tests, Skripte, AISCRAFT_NO_UPDATE_CHECK)"));
    }
    let now = crate::now_ms();
    let mut s = saved(c);
    if background {
        reoffer(c, now);
        let st = c.state();
        if !st.settings.update_check {
            return Ok(json!({"checked": false, "reason": "ausgeschaltet"}));
        }
        if now.saturating_sub(s.last_check) < HOUR_MS && s.last_check <= now {
            return Ok(json!({"checked": false, "reason": "in der letzten Stunde schon geprüft"}));
        }
    }
    let busy = c.update(|st| std::mem::replace(&mut st.update.checking, true));
    if busy {
        return Ok(json!({"checked": false, "reason": "läuft schon"}));
    }
    let result = fetch(c, feed).await;
    s.last_check = now;
    let outcome = match result {
        Ok(found) => {
            s.latest = found.as_ref().map(info);
            *lock(&c.0.update_job.release) = found;
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
                // A dismissed hint stays away for this run unless an even newer version appears;
                // „Später“ keeps it away until its time.
                if background {
                    let dismissed = st.update.dismissed.clone();
                    let later = st.update.remind_at > now;
                    st.update.available = available.clone().filter(|a| !later && dismissed.as_deref().is_none_or(|d| newer(&a.version, d)));
                } else {
                    st.update.available = available.clone();
                    st.update.dismissed = None;
                    st.update.remind_at = 0;
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

/// „Später“: no hint for `hours` (4 without; 1 to 168), or until the next start, whichever
/// comes first. Nothing is asked of GitHub for it; the hourly check goes on.
pub(crate) fn later(c: &Ctx, p: &Value) -> Result<Value> {
    let hours = match p.get("hours") {
        None | Some(Value::Null) => LATER_HOURS,
        Some(v) => v.as_f64().filter(|h| h.is_finite()).ok_or_else(|| Error::invalid("hours: eine Zahl von 1 bis 168"))?,
    }
    .clamp(1.0, LATER_MAX_HOURS);
    #[allow(clippy::cast_possible_truncation)]
    let until = crate::now_ms().saturating_add((hours * HOUR_MS as f64) as i64);
    let hidden = c.update(|st| {
        st.update.remind_at = until;
        st.update.available.take().is_some()
    });
    Ok(json!({"remind_at": until, "hours": hours, "hidden": hidden}))
}

/// What the update does now: the offered version, whether this copy can replace itself, the
/// download's progress and the last error.
pub(crate) fn status(c: &Ctx) -> Result<Value> {
    let u = c.state().update.clone();
    Ok(json!({
        "current": u.current,
        "available": u.available.as_ref().map(|a| a.version.clone()),
        "can_install": u.can_install,
        "progress": u.progress,
        "error": u.error,
        "remind_at": u.remind_at,
        "restart": u.restart,
        "last_check": u.last_check,
    }))
}

/// „Jetzt aktualisieren“: craft-update downloads this computer's package in the background,
/// checks it against the release's checksums, waits for running answers, puts it in place
/// (Windows: after the restart) and prepares the restart. The person's click is the yes; nothing
/// runs without it.
pub(crate) async fn now(c: &Ctx) -> Result<Value> {
    let st = c.state();
    if let Some(p) = &st.update.progress {
        let reason = if p.phase == UpdatePhase::Ready { "bereit zum Neustart" } else { "läuft schon" };
        return Ok(json!({"started": false, "reason": reason, "version": p.version}));
    }
    let test = lock(&c.0.update_job.test).clone();
    let feed = feed(c);
    if test.is_none() && !matches!(feed, Feed::GitHub) {
        return Err(Error::unsupported("In dieser Umgebung aktualisiert sich AIS.ChatCraft nicht selbst (Tests, Skripte, AISCRAFT_NO_UPDATE_CHECK)"));
    }
    if !st.update.can_install {
        return Err(Error::unsupported(
            "Diese Installation kann sich nicht selbst aktualisieren. Lade die neue Version über „Herunterladen“ (öffnet die Release-Seite).",
        ));
    }
    let s = saved(c);
    let target = offer(s.latest, s.skipped.as_deref()).ok_or_else(|| Error::not_found("Keine neue Version bekannt. Prüf zuerst nach Updates."))?;
    let known = lock(&c.0.update_job.release).clone().filter(|r| r.tag == target.tag);
    let release = match known {
        Some(r) => r,
        // After a restart only the hint is remembered: read the release with its packages again.
        None => fetch(c, feed)
            .await
            .map_err(|e| Error::network(german(&e)))?
            .filter(|r| r.tag == target.tag)
            .ok_or_else(|| Error::not_found("Diese Version ist nicht mehr verfügbar. Prüf nach Updates."))?,
    };
    let release = match &test {
        Some(t) => mirrored(release, &t.mirror),
        None => release,
    };
    let stop = Arc::new(Stop::default());
    *lock(&c.0.update_job.stop) = Some(stop.clone());
    let version = release.version_label();
    c.update(|st| {
        st.update.error = None;
        st.update.restart = false;
        st.update.progress = Some(UpdateProgress { version: version.clone(), phase: UpdatePhase::Download, done: 0, total: 0 });
    });
    let ctx = c.clone();
    c.spawn(async move {
        let job = ctx.clone();
        let result = tokio::task::spawn_blocking(move || install(&job, &release, test.as_ref(), &stop))
            .await
            .unwrap_or_else(|e| Err(format!("the update stopped ({e})")));
        *lock(&ctx.0.update_job.stop) = None;
        match result {
            Ok(pending) => {
                *lock(&ctx.0.update_job.restart) = Some(pending);
                ctx.update(|st| {
                    if let Some(p) = st.update.progress.as_mut() {
                        p.phase = UpdatePhase::Ready;
                    }
                });
            }
            Err(e) => {
                let cancelled = e.trim() == craft_update::protocol::CANCELLED;
                let text = install_german(&e);
                log::warn!("Update: {e}");
                ctx.update(|st| {
                    st.update.progress = None;
                    st.update.error = (!cancelled).then(|| text.clone());
                    if cancelled {
                        st.update.message = Some(text.clone());
                    }
                });
            }
        }
    });
    Ok(json!({"started": true, "version": version}))
}

/// Tests: every package and the checksum list come from the local server.
fn mirrored(mut r: Release, mirror: &str) -> Release {
    for a in &mut r.assets {
        a.url = format!("{mirror}/{}", a.name);
    }
    r
}

/// The blocking part of „Jetzt aktualisieren“: craft-update's `prepare_update`, with progress
/// into the state and the wait for running answers as its last step before the swap.
fn install(c: &Ctx, release: &Release, test: Option<&TestInstall>, stop: &Stop) -> std::result::Result<Pending, String> {
    let http = craft_update::Http::new(&agent())?;
    let (install, platform, paths) = match test {
        Some(t) => (
            Install::AppImage(t.dir.join(format!("{ASSET_STEM}.AppImage"))),
            Platform { os: craft_update::Os::Linux, arch: craft_update::Arch::X64 },
            Paths::under(&t.dir),
        ),
        None => (
            self_update::current(&APP).ok_or("this copy can't update itself")?,
            Platform::current().ok_or("this copy can't update itself")?,
            Paths::for_app(&APP)?,
        ),
    };
    let mut shown: Option<(UpdatePhase, u64)> = None;
    let mut phase = |p: JobPhase| {
        let (phase, done, total) = match p {
            JobPhase::Downloading { done, total } | JobPhase::Reconnecting { done, total } => (UpdatePhase::Download, done, total),
            JobPhase::Verifying => (UpdatePhase::Verify, 0, 0),
            JobPhase::Waiting | JobPhase::Installing | JobPhase::Removing => (UpdatePhase::Install, 0, 0),
        };
        // Whole percents only: a state change redraws the window.
        let step = done.saturating_mul(100).checked_div(total).unwrap_or(0);
        if shown == Some((phase, step)) {
            return;
        }
        shown = Some((phase, step));
        set_phase(c, phase, done, total);
    };
    let last = || -> std::result::Result<(), String> {
        wait_for_answers(c, stop)?;
        if test.is_some() { Err(TEST_READY.into()) } else { Ok(()) }
    };
    match self_update::prepare_update(&APP, &install, release, platform, &http, &paths, stop, &mut phase, &last, Some(CURRENT_VERSION)) {
        Ok(restart) => Ok(Pending::Helper(restart)),
        Err(e) if test.is_some() && e == TEST_READY => Ok(Pending::Test),
        Err(e) => Err(e),
    }
}

fn set_phase(c: &Ctx, phase: UpdatePhase, done: u64, total: u64) {
    c.update(|st| {
        if let Some(p) = st.update.progress.as_mut() {
            p.phase = phase;
            // Later steps keep what was downloaded.
            if phase == UpdatePhase::Download {
                p.done = done;
                p.total = total;
            }
        }
    });
}

/// Before the swap: answers being written are finished first (or stopped by the person). Chats
/// and messages are in the database as they arrive; the composer's text goes with the restart
/// (`app.update_restart`).
fn wait_for_answers(c: &Ctx, stop: &Stop) -> std::result::Result<(), String> {
    let mut waiting = false;
    loop {
        stop.check()?;
        if c.state().generating.is_empty() {
            return Ok(());
        }
        if !waiting {
            waiting = true;
            set_phase(c, UpdatePhase::WaitAnswer, 0, 0);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// „Abbrechen“: stops the download (craft-update removes what it fetched), or drops an update
/// that waits for the restart.
pub(crate) fn cancel(c: &Ctx) -> Result<Value> {
    if let Some(stop) = lock(&c.0.update_job.stop).as_ref() {
        stop.cancel();
        return Ok(json!({"cancelled": true}));
    }
    if lock(&c.0.update_job.restart).take().is_some() {
        c.update(|st| {
            st.update.progress = None;
            st.update.message = Some(if cfg!(windows) {
                "Neustart verschoben. Das Update wurde nicht installiert.".into()
            } else {
                "Neustart verschoben. Die neue Version startet beim nächsten Öffnen.".into()
            });
        });
        return Ok(json!({"cancelled": true}));
    }
    Ok(json!({"cancelled": false, "reason": "Es läuft kein Update"}))
}

/// The restart into the new version: keeps the composer's text for the next start, starts
/// craft-update's helper and asks the window to close. Refused while an answer is written.
pub(crate) fn restart(c: &Ctx, p: &Value) -> Result<Value> {
    if !c.state().generating.is_empty() {
        return Err(Error::busy("Eine Antwort wird noch geschrieben. Warte, bis sie fertig ist, oder halte sie an."));
    }
    let pending = lock(&c.0.update_job.restart).take().ok_or_else(|| Error::not_found("Kein Update wartet auf den Neustart"))?;
    let draft: String = p.get("draft").and_then(Value::as_str).unwrap_or_default().chars().take(MAX_DRAFT).collect();
    let keep = !draft.trim().is_empty();
    if keep && let Err(e) = c.store(|st| st.set_setting(DRAFT_KEY, &json!(draft))) {
        // The text stays in the window: no restart that would lose it.
        *lock(&c.0.update_job.restart) = Some(pending);
        return Err(e);
    }
    match pending {
        Pending::Helper(r) => {
            if let Err(e) = r.start() {
                let text = install_german(&e);
                c.update(|st| {
                    st.update.progress = None;
                    st.update.error = Some(text.clone());
                });
                return Err(Error::unsupported(text));
            }
        }
        Pending::Test => {
            c.0.update_job.restarts.fetch_add(1, Ordering::SeqCst);
        }
    }
    c.update(|st| st.update.restart = true);
    Ok(json!({"restarting": true, "draft_kept": keep}))
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

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "app.update_now",
        label: "Jetzt aktualisieren",
        group: "App",
        shortcut: None,
        params: "",
        palette: true,
        run: |c, _| Box::pin(async move { now(&c).await }),
    },
    CommandSpec {
        id: "app.update_later",
        label: "Update später",
        group: "App",
        shortcut: None,
        params: "hours?: number (ohne: 4; 1 bis 168, längstens bis zum nächsten Start)",
        palette: false,
        run: |c, p| Box::pin(async move { later(&c, &p) }),
    },
    CommandSpec {
        id: "app.update_status",
        label: "Update-Stand lesen",
        group: "App",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, _| Box::pin(async move { status(&c) }),
    },
    CommandSpec {
        id: "app.update_cancel",
        label: "Update abbrechen",
        group: "App",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, _| Box::pin(async move { cancel(&c) }),
    },
    CommandSpec {
        id: "app.update_restart",
        label: "Neu starten mit der neuen Version",
        group: "App",
        shortcut: None,
        params: "draft?: string (Text im Eingabefeld; kommt nach dem Neustart zurück)",
        palette: false,
        run: |c, p| Box::pin(async move { restart(&c, &p) }),
    },
];

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
        crate::disk::write(&dir.as_path().join("portable.txt"), b"AIS.ChatCraft portable.").unwrap();
        assert_eq!(crate::portable_data_dir(&exe), Some(dir.as_path().join("AIS.ChatCraftData").join("aiscraft")));
        let paths = craft_update::Paths::for_exe(&APP, &exe).unwrap();
        assert!(paths.data.starts_with(dir.as_path().join("AIS.ChatCraftData")), "{:?}", paths.data);
        assert_eq!(crate::portable_data_dir(std::path::Path::new("")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
