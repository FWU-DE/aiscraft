//! Local files (#25), as in n8n's Nod: a built-in tool that lists, reads and writes files in the
//! person's home folder and on external drives, without a connector of its own.
//!
//! Off by default. When it is off, the first call asks in the chat whether to switch it on; the
//! yes switches it on and runs that call. Work without a person present (tasks, schedules) never
//! switches it on and never touches files. Every path is resolved first (`~`, links followed) and
//! the resolved path is checked: it must lie in the home folder or on an external drive, never in
//! a hidden folder (`.ssh`, `.config`, browser profiles, keyrings), never in AIS.ChatCraft's own
//! data, never in a folder the person keeps out, and never be a file that usually holds secrets.
//! Listing runs without asking; reading and writing follow the trust check and the approval
//! rules like a connector tool that changes something (`tools.rs`).

use std::path::{Component, Path, PathBuf};

use aiscraft_core::{Error, Result, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::req_str;
use crate::{ConnectorTool, ConnectorView, Ctx, NoticeLevel};

/// The id the trust rules, the action log and the chat's sources know local files by.
pub(crate) const ID: &str = "local-files";
pub(crate) const LIST: &str = "local_files_list";
pub(crate) const READ: &str = "local_files_read";
pub(crate) const WRITE: &str = "local_files_write";
const KEY: &str = "local_files";

/// Most entries a listing shows.
const MAX_LIST: usize = 500;
/// Most bytes of a text file read.
const MAX_READ: u64 = 256 * 1024;
/// Largest PDF whose text is read.
const MAX_PDF: u64 = 32 * 1024 * 1024;
/// Largest text written.
const MAX_WRITE: usize = 1024 * 1024;
/// Most folders kept out.
const MAX_KEEP_OUT: usize = 100;
const MAX_PATH_CHARS: usize = 4096;

/// What people see and change in the settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalFilesState {
    pub enabled: bool,
    /// Folders local files never touch, as resolved paths; only the person removes them.
    pub keep_out: Vec<String>,
}

pub(crate) fn is_tool(name: &str) -> bool {
    matches!(name, LIST | READ | WRITE)
}

pub(crate) fn specs() -> Vec<ToolSpec> {
    let path = json!({"type": "string", "description": "voller Pfad, ~ für den Persönlichen Ordner (z. B. ~/Downloads)"});
    vec![
        ToolSpec {
            name: LIST.into(),
            description: "Listet einen Ordner auf dem Rechner der Person (Persönlicher Ordner, externe Laufwerke), neueste zuerst. Versteckte Ordner und Dateien mit Geheimnissen bleiben immer gesperrt.".into(),
            input_schema: json!({"type": "object", "properties": {"path": path}, "required": ["path"], "additionalProperties": false}),
        },
        ToolSpec {
            name: READ.into(),
            description: "Liest eine Textdatei oder den Text einer PDF auf dem Rechner der Person (bis 256 KB). Nur Dateien, um die die Person bittet.".into(),
            input_schema: json!({"type": "object", "properties": {"path": path}, "required": ["path"], "additionalProperties": false}),
        },
        ToolSpec {
            name: WRITE.into(),
            description: "Speichert Text als Datei auf dem Rechner der Person (bis 1 MB), z. B. einen Bericht in ~/Dokumente. Eine vorhandene Datei wird nur mit replace: true ersetzt. Braucht die Freigabe der Person.".into(),
            input_schema: json!({"type": "object", "properties": {"path": path, "text": {"type": "string"}, "replace": {"type": "boolean"}}, "required": ["path", "text"], "additionalProperties": false}),
        },
    ]
}

pub(crate) static COMMANDS: &[crate::CommandSpec] = &[
    crate::CommandSpec {
        id: "files.local_set",
        label: "Lokale Dateien ein/aus",
        group: "Konnektoren",
        shortcut: None,
        params: "enabled: bool",
        palette: false,
        run: |c, p| Box::pin(set(c, p)),
    },
    crate::CommandSpec {
        id: "files.keep_out_add",
        label: "Ordner für lokale Dateien sperren",
        group: "Konnektoren",
        shortcut: None,
        params: "path: string (Ordner, ~ erlaubt)",
        palette: false,
        run: |c, p| Box::pin(keep_out_add(c, p)),
    },
    crate::CommandSpec {
        id: "files.keep_out_remove",
        label: "Gesperrten Ordner freigeben",
        group: "Konnektoren",
        shortcut: None,
        params: "path: string (wie in der Liste)",
        palette: false,
        run: |c, p| Box::pin(keep_out_remove(c, p)),
    },
];

// ---- state ---------------------------------------------------------------------------------

