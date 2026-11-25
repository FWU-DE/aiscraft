//! „Eigener Rechner (Sandbox)“ (#94), as n8n Nod's workspace: an optional, locked-down Linux
//! computer in which the model runs commands, converts files with LibreOffice and works on data
//! with Python, without touching the person's own computer.
//!
//! - Off by default. Settings › Fähigkeiten say what it can do, what it cannot see, the size,
//!   source and licence; nothing is downloaded, created or started before the person's yes
//!   (`sandbox.install` with `yes: true`). Engines in memory (tests, scripts) never download or
//!   start anything; tests hand them a fake machine.
//! - The machine is private to AIS.ChatCraft: on Windows an own WSL2 distribution (`wsl.rs`); on
//!   macOS and Linux it is not available yet (Lima, later). The image is n8n Nod's own Windows
//!   runtime, pinned and updated as Nod does it (`image.rs`).
//! - Work runs in a locked-down container in that machine (`container.rs`): no capabilities, no
//!   network unless switched on, 2 GB RAM, 2 CPUs, 1024 processes, one folder per chat.
//! - The tools follow Nod's rule like connector tools (`tools.rs`): listing and reading run;
//!   running, writing and converting need the owner's yes for exactly this command, folder and
//!   time limit, or the owner's own request. Background work refuses. What comes back is
//!   untrusted data and goes out to the model through „Namen schützen“ like every tool result.
//! - AIS.chat gives the model no tools (#70), so there the workspace is not offered.

pub(crate) mod container;
pub mod image;
pub(crate) mod runtime;
#[cfg(windows)]
pub(crate) mod wsl;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aiscraft_core::{Attachment, Error, Result, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{ConnectorTool, ConnectorView, Ctx, NoticeLevel};
use container as k;

/// The id the trust rules, the action log and the chat's sources know the workspace by.
pub(crate) const ID: &str = "sandbox";
pub(crate) const RUN: &str = "workspace_run";
pub(crate) const WRITE: &str = "workspace_write";
pub(crate) const READ: &str = "workspace_read";
pub(crate) const CONVERT: &str = "workspace_convert";
pub(crate) const LIST: &str = "workspace_list";
const KEY: &str = "sandbox";
/// Bump when the machine's layout changes: an older one is offered again.
const LAYOUT: u32 = 1;
const MARKER: &str = "installed.json";
pub(crate) const DEFAULT_TIMEOUT: u64 = 60;
pub(crate) const MAX_TIMEOUT: u64 = 600;
const MAX_COMMAND: usize = 8000;
/// Output of a command kept per stream.
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_WRITE: usize = 5 * 1024 * 1024;
const MAX_READ_TEXT: usize = 256 * 1024;

/// What people say before they switch it on.
pub const CAN: &str = "Führt Befehle in einem abgeschotteten Linux aus (Python 3, LibreOffice, Texterkennung, qpdf, Ghostscript, Noto-Schriften): Dateien umwandeln (DOCX, ODT und PPTX in PDF), Tabellen neu berechnen, PDFs reparieren oder zusammenfügen, Daten auswerten.";
pub const CANNOT: &str = "Er sieht weder deine Dateien und Laufwerke noch Schlüsselbund, Passwörter oder die Daten von AIS.ChatCraft, und er erreicht ohne deine Erlaubnis kein Netz. Dateien kommen nur hinein, wenn das Modell sie anlegt, und nur als Anhang im Chat heraus. Befehle, Schreiben und Umwandeln brauchen deine Freigabe.";
pub const NOT_WITH_AIS: &str = "Mit AIS.chat nicht verfügbar: AIS.chat gibt dem Modell keine Werkzeuge.";

/// What the settings and agents see.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxState {
    /// Switched on by the person (only possible once installed).
    pub enabled: bool,
    /// Containers may reach the network (off: `--network none`).
    pub network: bool,
    pub installed: bool,
    /// This engine can have a machine here (Windows; never engines in memory without a test machine).
    pub supported: bool,
    /// „WSL2“; empty where there is none.
    pub kind: String,
    /// The machine's name (`aiscraft-<profil>`).
    pub machine: String,
    /// What is missing or went wrong, for the person.
    pub problem: Option<String>,
    /// Downloading: bytes so far and in all.
    pub progress: Option<(u64, u64)>,
    /// The step of the installation running now.
    pub step: Option<String>,
    /// The Nod runtime installed („nod-v0.9.7“).
    pub version: String,
    /// A newer pinned Nod runtime than the installed one (`sandbox.update`), as people read it.
    pub update: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    enabled: bool,
    network: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    layout: u32,
    machine: String,
    sha256: String,
    #[serde(default)]
    version: String,
}

