//! What reaches beyond AIS.ChatCraft's own data: local files (#25), connector packages (`.mcpb`,
//! #43) and plug-ins (#28), and connector views (#74). Everything from there is untrusted.
//!
//! Installing is two steps, so nothing runs before the person has seen it: `extension.inspect`
//! unpacks a package into a staging folder, checks it and shows what it would do (programs it
//! starts, addresses it reaches, folders and secrets it gets, its settings); `extension.install`
//! with the person's settings moves it into `<data>/plugins/<name>` and only then starts the
//! declared server. Its tools are connector tools: read-only ones run, everything else follows
//! the approval rules (`tools.rs`). Rules a package brings along only ever forbid.

pub(crate) mod mcpb;
pub(crate) mod package;
pub(crate) mod plugin;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use aiscraft_core::{Error, Result};
use aiscraft_mcp::{ServerConfig, Transport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::req_str;
use crate::{Ctx, NoticeLevel, new_id};

const KEY: &str = "extensions";
const MAX_EXTENSIONS: usize = 50;
const STAGING: &str = ".staging";

/// Runtime state of the extensions, held by the engine.
pub(crate) struct Runtime {
    /// The app's data folder; `None` for engines in memory (tests, scripts).
    pub(crate) data_dir: Option<PathBuf>,
    /// `<data>/plugins`: one folder per installed package or plug-in.
    pub(crate) dir: PathBuf,
    /// Test hook: where local files may work instead of the home folder and external drives.
    local_roots: Mutex<Option<Vec<PathBuf>>>,
    /// The package being looked at: its staging folder and what it says.
    offer: Mutex<Option<(String, PathBuf, Parsed)>>,
}

impl Runtime {
    pub(crate) fn new(data_dir: Option<PathBuf>) -> Self {
        let dir = match &data_dir {
            Some(d) => d.join("plugins"),
            None => std::env::temp_dir().join(format!("aiscraft-plugins-{}-{}", std::process::id(), new_id())),
        };
        Self { data_dir, dir, local_roots: Mutex::new(None), offer: Mutex::new(None) }
    }

    pub(crate) fn local_roots(&self) -> Option<Vec<PathBuf>> {
        self.local_roots.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[cfg(test)]
    pub(crate) fn set_local_roots(&self, roots: Vec<PathBuf>) {
        *self.local_roots.lock().unwrap_or_else(PoisonError::into_inner) = Some(roots);
    }
}

// ---- what a package says ---------------------------------------------------------------------

/// One setting a package asks for (MCPB `user_config`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub title: String,
    pub description: String,
    /// "string", "number", "boolean", "directory" or "file".
    pub kind: String,
    pub required: bool,
    /// Kept in the system keychain, never shown again.
    pub sensitive: bool,
    /// Several folders or files, one per line.
    pub multiple: bool,
    pub default: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// A rule a package brings along. Only prohibitions are taken over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageRule {
    /// "forbid" or "allow".
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SpecTransport {
    /// A program, with `${…}` placeholders filled in at start.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// One MCP server of a package, as declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ServerSpec {
    /// The server's name in `mcp.json`; empty for the one server of a bundle.
    pub name: String,
    pub transport: SpecTransport,
}

/// A skill of a plug-in (`skills/<name>/SKILL.md`): only ever a hint for the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginSkill {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub body: String,
}

/// A package, read and checked.
#[derive(Debug, Clone)]
pub(crate) struct Parsed {
    pub kind: &'static str,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub homepage: Option<String>,
    pub servers: Vec<ServerSpec>,
    pub fields: Vec<ConfigField>,
    pub tools: Vec<String>,
    pub skills: Vec<PluginSkill>,
    pub rules: Vec<PackageRule>,
    pub privacy: Vec<String>,
    pub warnings: Vec<String>,
}

/// What the install dialog shows before anything runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionOffer {
    pub id: String,
    /// "bundle" (`.mcpb`) or "plugin" (Agent Plugins).
    pub kind: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub homepage: Option<String>,
    /// What it may do, in plain words.
    pub permissions: Vec<String>,
    pub warnings: Vec<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub fields: Vec<ConfigField>,
    pub rules: Vec<PackageRule>,
    /// A package of that name is installed and would be replaced.
    pub update: bool,
}

/// An installed package or plug-in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Installed {
    pub name: String,
    pub kind: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    /// Settings that are no secrets.
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
    pub servers: Vec<ServerSpec>,
    #[serde(default)]
    pub skills: Vec<PluginSkill>,
    #[serde(default)]
    pub rule_ids: Vec<String>,
    pub installed_at: i64,
}