pub(crate) fn load(c: &Ctx) {
    let st: LocalFilesState = c.store(|s| Ok(s.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default();
    c.update(|s| s.local_files = st);
}

fn save(c: &Ctx, st: LocalFilesState) -> Result<()> {
    c.store(|s| s.set_setting(KEY, &serde_json::to_value(&st)?))?;
    c.update(|s| s.local_files = st);
    Ok(())
}

pub(crate) fn enabled(c: &Ctx) -> bool {
    c.state().local_files.enabled
}

/// `files.local_set`: the person switches local files on or off in the settings.
pub(crate) async fn set(c: Ctx, p: Value) -> Result<Value> {
    let on = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| Error::invalid("enabled (ja/nein) fehlt"))?;
    let mut st = c.state().local_files.clone();
    st.enabled = on;
    save(&c, st)?;
    Ok(json!({"enabled": on}))
}

/// `files.keep_out_add`: a folder local files never touches.
pub(crate) async fn keep_out_add(c: Ctx, p: Value) -> Result<Value> {
    let raw = req_str(&p, "path")?;
    let path = expand(&raw, home().as_deref())?;
    let resolved = std::fs::canonicalize(&path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
    if !resolved.is_dir() {
        return Err(Error::invalid("Das ist kein Ordner"));
    }
    let shown = resolved.display().to_string();
    let mut st = c.state().local_files.clone();
    if !st.keep_out.contains(&shown) {
        if st.keep_out.len() >= MAX_KEEP_OUT {
            return Err(Error::invalid(format!("Höchstens {MAX_KEEP_OUT} gesperrte Ordner; entferne erst einen")));
        }
        st.keep_out.push(shown.clone());
    }
    save(&c, st)?;
    Ok(json!({"path": shown}))
}

/// `files.keep_out_remove`: only the person takes a folder off the list.
pub(crate) async fn keep_out_remove(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let mut st = c.state().local_files.clone();
    let before = st.keep_out.len();
    st.keep_out.retain(|k| k != &path);
    if st.keep_out.len() == before {
        return Err(Error::not_found("Dieser Ordner ist nicht gesperrt"));
    }
    save(&c, st)?;
    Ok(json!({}))
}

// ---- where files may be --------------------------------------------------------------------

fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// The places local files may work in, and those it never touches, all resolved.
#[derive(Debug, Clone, Default)]
pub(crate) struct Places {
    pub roots: Vec<PathBuf>,
    pub refused: Vec<PathBuf>,
    pub home: Option<PathBuf>,
}

fn places(c: &Ctx) -> Places {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let (roots, home) = match c.0.ext.local_roots() {
        // Tests point local files at a folder of their own; engines in memory never see the real home.
        Some(r) => {
            let home = r.first().cloned();
            (r, home)
        }
        None if c.0.ext.data_dir.is_none() => (Vec::new(), None),
        None => {
            let home = home();
            let mut roots: Vec<PathBuf> = home.iter().cloned().collect();
            if cfg!(target_os = "macos") {
                roots.push(PathBuf::from("/Volumes"));
            } else if cfg!(target_os = "linux")
                && let Some(user) = home.as_ref().and_then(|h| h.file_name()).map(|n| n.to_os_string())
            {
                roots.push(Path::new("/media").join(&user));
                roots.push(Path::new("/run/media").join(&user));
            }
            (roots, home)
        }
    };
    let mut refused: Vec<PathBuf> = c.state().local_files.keep_out.iter().map(PathBuf::from).collect();
    if let Some(d) = &c.0.ext.data_dir {
        refused.push(d.clone());
    }
    Places {
        roots: roots.iter().filter_map(|r| canon(r)).collect(),
        refused: refused.iter().filter_map(|r| canon(r).or_else(|| Some(r.clone()))).collect(),
        home: home.and_then(|h| canon(&h)),
    }
}

/// `~` and `~/…` become the home folder; anything else must be absolute.
fn expand(raw: &str, home: Option<&Path>) -> Result<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().count() > MAX_PATH_CHARS || raw.chars().any(char::is_control) {
        return Err(Error::invalid("Kein gültiger Pfad"));
    }
    let path = if raw == "~" {
        home.map(Path::to_path_buf).ok_or_else(|| Error::not_found("Kein Persönlicher Ordner"))?
    } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        home.ok_or_else(|| Error::not_found("Kein Persönlicher Ordner"))?.join(rest)
    } else {
        PathBuf::from(raw)
    };
    if !path.is_absolute() {
        return Err(Error::invalid("Bitte einen vollständigen Pfad angeben (z. B. ~/Downloads/datei.pdf)"));
    }
    Ok(path)
}