/// What the engine holds for the workspace.
pub(crate) struct Runtime {
    /// `<data>/sandbox` (engines in memory: a folder of their own).
    dir: PathBuf,
    machine: Mutex<Option<runtime::Shared>>,
    source: Mutex<Option<image::Source>>,
    busy: AtomicBool,
}

/// `aiscraft-default` for the usual data folder, else a short hash of the folder (portable).
fn machine_name(data_dir: Option<&Path>) -> String {
    use sha2::Digest as _;
    let usual = directories::ProjectDirs::from("ai", "uds", "aiscraft").map(|d| d.data_dir().to_path_buf());
    match data_dir {
        Some(d) if usual.as_deref() == Some(d) => "aiscraft-default".into(),
        Some(d) => {
            let h = sha2::Sha256::digest(d.display().to_string().as_bytes());
            format!("aiscraft-{}", h.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
        }
        None => "aiscraft-test".into(),
    }
}

impl Runtime {
    pub(crate) fn new(data_dir: Option<&Path>) -> Runtime {
        let real = data_dir.is_some() && !cfg!(test);
        Runtime {
            dir: data_dir.map(|d| d.join("sandbox")).unwrap_or_else(|| std::env::temp_dir().join(format!("aiscraft-sandbox-{}", crate::new_id()))),
            machine: Mutex::new(real.then(|| runtime::for_this_system(machine_name(data_dir)))),
            source: Mutex::new(if real { image::Source::github() } else { None }),
            busy: AtomicBool::new(false),
        }
    }

    fn machine(&self) -> Option<runtime::Shared> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn source(&self) -> Option<image::Source> {
        self.source.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Test hook: this machine stands in for WSL (nothing is started).
    #[cfg(test)]
    pub(crate) fn set_machine(&self, m: runtime::Shared) {
        *self.machine.lock().unwrap_or_else(PoisonError::into_inner) = Some(m);
    }

    /// Test hook: the image comes from a local server.
    #[cfg(test)]
    pub(crate) fn set_source(&self, base: &str, asset: &str, pinned: Option<&str>) {
        *self.source.lock().unwrap_or_else(PoisonError::into_inner) = Some(image::Source {
            base: base.into(),
            asset: asset.into(),
            sums: "SHA256SUMS.txt".into(),
            pinned: pinned.map(str::to_string),
            bytes: None,
            version: format!("test-{}", pinned.unwrap_or("").chars().take(6).collect::<String>()),
        });
    }

    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

// ---- state ---------------------------------------------------------------------------------

fn read_marker(c: &Ctx) -> Option<Marker> {
    let path = c.0.sandbox.dir.join(MARKER);
    if std::fs::metadata(&path).ok()?.len() > 64 * 1024 {
        return None;
    }
    let m: Marker = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let machine = c.0.sandbox.machine().map(|r| r.machine())?;
    (m.layout == LAYOUT && m.machine == machine).then_some(m)
}

fn saved(c: &Ctx) -> Saved {
    c.store(|s| Ok(s.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

fn save(c: &Ctx, st: &Saved) -> Result<()> {
    c.store(|s| s.set_setting(KEY, &serde_json::to_value(st)?))?;
    refresh(c);
    Ok(())
}

/// The state from what is saved and on disk (no machine is asked).
pub(crate) fn refresh(c: &Ctx) {
    let saved = saved(c);
    let machine = c.0.sandbox.machine();
    let marker = read_marker(c);
    let installed = marker.is_some();
    // As Nod compares its guest: the installed runtime against the pinned one.
    let update = match (&marker, c.0.sandbox.source()) {
        (Some(m), Some(src)) if src.pinned.as_deref().is_some_and(|p| p != m.sha256) => Some(src.version.clone()),
        _ => None,
    };
    c.update(|s| {
        let st = &mut s.sandbox;
        st.installed = installed;
        st.enabled = saved.enabled && installed;
        st.network = saved.network;
        st.supported = machine.as_ref().is_some_and(|m| m.supported());
        st.kind = machine.as_ref().map(|m| m.kind().to_string()).unwrap_or_default();
        st.machine = machine.as_ref().map(|m| m.machine()).unwrap_or_default();
        st.version = marker.as_ref().map(|m| m.version.clone()).unwrap_or_default();
        st.update = update;
    });
}

pub(crate) fn load(c: &Ctx) {
    refresh(c);
    if let Some(v) = c.state().sandbox.update.clone() {
        c.notice(NoticeLevel::Info, format!("Für den eigenen Rechner gibt es eine neue Version ({v}). Einstellungen › Fähigkeiten › Aktualisieren."));
    }
}

fn set_problem(c: &Ctx, p: Option<String>) {
    c.update(|s| s.sandbox.problem = p);
}

fn mb(n: u64) -> String {
    format!("{n} MB")
}

/// What the question before the download says.
pub(crate) fn offer(c: &Ctx) -> Value {
    let s = c.state().sandbox.clone();
    json!({
        "can": CAN,
        "cannot": CANNOT,
        "ais_chat": NOT_WITH_AIS,
        "size": image::APPROX_MB * 1_000_000,
        "size_text": mb(image::APPROX_MB),
        "disk_text": mb(image::DISK_MB),
        "source": image::SOURCE_TEXT,
        "license": image::LICENSE,
        "limits": "2 GB Arbeitsspeicher, 2 Prozessorkerne, 1024 Prozesse, höchstens 10 Minuten je Befehl",
        "kind": s.kind,
        "machine": s.machine,
        "supported": s.supported,
        "installed": s.installed,
        "enabled": s.enabled,
        "network": s.network,
        "problem": s.problem,
        "version": s.version,
        "update": s.update,
    })
}

// ---- commands ------------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sandbox.status",
        label: "Eigener Rechner: Stand",
        group: "Fähigkeiten",
        shortcut: None,
        params: "Antwort: was er kann und nicht sieht, Größe, Quelle, Lizenz, ob WSL2 da ist, eingerichtet, an, Netz",
        palette: false,
        run: |c, p| Box::pin(status_cmd(c, p)),
    },
    CommandSpec {
        id: "sandbox.install",
        label: "Eigenen Rechner einrichten",
        group: "Fähigkeiten",
        shortcut: None,
        params: "yes: bool (die Antwort der Person auf die Frage mit Größe, Quelle und Lizenz; ohne yes nur die Frage). Lädt das Abbild, legt den Rechner an und schaltet ihn ein",
        palette: false,
        run: |c, p| Box::pin(install(c, p)),
    },
    CommandSpec {
        id: "sandbox.update",
        label: "Eigenen Rechner aktualisieren",
        group: "Fähigkeiten",
        shortcut: None,
        params: "yes: bool (die Antwort der Person; ohne yes nur die Frage). Lädt die neuere Nod-Laufzeit, wenn ihre Prüfsumme von der eingerichteten abweicht, und legt den Rechner neu an (Arbeitsordner werden geleert)",
        palette: false,
        run: |c, p| Box::pin(update(c, p)),
    },
    CommandSpec {
        id: "sandbox.set",
        label: "Eigener Rechner ein/aus, Netz",
        group: "Fähigkeiten",
        shortcut: None,
        params: "enabled?: bool, network?: bool (Netz für den Arbeitsbereich; aus: kein Netz)",
        palette: false,
        run: |c, p| Box::pin(set(c, p)),
    },
    CommandSpec {
        id: "sandbox.stop",
        label: "Eigenen Rechner anhalten",
        group: "Fähigkeiten",
        shortcut: None,
        params: "Beendet alles, was darin läuft; die Dateien bleiben",
        palette: false,
        run: |c, _p| Box::pin(stop(c)),
    },
    CommandSpec {
        id: "sandbox.remove",
        label: "Eigenen Rechner entfernen",
        group: "Fähigkeiten",
        shortcut: None,
        params: "Löscht den Rechner mit allen Arbeitsordnern und schaltet ihn aus",
        palette: false,
        run: |c, _p| Box::pin(remove(c)),
    },
    CommandSpec {
        id: "sandbox.clear",
        label: "Arbeitsordner eines Chats leeren",
        group: "Fähigkeiten",
        shortcut: None,
        params: "conversation?: string (sonst der offene Chat)",
        palette: false,
        run: |c, p| Box::pin(clear(c, p)),
    },
];

/// `sandbox.status`: also asks the system whether WSL2 is there and the machine still exists.
async fn status_cmd(c: Ctx, _p: Value) -> Result<Value> {
    refresh(&c);
    if let Some(m) = c.0.sandbox.machine()
        && !c.0.sandbox.busy.load(Ordering::SeqCst)
    {
        let problem = m.probe().await.err();
        if problem.is_none() && read_marker(&c).is_some() && !m.exists().await {
            // Removed outside the app (`wsl --unregister`): offered again.
            let _ = std::fs::remove_file(c.0.sandbox.dir.join(MARKER));
            refresh(&c);
            set_problem(&c, Some(format!("Der eigene Rechner {} fehlt (außerhalb der App entfernt?). Bitte neu einrichten.", m.machine())));
        } else {
            set_problem(&c, problem);
        }
    } else if c.0.sandbox.machine().is_none() {
        set_problem(&c, Some("Diese Sitzung startet keinen eigenen Rechner (Test- oder Skriptmodus).".into()));
    }
    Ok(offer(&c))
}

fn step(c: &Ctx, what: Option<&str>) {
    c.update(|s| s.sandbox.step = what.map(str::to_string));
}

/// What an update does to the working folders.
pub const UPDATE_NOTE: &str = "Der eigene Rechner wird dabei neu angelegt; die Arbeitsordner der Chats werden geleert (Anhänge im Chat bleiben).";

/// `sandbox.install`: only with `yes: true`; without it, the question's content.
async fn install(c: Ctx, p: Value) -> Result<Value> {
    set_up(c, p, false).await
}

/// `sandbox.update`: a newer pinned Nod runtime replaces the installed one, as in Nod only when
/// its checksum differs, and only with `yes: true`.
async fn update(c: Ctx, p: Value) -> Result<Value> {
    refresh(&c);
    if !c.state().sandbox.installed {
        return Err(Error::invalid("Der eigene Rechner ist nicht eingerichtet; „Einrichten …“ lädt die aktuelle Version."));
    }
    if c.state().sandbox.update.is_none() {
        return Ok(json!({"updated": false, "version": c.state().sandbox.version}));
    }
    set_up(c, p, true).await
}

async fn set_up(c: Ctx, p: Value, updating: bool) -> Result<Value> {
    if p.get("yes").and_then(Value::as_bool) != Some(true) {
        let mut o = offer(&c);
        if let Some(m) = o.as_object_mut() {
            m.insert("needs_yes".into(), json!(true));
            if updating {
                m.insert("note".into(), json!(UPDATE_NOTE));
            }
        }
        return Ok(o);
    }
    let rt = &c.0.sandbox;
    let Some(machine) = rt.machine() else {
        return Err(Error::unsupported("Diese Sitzung startet keinen eigenen Rechner (Test- oder Skriptmodus); es wird nichts geladen."));
    };
    if let Err(why) = machine.probe().await {
        set_problem(&c, Some(why.clone()));
        return Err(Error::unsupported(why));
    }
    let Some(src) = rt.source() else {
        return Err(Error::unsupported("Für diesen Prozessor gibt es den eigenen Rechner noch nicht; es wird nichts geladen."));
    };
    if rt.busy.swap(true, Ordering::SeqCst) {
        return Err(Error::busy("Der eigene Rechner wird schon eingerichtet."));
    }
    set_problem(&c, None);
    let r = async {
        step(&c, Some("Abbild wird geladen"));
        let (file, sha256) = image::fetch(&c, &src, &rt.dir).await?;
        step(&c, Some("Rechner wird angelegt"));
        // A leftover of a broken setup (it is ours by name) goes first.
        if machine.exists().await {
            machine.destroy().await?;
        }
        let created = machine.create(file.clone(), rt.dir.join("machine")).await;
        // The image is in the machine now (or not needed any more): ~1 GB back.
        let _ = std::fs::remove_file(&file);
        created?;
        step(&c, Some("Arbeitsumgebung wird vorbereitet"));
        prepare(&machine, None, saved(&c).network).await?;
        let marker = Marker { layout: LAYOUT, machine: machine.machine(), sha256, version: src.version.clone() };
        crate::disk::write(&rt.dir.join(MARKER), &serde_json::to_vec_pretty(&marker)?)?;
        let mut st = saved(&c);
        st.enabled = true;
        save(&c, &st)
    }
    .await;
    rt.busy.store(false, Ordering::SeqCst);
    c.update(|s| {
        s.sandbox.progress = None;
        s.sandbox.step = None;
    });
    refresh(&c);
    if let Err(e) = &r {
        set_problem(&c, Some(e.message.clone()));
    }
    r?;
    c.notice(
        NoticeLevel::Success,
        if updating {
            format!("Eigener Rechner auf {} aktualisiert.", src.version)
        } else {
            "Eigener Rechner eingerichtet und an. Befehle darin laufen erst nach deiner Freigabe.".to_string()
        },
    );
    Ok(offer(&c))
}

/// `sandbox.set`: on only once installed; the network switch at any time.
async fn set(c: Ctx, p: Value) -> Result<Value> {
    let enabled = p.get("enabled").and_then(Value::as_bool);
    let network = p.get("network").and_then(Value::as_bool);
    if enabled.is_none() && network.is_none() {
        return Err(Error::invalid("enabled oder network (ja/nein) fehlt"));
    }
    if enabled == Some(true) && read_marker(&c).is_none() {
        return Err(Error::invalid("Der eigene Rechner ist noch nicht eingerichtet. Erst „Einrichten …“ (lädt das Abbild nach deiner Zustimmung)."));
    }
    let mut st = saved(&c);
    if let Some(e) = enabled {
        st.enabled = e;
    }
    if let Some(n) = network {
        st.network = n;
    }
    save(&c, &st)?;
    Ok(json!({"enabled": c.state().sandbox.enabled, "network": st.network}))
}

async fn stop(c: Ctx) -> Result<Value> {
    let Some(m) = c.0.sandbox.machine().filter(|_| read_marker(&c).is_some()) else {
        return Ok(json!({"stopped": false}));
    };
    m.stop().await?;
    Ok(json!({"stopped": true}))
}

async fn remove(c: Ctx) -> Result<Value> {
    let rt = &c.0.sandbox;
    if rt.busy.load(Ordering::SeqCst) {
        return Err(Error::busy("Der eigene Rechner wird gerade eingerichtet."));
    }
    let mut removed = false;
    if let Some(m) = rt.machine()
        && (read_marker(&c).is_some() || m.exists().await)
    {
        m.destroy().await?;
        removed = true;
    }
    // The marker first: a half-removed folder is never "installed".
    let _ = std::fs::remove_file(rt.dir.join(MARKER));
    if rt.dir.exists() {
        std::fs::remove_dir_all(&rt.dir).map_err(|e| Error::io(rt.dir.display(), &e))?;
    }
    let mut st = saved(&c);
    st.enabled = false;
    save(&c, &st)?;
    set_problem(&c, None);
    Ok(json!({"removed": removed}))
}

async fn clear(c: Ctx, p: Value) -> Result<Value> {
    let conv = match p.get("conversation").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => c.state().current.clone().ok_or_else(|| Error::invalid("Kein Chat angegeben oder offen"))?,
    };
    let chat = k::chat_folder(&conv)?;
    let Some(m) = c.0.sandbox.machine().filter(|_| read_marker(&c).is_some()) else {
        return Ok(json!({"cleared": false}));
    };
    let out = m.exec(k::machine_script(k::CLEAR_SCRIPT, &[&chat]), Vec::new(), Duration::from_secs(60), 4096).await?;
    if !out.ok() {
        return Err(Error::storage(format!("Arbeitsordner nicht geleert: {}", out.why())));
    }
    Ok(json!({"cleared": true}))
}

/// A purged chat (#77) leaves no working folder behind. Best effort, in the background.
pub(crate) fn forget(c: &Ctx, conv: &str) {
    if read_marker(c).is_none() {
        return;
    }
    let ctx = c.clone();
    let conv = conv.to_string();
    c.spawn(async move {
        if let Err(e) = clear(ctx, json!({"conversation": conv})).await {
            log::warn!("Arbeitsordner: {e}");
        }
    });
}

// ---- the machine ----------------------------------------------------------------------------

/// Docker running, the image loaded, the chat's folder there (idempotent, cheap when ready).
async fn prepare(m: &runtime::Shared, chat: Option<&str>, network: bool) -> Result<()> {
    let args = [k::IMAGE, chat.unwrap_or(""), k::IMAGE_TAR, image::WORKSPACE_TAR_SHA256, if network { "1" } else { "0" }];
    // Loading the 2 GB workspace image the first time takes a while.
    let out = m.exec(k::machine_script(k::PREPARE_SCRIPT, &args), Vec::new(), Duration::from_secs(900), 16 * 1024).await?;
    if out.ok() {
        Ok(())
    } else if out.timed_out {
        Err(Error::unsupported(format!("Der eigene Rechner ({}) antwortet nicht. „Anhalten“ in den Einstellungen und noch einmal versuchen.", m.machine())))
    } else {
        Err(Error::unsupported(format!("Der eigene Rechner ({}) startet nicht: {}", m.machine(), out.why())))
    }
}

/// The machine when it is on and installed, and whether the network is allowed.
fn usable(c: &Ctx) -> Result<(runtime::Shared, bool)> {
    let s = c.state().sandbox.clone();
    if !s.enabled {
        return Err(Error::refused("Der eigene Rechner ist aus. Die Person schaltet ihn in den Einstellungen › Fähigkeiten ein."));
    }
    let m = c.0.sandbox.machine().filter(|_| read_marker(c).is_some()).ok_or_else(|| Error::refused("Der eigene Rechner ist nicht eingerichtet."))?;
    Ok((m, s.network))
}

// ---- the tools ------------------------------------------------------------------------------

pub(crate) fn is_tool(name: &str) -> bool {
    matches!(name, RUN | WRITE | READ | CONVERT | LIST)
}

/// Listing and reading change nothing; the rest needs the owner.
pub(crate) fn read_only(name: &str) -> bool {
    matches!(name, READ | LIST)
}

/// Offered while it is on (and the chat allows it).
pub(crate) fn offered(c: &Ctx) -> bool {
    let s = c.state().sandbox.clone();
    s.enabled && s.installed
}

pub(crate) fn specs() -> Vec<ToolSpec> {
    let path = json!({"type": "string", "description": "Pfad im Arbeitsordner /work dieses Chats, z. B. daten/noten.csv"});
    vec![
        ToolSpec {
            name: RUN.into(),
            description: format!("Führt einen Shell-Befehl im eigenen, abgeschotteten Linux-Rechner aus (Python 3, LibreOffice, qpdf, Ghostscript; kein Netz, keine Dateien der Person). Arbeitsordner /work gehört zu diesem Chat. Braucht die Freigabe der Person für genau diesen Befehl, Ordner und dieses Zeitlimit. Zeitlimit {DEFAULT_TIMEOUT} s, höchstens {MAX_TIMEOUT} s."),
            input_schema: json!({"type": "object", "properties": {
                "command": {"type": "string", "description": "Shell-Befehl (sh)"},
                "folder": {"type": "string", "description": "Ordner im Arbeitsordner, Standard ."},
                "timeout_s": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT}
            }, "required": ["command"], "additionalProperties": false}),
        },
        ToolSpec {
            name: WRITE.into(),
            description: "Speichert Text als Datei im Arbeitsordner des eigenen Rechners (bis 5 MB), z. B. ein Python-Skript oder eine CSV. Braucht die Freigabe der Person.".into(),
            input_schema: json!({"type": "object", "properties": {"path": path, "text": {"type": "string"}}, "required": ["path", "text"], "additionalProperties": false}),
        },
        ToolSpec {
            name: READ.into(),
            description: "Liest eine Datei aus dem Arbeitsordner des eigenen Rechners. Text kommt als Text zurück; mit attach: true (und bei PDFs, Bildern, Office-Dateien immer) erscheint die Datei als Anhang im Chat, den die Person speichern kann.".into(),
            input_schema: json!({"type": "object", "properties": {"path": path, "attach": {"type": "boolean"}}, "required": ["path"], "additionalProperties": false}),
        },
        ToolSpec {
            name: CONVERT.into(),
            description: format!("Wandelt eine Datei im Arbeitsordner mit LibreOffice um (z. B. DOCX → PDF, ODS → XLSX; Tabellen werden dabei neu berechnet). Das Ergebnis liegt daneben und erscheint als Anhang im Chat. Formate: {}. Braucht die Freigabe der Person.", k::FORMATS.join(", ")),
            input_schema: json!({"type": "object", "properties": {"path": path, "to": {"type": "string", "enum": k::FORMATS}}, "required": ["path", "to"], "additionalProperties": false}),
        },
        ToolSpec {
            name: LIST.into(),
            description: "Listet den Arbeitsordner dieses Chats im eigenen Rechner auf (Typ, Größe, Pfad).".into(),
            input_schema: json!({"type": "object", "properties": {"folder": {"type": "string"}}, "additionalProperties": false}),
        },
    ]
}

/// The arguments as they run: folder and time limit filled in, so the trust check, the approval
/// card and the action log name exactly what will run.
pub(crate) fn normalize(name: &str, args: &Value) -> Value {
    let mut a = args.clone();
    if let Some(o) = a.as_object_mut() {
        match name {
            RUN => {
                let folder = o.get("folder").and_then(Value::as_str).unwrap_or(".").to_string();
                let shown = match k::rel_path(&folder) {
                    Ok(f) if f == "." => "/work".to_string(),
                    Ok(f) => format!("/work/{f}"),
                    Err(_) => folder,
                };
                o.insert("folder".into(), json!(shown));
                let t = o.get("timeout_s").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT);
                o.insert("timeout_s".into(), json!(t));
            }
            LIST => {
                o.entry("folder").or_insert_with(|| json!("/work"));
            }
            _ => {}
        }
    }
    a
}

/// The connector view the approval card, trust check and action log use.
pub(crate) fn view() -> ConnectorView {
    ConnectorView {
        id: ID.into(),
        name: "Eigener Rechner".into(),
        enabled: true,
        target: String::new(),
        status: "connected".into(),
        error: None,
        tools: Vec::new(),
        extension: None,
    }
}

pub(crate) fn tool(name: &str) -> ConnectorTool {
    let title = match name {
        RUN => "Befehl ausführen",
        WRITE => "Datei anlegen",
        READ => "Datei lesen",
        CONVERT => "Datei umwandeln",
        _ => "Arbeitsordner auflisten",
    };
    let spec = specs().into_iter().find(|s| s.name == name);
    ConnectorTool {
        llm_name: name.to_string(),
        name: name.to_string(),
        title: title.into(),
        description: spec.as_ref().map(|s| s.description.clone()).unwrap_or_default(),
        read_only: read_only(name),
        destructive: false,
        input_schema: spec.map(|s| s.input_schema).unwrap_or(Value::Null),
        ui_resource: None,
        app_only: false,
        app_callable: false,
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Before anything is asked: refuse what never runs (background work, switched off, bad
/// arguments) and give the tool for the call.
pub(crate) fn prepare_call(c: &Ctx, name: &str, args: &Value, interactive: bool) -> std::result::Result<ConnectorTool, String> {
    if !interactive {
        return Err("Abgelehnt: Arbeit im Hintergrund nutzt den eigenen Rechner nicht.".into());
    }
    usable(c).map_err(|e| e.message)?;
    let check = |r: Result<String>| r.map(|_| ()).map_err(|e| e.message);
    match name {
        RUN => {
            let cmd = arg(args, "command");
            if cmd.trim().is_empty() {
                return Err("Parameter command fehlt".into());
            }
            if cmd.len() > MAX_COMMAND || cmd.contains('\u{0}') {
                return Err(format!("Der Befehl ist länger als {MAX_COMMAND} Zeichen oder enthält Nullzeichen."));
            }
            check(k::rel_path(args.get("folder").and_then(Value::as_str).unwrap_or(".")))?;
        }
        WRITE => {
            check(k::rel_path(arg(args, "path")))?;
            if arg(args, "path").trim().trim_start_matches("/work").trim_matches('/').is_empty() {
                return Err("Parameter path nennt keine Datei".into());
            }
            let text = args.get("text").and_then(Value::as_str).ok_or("Parameter text fehlt")?;
            if text.len() > MAX_WRITE {
                return Err(format!("Der Text ist größer als {} MB.", MAX_WRITE / (1024 * 1024)));
            }
        }
        READ | CONVERT => {
            check(k::rel_path(arg(args, "path")))?;
            if name == CONVERT && !k::FORMATS.contains(&arg(args, "to")) {
                return Err(format!("Format „{}“ gibt es nicht ({}).", arg(args, "to"), k::FORMATS.join(", ")));
            }
        }
        _ => check(k::rel_path(args.get("folder").and_then(Value::as_str).unwrap_or(".")))?,
    }
    Ok(tool(name))
}

fn container_name() -> String {
    format!("aiscraft-{}", crate::new_id().replace('-', "").chars().take(16).collect::<String>())
}

/// Run one call that may run. Files that come out become attachments of the answer.
pub(crate) async fn execute(c: &Ctx, conv: &str, name: &str, args: &Value) -> Result<(String, Vec<Attachment>)> {
    let (m, network) = usable(c)?;
    let chat = k::chat_folder(conv)?;
    prepare(&m, Some(&chat), network).await?;
    let start = |script: &str, a: &[&str]| k::docker_run(&chat, &container_name(), network, script, a);
    match name {
        RUN => {
            let folder = k::rel_path(args.get("folder").and_then(Value::as_str).unwrap_or("."))?;
            let secs = args.get("timeout_s").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT);
            let cmd = arg(args, "command");
            let argv = start(k::RUN_SCRIPT, &[&folder, cmd, &secs.to_string()]);
            let cname = argv.get(5).cloned().unwrap_or_default();
            // The container's own `timeout` ends the command; this is the backstop.
            let out = m.exec(argv, Vec::new(), Duration::from_secs(secs + 30), MAX_OUTPUT).await?;
            if out.timed_out {
                let _ = m.exec(vec!["docker".into(), "kill".into(), cname], Vec::new(), Duration::from_secs(30), 4096).await;
            }
            Ok((run_report(cmd, &folder, secs, &out), Vec::new()))
        }
        WRITE => {
            let path = k::rel_path(arg(args, "path"))?;
            let text = arg(args, "text");
            if text.len() > MAX_WRITE {
                return Err(Error::invalid("Der Text ist zu groß"));
            }
            let out = m.exec(start(k::WRITE_SCRIPT, &[&path]), text.as_bytes().to_vec(), Duration::from_secs(60), 16 * 1024).await?;
            if !out.ok() {
                return Err(Error::storage(format!("/work/{path} nicht gespeichert: {}", out.why())));
            }
            Ok((format!("Gespeichert: /work/{path} ({} Bytes).", text.len()), Vec::new()))
        }
        READ => {
            let path = k::rel_path(arg(args, "path"))?;
            let bytes = read_file(&m, &start, &path).await?;
            let attach = args.get("attach").and_then(Value::as_bool).unwrap_or(false);
            let text = (!attach && !bytes.contains(&0)).then(|| String::from_utf8(bytes.clone()).ok()).flatten().filter(|_| !binary_name(&path));
            match text {
                Some(t) => {
                    let cut = t.len() > MAX_READ_TEXT;
                    let shown: String = t.chars().take(MAX_READ_TEXT).collect();
                    Ok((format!("Datei /work/{path}:\n\n{shown}{}", if cut { "\n… (gekürzt)" } else { "" }), Vec::new()))
                }
                None => {
                    let a = attachment(c, &path, &bytes)?;
                    Ok((format!("/work/{path} liegt als Anhang „{}“ im Chat ({} Bytes).", a.name, bytes.len()), vec![a]))
                }
            }
        }
        CONVERT => {
            let path = k::rel_path(arg(args, "path"))?;
            let to = arg(args, "to");
            if !k::FORMATS.contains(&to) {
                return Err(Error::invalid(format!("Format {to} gibt es nicht")));
            }
            let out = m.exec(start(k::CONVERT_SCRIPT, &[&path, to]), Vec::new(), Duration::from_secs(300), MAX_OUTPUT).await?;
            if !out.ok() {
                return Err(Error::unsupported(format!("/work/{path} ließ sich nicht umwandeln: {}", out.why())));
            }
            let stem = path.rsplit_once('.').map(|(s, _)| s).unwrap_or(&path);
            let result = format!("{stem}.{to}");
            let bytes = read_file(&m, &start, &result).await?;
            let a = attachment(c, &result, &bytes)?;
            Ok((format!("Umgewandelt: /work/{result} liegt als Anhang „{}“ im Chat.", a.name), vec![a]))
        }
        LIST => {
            let folder = k::rel_path(args.get("folder").and_then(Value::as_str).unwrap_or("."))?;
            let out = m.exec(start(k::LIST_SCRIPT, &[&folder]), Vec::new(), Duration::from_secs(60), MAX_OUTPUT).await?;
            if !out.ok() {
                return Err(Error::not_found(format!("/work/{folder}: {}", out.why())));
            }
            let rows = String::from_utf8_lossy(&out.stdout).into_owned();
            Ok((format!("Arbeitsordner /work/{folder}:\n{}", if rows.trim().is_empty() { "(leer)" } else { rows.trim_end() }), Vec::new()))
        }
        _ => Err(Error::not_found(format!("Werkzeug {name} gibt es nicht"))),
    }
}

/// What a command gave back, for the model (untrusted data, wrapped by `tools.rs`).
fn run_report(cmd: &str, folder: &str, secs: u64, out: &runtime::Output) -> String {
    let status = if out.timed_out || out.code == Some(124) {
        format!("Zeitlimit von {secs} s erreicht, der Befehl wurde beendet")
    } else {
        match out.code {
            Some(c) => format!("Exit-Code {c}"),
            None => "beendet ohne Exit-Code".into(),
        }
    };
    let mut s = format!("Befehl: {cmd}\nOrdner: /work/{folder}\nErgebnis: {status}\n");
    let so = String::from_utf8_lossy(&out.stdout);
    // Docker under WSL2 warns on every start that swap is not accounted (the memory limit holds).
    let se: String = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| !l.starts_with("WARNING: Your kernel does not support swap limit"))
        .collect::<Vec<_>>()
        .join("\n");
    if !so.trim().is_empty() {
        s.push_str(&format!("--- Ausgabe ---\n{}\n", so.trim_end()));
    }
    if !se.trim().is_empty() {
        s.push_str(&format!("--- Fehlerausgabe ---\n{}\n", se.trim_end()));
    }
    if out.truncated {
        s.push_str(&format!("(Ausgabe nach {} KB gekürzt)\n", MAX_OUTPUT / 1024));
    }
    s
}

async fn read_file(m: &runtime::Shared, start: &impl Fn(&str, &[&str]) -> Vec<String>, path: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let max = crate::files::MAX_FILE_BYTES;
    let out = m.exec(start(k::READ_SCRIPT, &[path, &max.to_string()]), Vec::new(), Duration::from_secs(120), (max as usize).saturating_mul(2)).await?;
    if !out.ok() {
        return Err(Error::not_found(format!("/work/{path}: {}", out.why())));
    }
    let text: String = String::from_utf8_lossy(&out.stdout).split_whitespace().collect();
    base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).map_err(|e| Error::protocol(format!("/work/{path}: unlesbare Antwort ({e})")))
}

/// Files that are never shown as text.
fn binary_name(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "pdf" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "zip")
}

/// A file from the workspace as an attachment of the answer (a copy in the attachments folder).
fn attachment(c: &Ctx, path: &str, bytes: &[u8]) -> Result<Attachment> {
    if bytes.len() as u64 > crate::files::MAX_FILE_BYTES {
        return Err(Error::invalid("Die Datei ist größer als 25 MB"));
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let kept = crate::files::keep_copy(&c.0.attachments_dir, Path::new(&name), bytes)?;
    Ok(Attachment {
        mime: crate::files::mime_of(&kept).to_string(),
        size: bytes.len() as u64,
        local_path: Some(kept.display().to_string()),
        name,
        ..Attachment::default()
    })
}
