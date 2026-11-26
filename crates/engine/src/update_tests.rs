//! The self-update (#125): nothing is downloaded without the yes, a wrong checksum keeps
//! nothing, „Später“ waits for its time, the hourly check asks the release list at most once an
//! hour. The release list is scripted, the packages come from a local server through
//! craft-update's own download and checksum check, and the restart only counts.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::Digest;

use crate::{Engine, Options, UpdatePhase};

const PACKAGE: &str = "aischatcraft-99.0.0-linux-x86_64.AppImage";

/// A local server for the release's files; records each path asked for. `slow`: the package
/// comes in two halves with a pause between them (to cancel in the middle).
struct Mirror {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Mirror {
    fn start(files: Vec<(String, Vec<u8>)>, slow: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(Mutex::new(Vec::new()));
        let seen = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let files = files.clone();
                let seen = seen.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    let _ = reader.read_line(&mut first);
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                        line.clear();
                    }
                    let path = first.split_whitespace().nth(1).unwrap_or("").trim_start_matches('/').to_string();
                    seen.lock().unwrap().push(path.clone());
                    let Some((_, body)) = files.iter().find(|(n, _)| *n == path) else {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                        return;
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                    if slow && path == PACKAGE {
                        let half = body.len() / 2;
                        let _ = stream.write_all(&body[..half]);
                        let _ = stream.flush();
                        std::thread::sleep(Duration::from_millis(1500));
                        let _ = stream.write_all(&body[half..]);
                    } else {
                        let _ = stream.write_all(body);
                    }
                });
            }
        });
        Self { base, hits }
    }

    fn asked(&self, path: &str) -> usize {
        self.hits.lock().unwrap().iter().filter(|p| *p == path).count()
    }

    fn total(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

fn hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The release list as GitHub sends it, with the package's real size.
fn release_list(size: usize) -> String {
    let asset = |name: &str, size: usize| json!({"name": name, "size": size, "browser_download_url": format!("https://github.com/FWU-DE/aiscraft/releases/download/v99.0.0/{name}")});
    json!([{"tag_name": "v99.0.0", "name": "AIS.ChatCraft 99.0.0", "draft": false, "prerelease": false,
        "html_url": "https://github.com/FWU-DE/aiscraft/releases/tag/v99.0.0", "body": "## Neu\n\n- Selbst-Update",
        "assets": [asset(PACKAGE, size), asset("aischatcraft-99.0.0-windows-x64.msi", 10), asset("SHA256SUMS.txt", 100)]}])
    .to_string()
}

struct Setup {
    engine: Engine,
    mirror: Mirror,
    dir: std::path::PathBuf,
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An engine in memory with a scripted release list and the packages on a local server.
/// `published`: the checksum `SHA256SUMS.txt` names for the package.
fn setup(package: Vec<u8>, published: Option<String>, slow: bool) -> Setup {
    let sums = format!("{}  {PACKAGE}\n", published.unwrap_or_else(|| hex(&package)));
    let size = package.len();
    let mirror = Mirror::start(vec![(PACKAGE.to_string(), package), ("SHA256SUMS.txt".to_string(), sums.into_bytes())], slow);
    let dir = std::env::temp_dir().join(format!("aiscraft-selfupdate-{}", crate::new_id()));
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.set_update_feed(Ok(release_list(size))).unwrap();
    crate::update::use_test_install(engine.ctx(), &mirror.base, dir.clone()).unwrap();
    Setup { engine, mirror, dir }
}

fn wait_for(engine: &Engine, what: &str, done: impl Fn(&crate::UpdateState) -> bool) {
    for _ in 0..1500 {
        if done(&engine.state().update) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{what}: {:?}", engine.state().update);
}

fn downloads(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir.join("cache").join("downloads"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

/// Gate: the check and the hint download nothing; only „Jetzt aktualisieren“ fetches the
/// package (once) and its checksums, waits for a running answer, and the restart keeps the
/// composer's text for the next start.
#[test]
fn nothing_is_downloaded_before_the_yes_and_the_restart_keeps_the_draft() {
    let package = b"AppImage 99".repeat(2000);
    let s = setup(package.clone(), None, false);
    let e = &s.engine;
    // No yes yet: checking, reading the status, „Später“ and skipping download nothing.
    assert_eq!(e.call("app.check_update", json!({})).unwrap()["available"], true);
    let status = e.call("app.update_status", json!({})).unwrap();
    assert_eq!(status["available"], "99.0.0", "{status}");
    assert_eq!(status["can_install"], true);
    assert!(status["progress"].is_null());
    assert_eq!(e.call("app.update_later", json!({"hours": 4})).unwrap()["hidden"], true);
    assert_eq!(s.mirror.total(), 0, "nothing fetched without the yes");
    assert!(downloads(&s.dir).is_empty());

    // An answer is being written: the update downloads, then waits for it.
    e.ctx().update(|st| st.generating.push("chat-1".into()));
    assert_eq!(e.call("app.update_now", json!({})).unwrap()["started"], true);
    assert_eq!(e.call("app.update_now", json!({})).unwrap()["started"], false, "one update at a time");
    wait_for(e, "waits for the answer", |u| u.progress.as_ref().is_some_and(|p| p.phase == UpdatePhase::WaitAnswer));
    assert_eq!(s.mirror.asked(PACKAGE), 1);
    assert_eq!(s.mirror.asked("SHA256SUMS.txt"), 1);
    e.ctx().update(|st| st.generating.clear());
    wait_for(e, "ready", |u| u.progress.as_ref().is_some_and(|p| p.phase == UpdatePhase::Ready));
    let status = e.call("app.update_status", json!({})).unwrap();
    assert_eq!(status["progress"]["phase"], "ready", "{status}");
    assert_eq!(status["progress"]["done"].as_u64(), Some(package.len() as u64));
    assert!(status["error"].is_null());
    assert_eq!(downloads(&s.dir), vec![PACKAGE.to_string()], "the checked package");
    assert_eq!(s.mirror.asked(PACKAGE), 1, "downloaded once");

    // The restart: not while an answer is written; then the draft is kept and the window closes.
    e.ctx().update(|st| st.generating.push("chat-1".into()));
    let busy = e.call("app.update_restart", json!({"draft": "Mein Entwurf"})).unwrap_err();
    assert_eq!(busy.kind, aiscraft_core::ErrorKind::Busy, "{busy}");
    e.ctx().update(|st| st.generating.clear());
    let r = e.call("app.update_restart", json!({"draft": "Mein Entwurf"})).unwrap();
    assert_eq!(r["draft_kept"], true, "{r}");
    assert_eq!(e.ctx().0.update_job.restarts.load(Ordering::SeqCst), 1);
    assert!(e.state().update.restart);
    assert!(e.call("app.update_restart", json!({})).is_err(), "nothing left to restart");
    // The next start puts the draft back into the composer, once.
    crate::update::start(e.ctx());
    assert_eq!(e.state().composer_fill.as_ref().map(|f| f.text.as_str()), Some("Mein Entwurf"));
    assert!(e.ctx().store(|st| st.setting("update_draft")).unwrap().is_none());
}

/// Gate: a package that does not match `SHA256SUMS.txt` is deleted, nothing is installed or
/// restarted, and the person reads why in German.
#[test]
fn a_wrong_checksum_keeps_nothing_and_says_so_in_german() {
    let s = setup(b"manipuliert".repeat(500), Some("ab".repeat(32)), false);
    let e = &s.engine;
    e.call("app.check_update", json!({})).unwrap();
    assert_eq!(e.call("app.update_now", json!({})).unwrap()["started"], true);
    wait_for(e, "failed", |u| u.progress.is_none() && u.error.is_some());
    let error = e.state().update.error.clone().unwrap_or_default();
    assert!(error.contains("Prüfsumme") && error.contains("nichts installiert"), "{error}");
    assert!(downloads(&s.dir).is_empty(), "nothing kept: {:?}", downloads(&s.dir));
    assert!(s.mirror.asked(PACKAGE) >= 1);
    assert_eq!(e.ctx().0.update_job.restarts.load(Ordering::SeqCst), 0);
    assert!(e.call("app.update_restart", json!({})).is_err());
    assert_eq!(e.call("app.update_status", json!({})).unwrap()["error"], Value::String(error));
}

/// Gate: the background check asks the release list at most once an hour; „Später“ keeps the
/// hint away until its time, even when a check finds the version again.
#[test]
fn later_waits_for_its_time_and_the_hourly_check_asks_once_an_hour() {
    let s = setup(b"x".to_vec(), None, false);
    let e = &s.engine;
    let requests = || e.ctx().0.update_job.requests.load(Ordering::SeqCst);
    let backdate = |ms: i64| {
        e.ctx()
            .store(|st| {
                let mut v = st.setting("update")?.unwrap_or(json!({}));
                v["last_check"] = json!(crate::now_ms() - ms);
                st.set_setting("update", &v)
            })
            .unwrap();
    };
    assert_eq!(e.call("app.check_update", json!({"background": true})).unwrap()["checked"], true);
    assert_eq!(requests(), 1);
    for _ in 0..5 {
        let r = e.call("app.check_update", json!({"background": true})).unwrap();
        assert_eq!(r["checked"], false, "{r}");
    }
    assert_eq!(requests(), 1, "once an hour");
    backdate(59 * 60 * 1000);
    e.call("app.check_update", json!({"background": true})).unwrap();
    assert_eq!(requests(), 1, "not before the hour is over");
    backdate(61 * 60 * 1000);
    assert_eq!(e.call("app.check_update", json!({"background": true})).unwrap()["checked"], true);
    assert_eq!(requests(), 2, "again after an hour");
    assert!(e.state().update.available.is_some());

    // „Später“: hidden, and a background check after the hour finds it but keeps it hidden.
    let r = e.call("app.update_later", json!({})).unwrap();
    assert_eq!(r["hours"], 4.0, "{r}");
    assert!(e.state().update.available.is_none());
    backdate(2 * 60 * 60 * 1000);
    assert_eq!(e.call("app.check_update", json!({"background": true})).unwrap()["checked"], true);
    assert_eq!(requests(), 3);
    assert!(e.state().update.available.is_none(), "not before its time");
    // Its time has come: the hint is back without another request.
    e.ctx().update(|st| st.update.remind_at = crate::now_ms() - 1);
    e.call("app.check_update", json!({"background": true})).unwrap();
    assert_eq!(requests(), 3);
    assert_eq!(e.state().update.available.as_ref().map(|a| a.version.as_str()), Some("99.0.0"));
    assert_eq!(e.state().update.remind_at, 0);

    // Hours are bounded; nonsense is refused.
    assert_eq!(e.call("app.update_later", json!({"hours": 1e9})).unwrap()["hours"], 168.0);
    assert_eq!(e.call("app.update_later", json!({"hours": 0})).unwrap()["hours"], 1.0);
    assert!(e.call("app.update_later", json!({"hours": "bald"})).is_err());
    // A manual check brings the hint back at once.
    e.call("app.check_update", json!({})).unwrap();
    assert!(e.state().update.available.is_some());
    assert_eq!(e.state().update.remind_at, 0);
    assert_eq!(s.mirror.total(), 0, "checks never download");
}

/// „Abbrechen“ stops a running download (craft-update removes the partial file) and drops an
/// update waiting for the restart; without a way to replace itself the app refuses and the
/// download link stays.
#[test]
fn update_cancel_stops_the_download_and_update_now_needs_an_installation() {
    // Engines in memory without the test hook never update.
    let plain = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    assert_eq!(plain.call("app.update_now", json!({})).unwrap_err().kind, aiscraft_core::ErrorKind::Unsupported);
    assert_eq!(plain.call("app.update_cancel", json!({})).unwrap()["cancelled"], false);
    assert_eq!(plain.call("app.update_status", json!({})).unwrap()["can_install"], false);
    plain.set_update_feed(Ok(release_list(10))).unwrap();
    plain.call("app.check_update", json!({})).unwrap();
    let e = plain.call("app.update_now", json!({})).unwrap_err();
    assert!(e.message.contains("In dieser Umgebung"), "{e}");

    // Cancelled in the middle of the download.
    let s = setup(b"langsam".repeat(10_000), None, true);
    let e = &s.engine;
    // No release known yet: nothing to update to.
    assert_eq!(e.call("app.update_now", json!({})).unwrap_err().kind, aiscraft_core::ErrorKind::NotFound);
    e.call("app.check_update", json!({})).unwrap();
    e.call("app.update_now", json!({})).unwrap();
    wait_for(e, "downloading", |u| u.progress.as_ref().is_some_and(|p| p.phase == UpdatePhase::Download && p.done > 0));
    assert_eq!(e.call("app.update_cancel", json!({})).unwrap()["cancelled"], true);
    wait_for(e, "cancelled", |u| u.progress.is_none());
    assert!(e.state().update.error.is_none(), "a cancel is no error");
    assert!(e.state().update.message.as_deref().unwrap_or("").contains("abgebrochen"));
    assert!(downloads(&s.dir).is_empty(), "{:?}", downloads(&s.dir));

    // Ready, then „Abbrechen“: the restart is dropped.
    let s = setup(b"fertig".repeat(100), None, false);
    let e = &s.engine;
    e.call("app.check_update", json!({})).unwrap();
    e.call("app.update_now", json!({})).unwrap();
    wait_for(e, "ready", |u| u.progress.as_ref().is_some_and(|p| p.phase == UpdatePhase::Ready));
    assert_eq!(e.call("app.update_cancel", json!({})).unwrap()["cancelled"], true);
    assert!(e.state().update.progress.is_none());
    assert!(e.call("app.update_restart", json!({})).is_err());
    assert_eq!(e.ctx().0.update_job.restarts.load(Ordering::SeqCst), 0);
}