/// Names of files that usually hold secrets (keys, password stores, tokens).
pub(crate) fn secret_name(name: &str) -> bool {
    let n = name.to_lowercase();
    let ext = n.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    const EXTS: &[&str] = &[
        "pem",
        "key",
        "p12",
        "pfx",
        "ppk",
        "kdbx",
        "kdb",
        "1pux",
        "1pif",
        "ovpn",
        "gpg",
        "asc",
        "pgp",
        "tfstate",
        "keychain",
        "keychain-db",
        "jks",
        "keystore",
    ];
    EXTS.contains(&ext)
        || n.starts_with("id_rsa")
        || n.starts_with("id_dsa")
        || n.starts_with("id_ecdsa")
        || n.starts_with("id_ed25519")
        || n == "wallet.dat"
        || n == "known_hosts"
        || n == "authorized_keys"
        || n.starts_with(".env")
        || n.ends_with(".env")
        || (n.contains("credentials") && (ext == "csv" || ext == "json"))
        || (n.contains("service-account") && ext == "json")
        || (n.starts_with("bitwarden_export") && (ext == "json" || ext == "csv"))
        || n.ends_with(".tfstate.backup")
}

/// Folders where programs keep their data (browser profiles, keyrings) on macOS and Windows.
fn system_folder(name: &str) -> bool {
    let n = name.to_lowercase();
    matches!(n.as_str(), "library" | "appdata" | "application data" | "backups.backupdb" | "$recycle.bin" | "system volume information" | "snap")
}

/// Program files local files never creates (they would run with a click).
fn program_name(name: &str) -> bool {
    let n = name.to_lowercase();
    let ext = n.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    const EXTS: &[&str] = &[
        "exe",
        "bat",
        "cmd",
        "com",
        "ps1",
        "vbs",
        "msi",
        "lnk",
        "url",
        "scr",
        "sh",
        "bash",
        "zsh",
        "fish",
        "command",
        "app",
        "pkg",
        "dmg",
        "desktop",
        "run",
        "appimage",
        "jar",
        "flatpak",
        "flatpakref",
        "flatpakrepo",
        "service",
        "py",
        "pl",
        "rb",
    ];
    EXTS.contains(&ext)
}

/// Why `path` (resolved) is refused, if it is.
pub(crate) fn refusal(places: &Places, path: &Path) -> Option<String> {
    let shown = path.display().to_string();
    if places.refused.iter().any(|r| path.starts_with(r)) {
        return Some(format!("{shown} ist für lokale Dateien gesperrt."));
    }
    let Some(root) = places.roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.components().count()) else {
        return Some(format!("{shown} liegt außerhalb des Persönlichen Ordners und der externen Laufwerke."));
    };
    let rel = path.strip_prefix(root).unwrap_or(path);
    for part in rel.components() {
        let Component::Normal(name) = part else {
            return Some(format!("{shown} ist kein erlaubter Pfad."));
        };
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            return Some(format!("{shown} liegt in einem versteckten Ordner oder ist versteckt (immer gesperrt, z. B. .ssh, .config)."));
        }
        if system_folder(&name) {
            return Some(format!("{shown} liegt in einem Programmordner (immer gesperrt)."));
        }
        if secret_name(&name) {
            return Some(format!("{shown} enthält üblicherweise Geheimnisse (Schlüssel, Passwörter) und ist immer gesperrt."));
        }
    }
    None
}

/// The resolved path of an existing file or folder, checked.
pub(crate) fn resolve_existing(places: &Places, raw: &str) -> Result<PathBuf> {
    let path = expand(raw, places.home.as_deref())?;
    // Links are followed: the place they point to is what gets checked.
    let resolved = std::fs::canonicalize(&path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
    match refusal(places, &resolved) {
        Some(why) => Err(Error::refused(why)),
        None => Ok(resolved),
    }
}

/// Where a write goes: the resolved folder plus the file name, checked; whether it exists.
pub(crate) fn resolve_target(places: &Places, raw: &str) -> Result<(PathBuf, bool)> {
    let path = expand(raw, places.home.as_deref())?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| Error::invalid("Der Pfad nennt keinen Dateinamen"))?;
    let parent = path.parent().ok_or_else(|| Error::invalid("Der Pfad nennt keinen Ordner"))?;
    let dir = std::fs::canonicalize(parent).map_err(|e| Error::not_found(format!("Ordner {}: {e}", parent.display())))?;
    let target = dir.join(&name);
    if let Some(why) = refusal(places, &target) {
        return Err(Error::refused(why));
    }
    if program_name(&name) {
        return Err(Error::refused(format!("{name}: Programme und Skripte legen lokale Dateien nicht an.")));
    }
    let exists = match std::fs::symlink_metadata(&target) {
        Ok(m) if m.file_type().is_symlink() => return Err(Error::refused(format!("{} ist eine Verknüpfung; schreibe an ihr Ziel.", target.display()))),
        Ok(m) if m.is_dir() => return Err(Error::invalid(format!("{} ist ein Ordner", target.display()))),
        Ok(m) => {
            if executable(&m) {
                return Err(Error::refused(format!("{} ist ausführbar; lokale Dateien ändern keine Programme.", target.display())));
            }
            true
        }
        Err(_) => false,
    };
    Ok((target, exists))
}