/// An installed package as the settings show it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionView {
    pub name: String,
    pub kind: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    /// The connectors it brings (ids in `State::connectors`).
    pub connectors: Vec<String>,
    pub skills: Vec<String>,
    /// Found in the plug-in folder but never switched on: it has not run yet.
    pub found: bool,
}

/// The extensions part of the state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionsState {
    pub installed: Vec<ExtensionView>,
    /// The package being looked at in the install dialog.
    pub offer: Option<ExtensionOffer>,
    /// `<data>/plugins`, for people who drop plug-ins in by hand.
    pub folder: String,
}

// ---- small helpers ---------------------------------------------------------------------------

pub(crate) fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| s.chars().filter(|c| !c.is_control() || *c == '\n').take(2000).collect::<String>())
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn read_json(path: &Path, max: u64) -> Result<Value> {
    use std::io::Read as _;
    let shown = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::symlink_metadata(path).map_err(|_| Error::invalid(format!("{shown} fehlt")))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{shown} ist keine Datei")));
    }
    let mut buf = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(max + 1).read_to_end(&mut buf)).map_err(|e| Error::io(&shown, &e))?;
    if buf.len() as u64 > max {
        return Err(Error::invalid(format!("{shown} ist zu groß")));
    }
    serde_json::from_slice(&buf).map_err(|e| Error::invalid(format!("{shown} ist kein gültiges JSON: {e}")))
}