#[cfg(unix)]
fn executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_m: &std::fs::Metadata) -> bool {
    false
}

// ---- the tool path (tools.rs) ---------------------------------------------------------------

/// The connector view the approval card, trust check and action log use.
pub(crate) fn view() -> ConnectorView {
    ConnectorView {
        id: ID.into(),
        name: "Lokale Dateien".into(),
        enabled: true,
        target: String::new(),
        status: "connected".into(),
        error: None,
        tools: Vec::new(),
        extension: None,
    }
}

/// The tool for one call; `destructive` when a write replaces a file.
pub(crate) fn tool(name: &str, destructive: bool) -> ConnectorTool {
    let title = match name {
        LIST => "Ordner auflisten",
        READ => "Datei lesen",
        _ if destructive => "Datei ersetzen",
        _ => "Datei speichern",
    };
    let spec = specs().into_iter().find(|s| s.name == name);
    ConnectorTool {
        llm_name: name.to_string(),
        name: name.to_string(),
        title: title.into(),
        description: spec.as_ref().map(|s| s.description.clone()).unwrap_or_default(),
        read_only: name == LIST,
        destructive,
        input_schema: spec.map(|s| s.input_schema).unwrap_or(Value::Null),
    }
}

/// Before anything is asked: refuse what never runs (work without a person, refused paths), and
/// give the tool for the call. The path is checked again right before it runs.
pub(crate) fn prepare(c: &Ctx, name: &str, args: &Value, interactive: bool) -> std::result::Result<ConnectorTool, String> {
    if !interactive {
        return Err(if enabled(c) {
            "Abgelehnt: Arbeit im Hintergrund greift nicht auf lokale Dateien zu.".into()
        } else {
            "Abgelehnt: Lokale Dateien sind aus, und nur die Person kann sie im Chat einschalten. Hintergrundarbeit kann das nicht.".into()
        });
    }
    let raw = args.get("path").and_then(Value::as_str).unwrap_or("");
    let places = places(c);
    let destructive = match name {
        WRITE => {
            let text = args.get("text").and_then(Value::as_str).ok_or("Parameter text fehlt")?;
            if text.len() > MAX_WRITE {
                return Err(format!("Der Text ist größer als {} KB.", MAX_WRITE / 1024));
            }
            let (target, exists) = resolve_target(&places, raw).map_err(|e| e.message)?;
            if exists && args.get("replace").and_then(Value::as_bool) != Some(true) {
                return Err(format!("{} gibt es schon. Zum Ersetzen mit replace: true aufrufen (die Person wird gefragt).", target.display()));
            }
            exists
        }
        _ => {
            resolve_existing(&places, raw).map_err(|e| e.message)?;
            false
        }
    };
    Ok(tool(name, destructive))
}

/// The note on the approval card while local files are still off.
pub(crate) fn enable_note(c: &Ctx) -> Option<String> {
    (!enabled(c))
        .then(|| "Lokale Dateien sind aus. „Erlauben“ schaltet sie ein (für alle Chats, abschaltbar in den Einstellungen) und führt diese Aktion aus.".into())
}

/// Run a call that may run. A yes while local files were off switches them on.
pub(crate) fn execute(c: &Ctx, name: &str, args: &Value) -> Result<String> {
    if !enabled(c) {
        let mut st = c.state().local_files.clone();
        st.enabled = true;
        save(c, st)?;
        c.notice(NoticeLevel::Info, "Lokale Dateien sind jetzt eingeschaltet (Einstellungen → Konnektoren).");
    }
    let places = places(c);
    let raw = args.get("path").and_then(Value::as_str).unwrap_or("");
    match name {
        LIST => list(&places, raw),
        READ => read(&places, raw),
        WRITE => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if text.len() > MAX_WRITE {
                return Err(Error::invalid("Der Text ist zu groß"));
            }
            let (target, exists) = resolve_target(&places, raw)?;
            if exists && args.get("replace").and_then(Value::as_bool) != Some(true) {
                return Err(Error::invalid(format!("{} gibt es schon", target.display())));
            }
            crate::disk::write(&target, text.as_bytes())?;
            Ok(format!("{} {}.", if exists { "Ersetzt:" } else { "Gespeichert:" }, target.display()))
        }
        _ => Err(Error::not_found(format!("Werkzeug {name} gibt es nicht"))),
    }
}

fn list(places: &Places, raw: &str) -> Result<String> {
    let dir = resolve_existing(places, raw)?;
    if !dir.is_dir() {
        return Err(Error::invalid(format!("{} ist kein Ordner", dir.display())));
    }
    let rd = std::fs::read_dir(&dir).map_err(|e| Error::io(dir.display(), &e))?;
    let mut hidden = 0usize;
    let mut rows: Vec<(std::time::SystemTime, String)> = Vec::new();
    // A folder with a million entries is not read to the end.
    for entry in rd.flatten().take(20_000) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = std::fs::canonicalize(&path).unwrap_or(path.clone());
        if refusal(places, &path).is_some() || refusal(places, &target).is_some() {
            hidden += 1;
            continue;
        }
        let meta = std::fs::metadata(&target).ok();
        let when = meta.as_ref().and_then(|m| m.modified().ok()).unwrap_or(std::time::UNIX_EPOCH);
        let row = match &meta {
            Some(m) if m.is_dir() => format!("{name}/"),
            Some(m) => format!("{name} ({})", size(m.len())),
            None => name,
        };
        rows.push((when, row));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let total = rows.len();
    let mut out = format!("Ordner {}:\n", dir.display());
    for (_, r) in rows.iter().take(MAX_LIST) {
        out.push_str(&format!("- {r}\n"));
    }
    if total > MAX_LIST {
        out.push_str(&format!("… und {} weitere\n", total - MAX_LIST));
    }
    if total == 0 {
        out.push_str("(leer)\n");
    }
    if hidden > 0 {
        out.push_str(&format!("({hidden} versteckte oder gesperrte Einträge nicht gezeigt)\n"));
    }
    Ok(out)
}

fn size(n: u64) -> String {
    if n >= 1024 * 1024 { format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)) } else { format!("{} KB", n.div_ceil(1024)) }
}

fn read(places: &Places, raw: &str) -> Result<String> {
    use std::io::Read as _;
    let file = resolve_existing(places, raw)?;
    let meta = std::fs::metadata(&file).map_err(|e| Error::io(file.display(), &e))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{} ist keine Datei", file.display())));
    }
    let is_pdf = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    let cap = if is_pdf { MAX_PDF } else { MAX_READ };
    if is_pdf && meta.len() > MAX_PDF {
        return Err(Error::invalid(format!("Die PDF ist größer als {} MB", MAX_PDF / (1024 * 1024))));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&file).and_then(|f| f.take(cap).read_to_end(&mut bytes)).map_err(|e| Error::io(file.display(), &e))?;
    let (text, cut) = if is_pdf {
        let t = crate::attach::pdf_text::text(&bytes)?;
        let cut = t.len() > MAX_READ as usize;
        (t.chars().take(MAX_READ as usize).collect::<String>(), cut)
    } else {
        if bytes.iter().take(8192).any(|b| *b == 0) {
            return Err(Error::unsupported(format!("{} ist keine Textdatei", file.display())));
        }
        (String::from_utf8_lossy(&bytes).into_owned(), meta.len() > MAX_READ)
    };
    let more = if cut { "\n… (gekürzt)" } else { "" };
    Ok(format!("Datei {}:\n\n{text}{more}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_and_programs() {
        for n in
            ["id_rsa", "id_ed25519.pub", "server.pem", "Passwörter.kdbx", ".env", "prod.env", "aws-credentials.csv", "my-service-account.json", "wallet.dat"]
        {
            assert!(secret_name(n), "{n}");
        }
        for n in ["bericht.pdf", "envelope.txt", "Notizen.md"] {
            assert!(!secret_name(n), "{n}");
        }
        assert!(program_name("start.sh") && program_name("x.DESKTOP") && !program_name("bericht.md"));
    }

    #[test]
    fn tilde_and_relative_paths() {
        let home = Path::new("/home/a");
        assert_eq!(expand("~/Downloads", Some(home)).unwrap(), Path::new("/home/a/Downloads"));
        assert_eq!(expand("~", Some(home)).unwrap(), home);
        assert!(expand("Downloads/x", Some(home)).is_err());
        assert!(expand("", Some(home)).is_err());
        assert!(expand("/a\u{0}b", Some(home)).is_err());
    }
}