/// Rules from `{"rules": [{"kind": "forbid", "text": "…"}]}`.
pub(crate) fn rules_of(v: Option<&Value>) -> Vec<PackageRule> {
    v.and_then(|x| x.get("rules"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(20)
                .filter_map(|r| {
                    let kind = text(r, "kind")?;
                    let t: String = text(r, "text")?.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
                    matches!(kind.as_str(), "forbid" | "allow").then_some(PackageRule { kind, text: t })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The folder name for a package: lower-case letters, digits, `-` and `.`.
fn slug(name: &str) -> Result<String> {
    let mut s: String = name.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s: String = s.trim_matches(|c| c == '-' || c == '.').chars().take(64).collect();
    if s.is_empty() || s.starts_with('.') || s.contains("..") {
        return Err(Error::invalid("Der Name des Pakets taugt nicht als Ordnername"));
    }
    Ok(s)
}

/// The connector id of a package's server (stable across restarts).
fn connector_id(name: &str, server: &str) -> String {
    use sha1::Digest;
    let d = sha1::Sha1::digest(format!("{name}/{server}").as_bytes());
    format!("x{}", d.iter().take(5).map(|b| format!("{b:02x}")).collect::<String>())
}

fn records(c: &Ctx) -> Vec<Installed> {
    c.store(|st| Ok(st.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

fn save_records(c: &Ctx, all: &[Installed]) -> Result<()> {
    c.store(|st| st.set_setting(KEY, &serde_json::to_value(all)?))?;
    refresh_views(c, all);
    Ok(())
}

fn secret_key(name: &str) -> String {
    format!("extension:{name}")
}

fn secrets_of(c: &Ctx, name: &str) -> BTreeMap<String, Value> {
    c.0.secrets.get(&secret_key(name)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn connector_name(rec: &Installed, server: &ServerSpec) -> String {
    if rec.servers.len() > 1 && !server.name.is_empty() { format!("{} · {}", rec.display_name, server.name) } else { rec.display_name.clone() }
}

fn refresh_views(c: &Ctx, all: &[Installed]) {
    let found: Vec<String> = discover(c, all).into_iter().map(|(n, _)| n).collect();
    let mut views: Vec<ExtensionView> = all
        .iter()
        .map(|r| ExtensionView {
            name: r.name.clone(),
            kind: r.kind.clone(),
            display_name: r.display_name.clone(),
            version: r.version.clone(),
            description: r.description.clone(),
            author: r.author.clone(),
            enabled: r.enabled,
            connectors: r.servers.iter().map(|s| connector_id(&r.name, &s.name)).collect(),
            skills: r.skills.iter().map(|s| s.name.clone()).collect(),
            found: false,
        })
        .collect();
    views.extend(found.into_iter().map(|name| ExtensionView { display_name: name.clone(), name, kind: "plugin".into(), found: true, ..Default::default() }));
    let folder = c.0.ext.dir.display().to_string();
    c.update(|s| {
        s.extensions.installed = views;
        s.extensions.folder = folder;
    });
}

/// Plug-in folders someone put into `<data>/plugins` by hand that are not set up yet.
fn discover(c: &Ctx, all: &[Installed]) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(&c.0.ext.dir) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .take(500)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let ok = !name.starts_with('.') && path.is_dir() && path.join("plugin.json").is_file() && !all.iter().any(|r| r.name == name);
            ok.then_some((name, path))
        })
        .take(MAX_EXTENSIONS)
        .collect();
    out.sort();
    out
}

// ---- reading a package -----------------------------------------------------------------------

/// Which format the unpacked folder is, and where its root is (a ZIP may wrap it in one folder).
fn parse_root(staged: &Path) -> Result<(PathBuf, Parsed)> {
    let mut root = staged.to_path_buf();
    if !root.join("manifest.json").exists() && !root.join("plugin.json").exists() {
        let dirs: Vec<PathBuf> =
            std::fs::read_dir(staged).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).take(3).collect()).unwrap_or_default();
        if let [only] = dirs.as_slice() {
            root = only.clone();
        }
    }
    if root.join("manifest.json").exists() {
        Ok((root.clone(), mcpb::parse(&root)?))
    } else if root.join("plugin.json").exists() {
        Ok((root.clone(), parse_plugin(&root)?))
    } else {
        Err(Error::invalid("Weder ein MCP-Paket (manifest.json) noch ein Plug-in (plugin.json)"))
    }
}

/// Agent Plugins (#28).
fn parse_plugin(root: &Path) -> Result<Parsed> {
    plugin::parse(root)
}

/// What the person reads before saying yes.
fn permissions(p: &Parsed) -> Vec<String> {
    let mut out = Vec::new();
    for s in &p.servers {
        match &s.transport {
            SpecTransport::Stdio { command, args, env, .. } => {
                let shown = |x: &str| x.replace("${__dirname}", ".").replace("${PLUGIN_ROOT}", ".");
                let line: String =
                    std::iter::once(shown(command)).chain(args.iter().map(|a| shown(a))).collect::<Vec<_>>().join(" ").chars().take(300).collect();
                out.push(format!("Startet ein Programm auf diesem Rechner, mit deinen Rechten: {line}"));
                if !env.is_empty() {
                    out.push(format!("Bekommt die Umgebungsvariablen {}", env.keys().cloned().collect::<Vec<_>>().join(", ")));
                }
            }
            SpecTransport::Http { url, .. } => out.push(format!("Verbindet sich mit {url}")),
        }
    }
    for f in &p.fields {
        match (f.kind.as_str(), f.sensitive) {
            ("directory", _) => out.push(format!("Bekommt Zugriff auf die Ordner, die du bei „{}“ wählst", f.title)),
            ("file", _) => out.push(format!("Bekommt Zugriff auf die Datei, die du bei „{}“ wählst", f.title)),
            (_, true) => out.push(format!("Bekommt „{}“ (im Schlüsselbund gespeichert)", f.title)),
            _ => {}
        }
    }
    if !p.skills.is_empty() {
        out.push(format!("Bringt {} Fähigkeiten als Hinweise für das Modell mit", p.skills.len()));
    }
    for u in &p.privacy {
        out.push(format!("Datenschutz des Dienstes: {u}"));
    }
    out
}

/// `extension.inspect`: unpack and check a package; nothing runs.
pub(crate) async fn inspect(c: Ctx, p: Value) -> Result<Value> {
    let path = PathBuf::from(req_str(&p, "path")?);
    cancel_offer(&c);
    let id = new_id();
    let staged = c.0.ext.dir.join(STAGING).join(&id);
    let result = (|| -> Result<(PathBuf, Parsed, Vec<String>)> {
        let meta = std::fs::metadata(&path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
        let skipped = if meta.is_dir() {
            package::copy_folder(&path, &staged)?
        } else {
            if meta.len() > package::MAX_PACKAGE_BYTES {
                return Err(Error::invalid(format!("Das Paket ist größer als {} MB", package::MAX_PACKAGE_BYTES / (1024 * 1024))));
            }
            let bytes = std::fs::read(&path).map_err(|e| Error::io(path.display(), &e))?;
            package::unzip(&bytes, &staged)?;
            Vec::new()
        };
        let (root, parsed) = parse_root(&staged)?;
        Ok((root, parsed, skipped))
    })();
    let (root, mut parsed, skipped) = match result {
        Ok(x) => x,
        Err(e) => {
            package::remove(&staged);
            return Err(e);
        }
    };
    if !skipped.is_empty() {
        parsed.warnings.push(format!("Verknüpfungen nach außerhalb ausgelassen: {}", skipped.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
    }
    let name = slug(&parsed.name)?;
    let all = records(&c);
    let update = all.iter().any(|r| r.name == name);
    if !update && all.len() >= MAX_EXTENSIONS {
        package::remove(&staged);
        return Err(Error::invalid(format!("Höchstens {MAX_EXTENSIONS} Erweiterungen; entferne erst eine")));
    }
    let mut warnings = vec!["Das Programm läuft ohne Abschottung mit deinen Rechten. Installiere nur Pakete aus Quellen, denen du vertraust.".to_string()];
    warnings.append(&mut parsed.warnings.clone());
    if parsed.rules.iter().any(|r| r.kind == "allow") {
        warnings.push("Erlaubnis-Regeln des Pakets werden nicht übernommen; was ohne Nachfrage laufen darf, legst du selbst fest.".into());
    }
    let offer = ExtensionOffer {
        id: id.clone(),
        kind: parsed.kind.to_string(),
        name,
        display_name: parsed.display_name.chars().take(80).collect(),
        version: parsed.version.chars().take(40).collect(),
        description: parsed.description.chars().take(1000).collect(),
        author: parsed.author.chars().take(120).collect(),
        homepage: parsed.homepage.clone(),
        permissions: permissions(&parsed),
        warnings,
        tools: parsed.tools.clone(),
        skills: parsed.skills.iter().map(|s| s.name.clone()).collect(),
        fields: parsed.fields.clone(),
        rules: parsed.rules.clone(),
        update,
    };
    *c.0.ext.offer.lock().unwrap_or_else(PoisonError::into_inner) = Some((id, root, parsed));
    c.update(|s| s.extensions.offer = Some(offer.clone()));
    Ok(serde_json::to_value(offer)?)
}

fn cancel_offer(c: &Ctx) {
    let old = c.0.ext.offer.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some((id, _, _)) = old {
        package::remove(&c.0.ext.dir.join(STAGING).join(id));
    }
    c.update(|s| s.extensions.offer = None);
}

/// `extension.cancel`: close the install dialog; the staged files go.
pub(crate) async fn cancel(c: Ctx, _p: Value) -> Result<Value> {
    cancel_offer(&c);
    Ok(json!({}))
}

/// Check the person's settings against the fields: (plain values, secrets).
fn check_config(fields: &[ConfigField], given: &serde_json::Map<String, Value>) -> Result<(BTreeMap<String, Value>, BTreeMap<String, Value>)> {
    let (mut plain, mut secret) = (BTreeMap::new(), BTreeMap::new());
    for f in fields {
        let v = given.get(&f.key).cloned().filter(|v| !v.is_null() && v != &json!("") && v != &json!([])).unwrap_or_else(|| f.default.clone());
        let v = match (f.kind.as_str(), v) {
            (_, Value::Null) if f.required => return Err(Error::invalid(format!("„{}“ fehlt", f.title))),
            (_, Value::Null) => continue,
            ("boolean", v) => Value::Bool(v.as_bool().or_else(|| v.as_str().map(|s| s == "true")).unwrap_or(false)),
            ("number", v) => {
                let n = v
                    .as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().replace(',', ".").parse().ok()))
                    .ok_or_else(|| Error::invalid(format!("„{}“ muss eine Zahl sein", f.title)))?;
                if f.min.is_some_and(|m| n < m) || f.max.is_some_and(|m| n > m) || !n.is_finite() {
                    return Err(Error::invalid(format!("„{}“ liegt außerhalb des erlaubten Bereichs", f.title)));
                }
                json!(n)
            }
            ("directory" | "file", v) => {
                let list: Vec<String> = match v {
                    Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                    Value::String(s) => s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect(),
                    _ => Vec::new(),
                };
                let mut out = Vec::new();
                for raw in list.iter().take(20) {
                    out.push(checked_place(f, &expand_home(raw))?);
                }
                if out.is_empty() && f.required {
                    return Err(Error::invalid(format!("„{}“ fehlt", f.title)));
                }
                if f.multiple { json!(out) } else { json!(out.into_iter().next().unwrap_or_default()) }
            }
            (_, v) => {
                let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                if s.chars().count() > 4000 || s.contains('\0') {
                    return Err(Error::invalid(format!("„{}“ ist zu lang", f.title)));
                }
                json!(s)
            }
        };
        if f.sensitive {
            secret.insert(f.key.clone(), v)
        } else {
            plain.insert(f.key.clone(), v)
        };
    }
    Ok((plain, secret))
}

fn expand_home(raw: &str) -> String {
    let home = directories::BaseDirs::new().map(|d| d.home_dir().display().to_string()).unwrap_or_default();
    raw.trim().replace("${HOME}", &home).replacen('~', &home, usize::from(raw.trim().starts_with('~')))
}

/// A folder or file the person gives a package: it must exist, and never be a place local files
/// keeps secret (hidden folders, keys, password stores).
fn checked_place(f: &ConfigField, raw: &str) -> Result<String> {
    let real = std::fs::canonicalize(raw).map_err(|e| Error::not_found(format!("„{}“: {raw}: {e}", f.title)))?;
    let ok_kind = if f.kind == "directory" { real.is_dir() } else { real.is_file() };
    if !ok_kind {
        return Err(Error::invalid(format!("„{}“: {} ist kein {}", f.title, real.display(), if f.kind == "directory" { "Ordner" } else { "Datei" })));
    }
    for part in real.components() {
        let name = part.as_os_str().to_string_lossy();
        if name.starts_with('.') || crate::local_files::secret_name(&name) {
            return Err(Error::refused(format!("„{}“: {} enthält Geheimnisse oder ist versteckt und wird keinem Paket gegeben.", f.title, real.display())));
        }
    }
    Ok(real.display().to_string())
}

/// `extension.install`: the person's yes. Moves the package into place, keeps its settings and
/// starts its servers.
pub(crate) async fn install(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let given = p.get("config").and_then(Value::as_object).cloned().unwrap_or_default();
    let (root, parsed) = {
        let guard = c.0.ext.offer.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some((oid, root, parsed)) if *oid == id => (root.clone(), parsed.clone()),
            _ => return Err(Error::not_found("Dieses Paket wird gerade nicht installiert; bitte erneut öffnen")),
        }
    };
    let (plain, secret) = check_config(&parsed.fields, &given)?;
    let name = slug(&parsed.name)?;
    let dest = c.0.ext.dir.join(&name);
    // An update replaces the old copy: its servers stop first.
    let mut all = records(&c);
    if let Some(old) = all.iter().find(|r| r.name == name).cloned() {
        stop(&c, &old).await;
        drop_rules(&c, &old);
        all.retain(|r| r.name != name);
    }
    package::remove(&dest);
    crate::disk::create_dir(&c.0.ext.dir)?;
    std::fs::rename(&root, &dest).map_err(|e| Error::io(dest.display(), &e))?;
    cancel_offer(&c);
    if !secret.is_empty() {
        c.0.secrets.set(&secret_key(&name), Some(&serde_json::to_string(&secret)?))?;
    } else {
        let _ = c.0.secrets.set(&secret_key(&name), None);
    }
    let rec = Installed {
        name: name.clone(),
        kind: parsed.kind.to_string(),
        display_name: parsed.display_name.chars().take(80).collect(),
        version: parsed.version.chars().take(40).collect(),
        description: parsed.description.chars().take(1000).collect(),
        author: parsed.author.chars().take(120).collect(),
        enabled: true,
        config: plain,
        servers: parsed.servers.clone(),
        skills: parsed.skills.clone(),
        rule_ids: Vec::new(),
        installed_at: crate::now_ms(),
    };
    all.push(rec.clone());
    save_records(&c, &all)?;
    let rule_ids = start(&c, &rec, &parsed.rules).await;
    if let Some(r) = all.iter_mut().find(|r| r.name == name) {
        r.rule_ids = rule_ids;
    }
    save_records(&c, &all)?;
    c.notice(NoticeLevel::Success, format!("{} installiert", rec.display_name));
    Ok(json!({"name": name, "connectors": rec.servers.iter().map(|s| connector_id(&name, &s.name)).collect::<Vec<_>>()}))
}

/// Register the package's connectors and connect them; take over its prohibitions.
async fn start(c: &Ctx, rec: &Installed, rules: &[PackageRule]) -> Vec<String> {
    let cfgs = server_configs_of(c, rec);
    c.update(|s| {
        for (cfg, _) in &cfgs {
            s.connectors.retain(|v| v.id != cfg.id);
            let mut v = crate::tools::view_of(cfg);
            v.extension = Some(rec.name.clone());
            s.connectors.push(v);
        }
    });
    let mut ids = Vec::new();
    for rule in rules.iter().filter(|r| r.kind == "forbid") {
        for (cfg, _) in &cfgs {
            if let Ok(v) = crate::tools::rule_add(c.clone(), json!({"kind": "forbid", "text": rule.text, "connector": cfg.id})).await
                && let Some(id) = v.get("id").and_then(Value::as_str)
            {
                ids.push(id.to_string());
            }
        }
    }
    for (cfg, err) in cfgs {
        if let Some(e) = err {
            let id = cfg.id.clone();
            c.update(move |s| {
                if let Some(v) = s.connectors.iter_mut().find(|v| v.id == id) {
                    v.status = "error".into();
                    v.error = Some(e);
                }
            });
            continue;
        }
        if cfg.enabled
            && let Err(e) = crate::tools::connect(c, &cfg).await
        {
            c.notice(NoticeLevel::Warning, format!("{} installiert, aber nicht verbunden: {}", cfg.name, e.message));
        }
    }
    ids
}

async fn stop(c: &Ctx, rec: &Installed) {
    for s in &rec.servers {
        let id = connector_id(&rec.name, &s.name);
        crate::tools::disconnect(c, &id).await;
        c.update(|st| st.connectors.retain(|v| v.id != id));
    }
}

fn drop_rules(c: &Ctx, rec: &Installed) {
    if rec.rule_ids.is_empty() {
        return;
    }
    let mut settings = c.state().settings.clone();
    settings.trust_rules.retain(|r| !rec.rule_ids.contains(&r.id));
    if c.store(|st| st.set_setting("settings", &serde_json::to_value(&settings)?)).is_ok() {
        c.update(|s| s.settings = settings);
    }
}

/// Fill in `${…}`: one pass, unknown placeholders stay as they are. A placeholder that makes up
/// a whole argument and stands for several values (several folders) becomes several arguments.
pub(crate) fn fill(s: &str, vars: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    if let Some(key) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}'))
        && let Some(v) = vars.get(key)
    {
        return v.clone();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(rest.get(..start).unwrap_or(""));
        let after = rest.get(start + 2..).unwrap_or("");
        match after.find('}') {
            Some(end) => {
                let key = after.get(..end).unwrap_or("");
                match vars.get(key) {
                    Some(v) => out.push_str(&v.join(" ")),
                    None => {
                        out.push_str("${");
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    vec![out]
}

fn vars_for(c: &Ctx, rec: &Installed, root: &Path) -> BTreeMap<String, Vec<String>> {
    let mut vars: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let one = |p: Option<PathBuf>| vec![p.map(|p| p.display().to_string()).unwrap_or_default()];
    let user = directories::UserDirs::new();
    let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
    vars.insert("HOME".into(), one(home));
    vars.insert("DESKTOP".into(), one(user.as_ref().and_then(|u| u.desktop_dir().map(Path::to_path_buf))));
    vars.insert("DOCUMENTS".into(), one(user.as_ref().and_then(|u| u.document_dir().map(Path::to_path_buf))));
    vars.insert("DOWNLOADS".into(), one(user.as_ref().and_then(|u| u.download_dir().map(Path::to_path_buf))));
    vars.insert("/".into(), vec![std::path::MAIN_SEPARATOR.to_string()]);
    vars.insert("pathSeparator".into(), vec![std::path::MAIN_SEPARATOR.to_string()]);
    let root_s = root.display().to_string();
    if rec.kind == "bundle" {
        vars.insert("__dirname".into(), vec![root_s]);
        let mut values = rec.config.clone();
        values.extend(secrets_of(c, &rec.name));
        for (k, v) in values {
            let list = match v {
                Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect(),
                Value::String(s) => vec![s],
                other => vec![other.to_string()],
            };
            vars.insert(format!("user_config.{k}"), list);
        }
    } else {
        vars.insert("PLUGIN_ROOT".into(), vec![root_s]);
        vars.insert("PLUGIN_DATA".into(), vec![plugin_data(c, &rec.name).display().to_string()]);
    }
    vars
}

fn plugin_data(c: &Ctx, name: &str) -> PathBuf {
    c.0.ext.dir.join(".data").join(name)
}

/// The MCP server configs of one package, each with why it cannot start (if it cannot).
fn server_configs_of(c: &Ctx, rec: &Installed) -> Vec<(ServerConfig, Option<String>)> {
    let root = c.0.ext.dir.join(&rec.name);
    let vars = vars_for(c, rec, &root);
    rec.servers
        .iter()
        .map(|s| {
            let id = connector_id(&rec.name, &s.name);
            let name = connector_name(rec, s);
            let built = build_transport(c, rec, &root, &vars, &s.transport);
            let (transport, err) = match built {
                Ok(t) => (t, None),
                Err(e) => (Transport::Stdio { command: String::new(), args: vec![], env: Default::default(), cwd: None }, Some(e.message)),
            };
            (ServerConfig { id, name, transport, enabled: rec.enabled && err.is_none(), timeout_secs: None }, err)
        })
        .collect()
}

fn build_transport(c: &Ctx, rec: &Installed, root: &Path, vars: &BTreeMap<String, Vec<String>>, t: &SpecTransport) -> Result<Transport> {
    match t {
        SpecTransport::Http { url, headers } => Ok(Transport::Http { url: url.clone(), headers: headers.clone() }),
        SpecTransport::Stdio { command, args, env, cwd } => {
            // The command itself gets only the package's own folder filled in.
            let cmd_vars: BTreeMap<String, Vec<String>> =
                vars.iter().filter(|(k, _)| matches!(k.as_str(), "__dirname" | "/" | "pathSeparator")).map(|(k, v)| (k.clone(), v.clone())).collect();
            let command = fill(command, &cmd_vars).join(" ");
            let command = package::resolve_command(root, &command)?;
            if Path::new(&command).is_absolute() {
                crate::disk::make_executable(Path::new(&command))?;
            }
            let args: Vec<String> = args.iter().flat_map(|a| fill(a, vars)).take(200).collect();
            let mut env: BTreeMap<String, String> = env.iter().map(|(k, v)| (k.clone(), fill(v, vars).join(" "))).collect();
            let cwd = match cwd {
                Some(raw) => {
                    let d = fill(raw, vars).join(" ");
                    let d = if let Some(rel) = d.strip_prefix("./") { root.join(rel).display().to_string() } else { d };
                    let data = plugin_data(c, &rec.name);
                    crate::disk::create_dir(&data)?;
                    let real = std::fs::canonicalize(&d).map_err(|e| Error::invalid(format!("Arbeitsordner {d}: {e}")))?;
                    let inside = [root.to_path_buf(), data].iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|r| real.starts_with(r));
                    if !inside {
                        return Err(Error::invalid("Der Arbeitsordner des Servers liegt außerhalb des Plug-ins"));
                    }
                    real.display().to_string()
                }
                None => root.display().to_string(),
            };
            if rec.kind == "plugin" {
                let data = plugin_data(c, &rec.name);
                crate::disk::create_dir(&data)?;
                env.insert("PLUGIN_ROOT".into(), root.display().to_string());
                env.insert("PLUGIN_DATA".into(), data.display().to_string());
            }
            Ok(Transport::Stdio { command, args, env, cwd: Some(cwd) })
        }
    }
}

/// Configs of every enabled package's servers, for `tools.rs` to find a connector by id.
pub(crate) fn server_configs(c: &Ctx) -> Vec<ServerConfig> {
    records(c).iter().flat_map(|r| server_configs_of(c, r)).filter(|(_, e)| e.is_none()).map(|(cfg, _)| cfg).collect()
}

/// At start: the installed packages' connectors, connected when switched on.
pub(crate) fn load(c: &Ctx) {
    crate::local_files::load(c);
    // Staging leftovers of a run that ended in the middle of an install.
    package::remove(&c.0.ext.dir.join(STAGING));
    let all = records(c);
    refresh_views(c, &all);
    for rec in all {
        let ctx = c.clone();
        c.spawn(async move {
            let _ = start(&ctx, &rec, &[]).await;
        });
    }
}

/// `extension.toggle`: switch a package on or off. Switching on a plug-in that was dropped into
/// the folder by hand is its install: it is read and checked first.
pub(crate) async fn toggle(c: Ctx, p: Value) -> Result<Value> {
    let name = req_str(&p, "name")?;
    let on = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| Error::invalid("enabled (ja/nein) fehlt"))?;
    let mut all = records(&c);
    if !all.iter().any(|r| r.name == name) {
        let Some((_, path)) = discover(&c, &all).into_iter().find(|(n, _)| *n == name) else {
            return Err(Error::not_found("Erweiterung nicht gefunden"));
        };
        if !on {
            return Ok(json!({}));
        }
        let parsed = parse_plugin(&path)?;
        if parsed.fields.iter().any(|f| f.required) {
            return Err(Error::invalid("Dieses Plug-in braucht Einstellungen; installiere es über „Paket installieren“"));
        }
        let rec = Installed {
            name: name.clone(),
            kind: parsed.kind.to_string(),
            display_name: parsed.display_name.chars().take(80).collect(),
            version: parsed.version.chars().take(40).collect(),
            description: parsed.description.chars().take(1000).collect(),
            author: parsed.author.chars().take(120).collect(),
            enabled: true,
            config: BTreeMap::new(),
            servers: parsed.servers.clone(),
            skills: parsed.skills.clone(),
            rule_ids: Vec::new(),
            installed_at: crate::now_ms(),
        };
        all.push(rec.clone());
        save_records(&c, &all)?;
        let ids = start(&c, &rec, &parsed.rules).await;
        if let Some(r) = all.iter_mut().find(|r| r.name == name) {
            r.rule_ids = ids;
        }
        save_records(&c, &all)?;
        return Ok(json!({}));
    }
    let rec = all.iter_mut().find(|r| r.name == name).ok_or_else(|| Error::not_found("Erweiterung nicht gefunden"))?;
    rec.enabled = on;
    let rec = rec.clone();
    save_records(&c, &all)?;
    stop(&c, &rec).await;
    start(&c, &rec, &[]).await;
    Ok(json!({}))
}

/// `extension.remove`: stop it, delete its folder, settings, secrets and rules.
pub(crate) async fn remove(c: Ctx, p: Value) -> Result<Value> {
    let name = req_str(&p, "name")?;
    let mut all = records(&c);
    let rec = all.iter().find(|r| r.name == name).cloned().ok_or_else(|| Error::not_found("Erweiterung nicht gefunden"))?;
    stop(&c, &rec).await;
    drop_rules(&c, &rec);
    all.retain(|r| r.name != name);
    save_records(&c, &all)?;
    let _ = c.0.secrets.set(&secret_key(&name), None);
    if slug(&name).is_ok_and(|s| s == name) {
        package::remove(&c.0.ext.dir.join(&name));
        package::remove(&plugin_data(&c, &name));
    }
    refresh_views(&c, &all);
    Ok(json!({}))
}

/// Skills of enabled plug-ins, for recall (`skills.rs`).
pub(crate) fn skills(c: &Ctx) -> Vec<(String, PluginSkill)> {
    records(c).into_iter().filter(|r| r.enabled).flat_map(|r| r.skills.into_iter().map(move |s| (r.display_name.clone(), s))).collect()
}

/// `extension.rescan`: read the plug-in folder again (plug-ins dropped in by hand).
pub(crate) async fn rescan(c: Ctx, _p: Value) -> Result<Value> {
    let all = records(&c);
    refresh_views(&c, &all);
    Ok(json!({"found": discover(&c, &all).len()}))
}

pub(crate) static COMMANDS: &[crate::CommandSpec] = &[
    crate::CommandSpec {
        id: "extension.inspect",
        label: "Paket oder Plug-in öffnen",
        group: "Konnektoren",
        shortcut: None,
        params: "path: string (.mcpb-Datei, ZIP oder Plug-in-Ordner). Prüft das Paket und zeigt, was es darf; startet nichts",
        palette: false,
        run: |c, p| Box::pin(inspect(c, p)),
    },
    crate::CommandSpec {
        id: "extension.install",
        label: "Paket installieren",
        group: "Konnektoren",
        shortcut: None,
        params: "id: string (aus extension.inspect), config?: {Schlüssel: Wert} (Einstellungen des Pakets)",
        palette: false,
        run: |c, p| Box::pin(install(c, p)),
    },
    crate::CommandSpec {
        id: "extension.cancel",
        label: "Installation abbrechen",
        group: "Konnektoren",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, p| Box::pin(cancel(c, p)),
    },
    crate::CommandSpec {
        id: "extension.toggle",
        label: "Erweiterung ein/aus",
        group: "Konnektoren",
        shortcut: None,
        params: "name: string, enabled: bool",
        palette: false,
        run: |c, p| Box::pin(toggle(c, p)),
    },
    crate::CommandSpec {
        id: "extension.rescan",
        label: "Plug-in-Ordner neu einlesen",
        group: "Konnektoren",
        shortcut: None,
        params: "",
        palette: false,
        run: |c, p| Box::pin(rescan(c, p)),
    },
    crate::CommandSpec {
        id: "extension.remove",
        label: "Erweiterung entfernen",
        group: "Konnektoren",
        shortcut: None,
        params: "name: string",
        palette: false,
        run: |c, p| Box::pin(remove(c, p)),
    },
];

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn placeholders_are_filled_once() {
        let mut vars = BTreeMap::new();
        vars.insert("__dirname".to_string(), vec!["/p".to_string()]);
        vars.insert("user_config.dirs".to_string(), vec!["/a".to_string(), "/b".to_string()]);
        vars.insert("user_config.evil".to_string(), vec!["${__dirname}".to_string()]);
        assert_eq!(fill("${__dirname}/server/index.js", &vars), vec!["/p/server/index.js"]);
        assert_eq!(fill("${user_config.dirs}", &vars), vec!["/a", "/b"]);
        assert_eq!(fill("--x=${user_config.evil}", &vars), vec!["--x=${__dirname}"], "no second pass");
        assert_eq!(fill("${unbekannt} ${", &vars), vec!["${unbekannt} ${"]);
    }

    #[test]
    fn folder_names() {
        assert_eq!(slug("My MCP Extension").unwrap(), "my-mcp-extension");
        assert_eq!(slug("../../etc").unwrap(), "etc");
        assert!(slug("..").is_err());
        assert!(slug("").is_err());
    }
}

#[cfg(test)]
mod plugin_tests;
#[cfg(test)]
pub(crate) mod tests;
