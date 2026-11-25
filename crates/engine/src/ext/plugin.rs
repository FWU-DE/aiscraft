//! Plug-ins in the Agent Plugins 1.0 format (#28, agent-plugins.org/specification), as n8n's Nod
//! uses them: `plugin.json` (manifest), `mcp.json` (MCP servers) and `skills/<name>/SKILL.md`.
//! A broken manifest refuses the plug-in; a broken server or skill is left out and named, and
//! the rest still loads (the spec's failure boundaries).

use std::collections::BTreeMap;
use std::path::Path;

use aiscraft_core::{Error, Result};
use serde_json::Value;

use super::{Parsed, PluginSkill, ServerSpec, SpecTransport, package, rules_of, text};

pub(crate) const PLUGIN_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
pub(crate) const MCP_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
const MAX_JSON: u64 = 1024 * 1024;
const MAX_SERVERS: usize = 10;
const MAX_SKILLS: usize = 30;
const MAX_SKILL_FILE: u64 = 64 * 1024;
const MAX_SKILL_BODY: usize = 8000;
const ROOT_FIELDS: &[&str] = &["$schema", "name", "version", "description", "author", "homepage", "repository", "license", "keywords", "extensions"];

/// `a-z`, `0-9`, `-`, `.`; letters or digits at both ends; no `--` or `..`; 1–64 characters.
pub(crate) fn valid_name(n: &str) -> bool {
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    (1..=64).contains(&n.len())
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && edge(n.chars().next())
        && edge(n.chars().last())
        && !n.contains("--")
        && !n.contains("..")
}

/// Read and check a plug-in folder.
pub(crate) fn parse(root: &Path) -> Result<Parsed> {
    let m = super::read_json(&root.join("plugin.json"), MAX_JSON)?;
    let Some(obj) = m.as_object() else { return Err(Error::invalid("plugin.json ist kein JSON-Objekt")) };
    if m.get("$schema").and_then(Value::as_str) != Some(PLUGIN_SCHEMA) {
        return Err(Error::invalid(format!("plugin.json nennt nicht das Schema {PLUGIN_SCHEMA}")));
    }
    let name = m.get("name").and_then(Value::as_str).unwrap_or("");
    if !valid_name(name) {
        return Err(Error::invalid("Der Name in plugin.json ist ungültig (a-z, 0-9, - und ., höchstens 64 Zeichen)"));
    }
    let mut warnings = Vec::new();
    let unknown: Vec<&str> = obj.keys().map(String::as_str).filter(|k| !ROOT_FIELDS.contains(k)).take(5).collect();
    if !unknown.is_empty() {
        warnings.push(format!("Unbekannte Angaben in plugin.json übergangen: {}", unknown.join(", ")));
    }
    for key in ["version", "description", "homepage", "repository", "license"] {
        if m.get(key).is_some_and(|v| !v.is_string()) {
            return Err(Error::invalid(format!("plugin.json: {key} muss Text sein")));
        }
    }
    if let Some(a) = m.get("author") {
        let ok = a.as_object().is_some_and(|o| o.iter().all(|(k, v)| matches!(k.as_str(), "name" | "email" | "url") && v.is_string()));
        if !ok {
            return Err(Error::invalid("plugin.json: author darf nur name, email und url als Text haben"));
        }
    }
    if m.get("keywords").is_some_and(|k| !k.as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
        return Err(Error::invalid("plugin.json: keywords muss eine Liste von Wörtern sein"));
    }
    let extensions = match m.get("extensions") {
        Some(e) if e.is_object() => e.clone(),
        Some(_) => {
            warnings.push("plugin.json: extensions ist kein Objekt und wird übergangen".into());
            Value::Null
        }
        None => Value::Null,
    };
    let mut display_name = name.to_string();
    // Nod's app profile: only the display name is taken from it.
    if let Some(app) = extensions.pointer("/io.n8n.nod/app").and_then(Value::as_str)
        && let Some(path) = app.strip_prefix("./").and_then(|rel| package::inside(root, rel))
        && let Ok(v) = super::read_json(&path, MAX_JSON)
        && let Some(d) = text(&v, "displayName")
    {
        display_name = d.lines().next().unwrap_or(name).chars().take(80).collect();
    }
    let servers = servers(root, &mut warnings);
    let skills = skills(root, &mut warnings);
    if servers.is_empty() && skills.is_empty() {
        return Err(Error::invalid(format!(
            "Das Plug-in bringt weder Werkzeuge (mcp.json) noch Fähigkeiten (skills/) mit, die AIS.ChatCraft laden kann.{}",
            if warnings.is_empty() { String::new() } else { format!(" {}", warnings.join(" ")) }
        )));
    }
    Ok(Parsed {
        kind: "plugin",
        name: name.to_string(),
        display_name,
        version: text(&m, "version").unwrap_or_default(),
        description: text(&m, "description").unwrap_or_default(),
        author: m.pointer("/author/name").and_then(Value::as_str).unwrap_or_default().to_string(),
        homepage: text(&m, "homepage").filter(|h| h.starts_with("https://")),
        servers,
        fields: Vec::new(),
        tools: Vec::new(),
        skills,
        rules: rules_of(extensions.get("ai.uds.aischatcraft")),
        privacy: Vec::new(),
        warnings,
    })
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        None => Some(Vec::new()),
        Some(Value::Array(a)) => a.iter().take(200).map(|x| x.as_str().map(str::to_string)).collect(),
        Some(_) => None,
    }
}

fn string_map(v: Option<&Value>) -> Option<BTreeMap<String, String>> {
    match v {
        None => Some(BTreeMap::new()),
        Some(Value::Object(o)) => o.iter().take(100).map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
        Some(_) => None,
    }
}

/// `mcp.json`: each server checked on its own; a bad one is named and left out.
fn servers(root: &Path, warnings: &mut Vec<String>) -> Vec<ServerSpec> {
    let path = root.join("mcp.json");
    if !path.exists() {
        return Vec::new();
    }
    if package::inside(root, "mcp.json").is_none() {
        warnings.push("mcp.json liegt außerhalb des Plug-ins und wird übergangen".into());
        return Vec::new();
    }
    let m = match super::read_json(&path, MAX_JSON) {
        Ok(m) => m,
        Err(e) => {
            warnings.push(format!("Werkzeuge aus: {}", e.message));
            return Vec::new();
        }
    };
    if m.get("$schema").and_then(Value::as_str) != Some(MCP_SCHEMA) {
        warnings.push("mcp.json nennt nicht das Schema von Agent Plugins 1.0.0; Werkzeuge bleiben aus".into());
        return Vec::new();
    }
    if m.as_object().is_some_and(|o| o.keys().any(|k| k != "$schema" && k != "mcpServers")) {
        warnings.push("mcp.json hat unbekannte Angaben; Werkzeuge bleiben aus".into());
        return Vec::new();
    }
    let Some(list) = m.get("mcpServers").and_then(Value::as_object) else {
        warnings.push("mcp.json: mcpServers fehlt".into());
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, s) in list.iter().take(MAX_SERVERS) {
        match server(root, name, s) {
            Ok(spec) => out.push(spec),
            Err(why) => warnings.push(format!("Server „{}“ übergangen: {why}", name.chars().take(64).collect::<String>())),
        }
    }
    if list.len() > MAX_SERVERS {
        warnings.push(format!("Nur die ersten {MAX_SERVERS} Server geladen"));
    }
    out
}

fn server(root: &Path, name: &str, s: &Value) -> std::result::Result<ServerSpec, String> {
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err("ungültiger Name".into());
    }
    let o = s.as_object().ok_or("kein Objekt")?;
    let kind = o.get("type").and_then(Value::as_str).ok_or("type fehlt")?;
    let allowed: &[&str] = match kind {
        "stdio" => &["type", "command", "args", "env", "cwd"],
        "streamable-http" | "sse" => &["type", "url", "headers"],
        _ => return Err(format!("Art {kind} gibt es nicht")),
    };
    if let Some(k) = o.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("unbekannte Angabe {k}"));
    }
    let transport = match kind {
        "stdio" => {
            let command = o.get("command").and_then(Value::as_str).ok_or("command fehlt")?;
            let bare = !command.contains('/') && !command.contains('\\');
            if !bare && !command.starts_with("./") {
                return Err("command muss ein Programmname oder ein ./-Pfad im Plug-in sein".into());
            }
            package::resolve_command(root, command).map_err(|e| e.message)?;
            let args = strings(o.get("args")).ok_or("args muss eine Liste von Texten sein")?;
            let env = string_map(o.get("env")).ok_or("env muss Text-Werte haben")?;
            if env.keys().any(|k| k == "PLUGIN_ROOT" || k == "PLUGIN_DATA") {
                return Err("env darf PLUGIN_ROOT und PLUGIN_DATA nicht setzen".into());
            }
            let cwd = match o.get("cwd") {
                None => None,
                Some(Value::String(c)) if c.starts_with("./") || c.starts_with("${PLUGIN_ROOT}") || c.starts_with("${PLUGIN_DATA}") => {
                    if c.split(['/', '\\']).any(|p| p == "..") {
                        return Err("cwd verlässt das Plug-in".into());
                    }
                    Some(c.clone())
                }
                Some(_) => return Err("cwd muss mit ./, ${PLUGIN_ROOT} oder ${PLUGIN_DATA} beginnen".into()),
            };
            SpecTransport::Stdio { command: command.to_string(), args, env, cwd }
        }
        "sse" => return Err("das alte HTTP+SSE wird nicht unterstützt".into()),
        _ => {
            let url = o.get("url").and_then(Value::as_str).ok_or("url fehlt")?;
            check_url(url)?;
            let headers = string_map(o.get("headers")).ok_or("headers muss Text-Werte haben")?;
            SpecTransport::Http { url: url.to_string(), headers }
        }
    };
    Ok(ServerSpec { name: name.to_string(), transport })
}

/// Absolute http(s), no user info, no fragment; HTTPS unless it is this computer.
pub(crate) fn check_url(url: &str) -> std::result::Result<(), String> {
    let bad = || "url muss eine http(s)-Adresse ohne Zugangsdaten und # sein".to_string();
    let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
    if !matches!(scheme, "https" | "http") || url.contains('#') || url.chars().any(|c| c.is_control() || c.is_whitespace()) || url.len() > 2048 {
        return Err(bad());
    }
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(bad());
    }
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    if scheme == "http" && !loopback {
        return Err("url muss https verwenden".into());
    }
    Ok(())
}

/// `skills/<name>/SKILL.md` (Agent Skills): name and description from the front matter.
fn skills(root: &Path, warnings: &mut Vec<String>) -> Vec<PluginSkill> {
    let Some(dir) = package::inside(root, "skills").filter(|d| d.is_dir()) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).take(500).collect();
    names.sort();
    let mut out = Vec::new();
    for n in names {
        let rel = format!("skills/{n}/SKILL.md");
        let Some(file) = package::inside(root, &rel).filter(|f| f.is_file()) else { continue };
        if out.len() >= MAX_SKILLS {
            warnings.push(format!("Nur die ersten {MAX_SKILLS} Fähigkeiten geladen"));
            break;
        }
        match skill(&file, &n) {
            Ok(s) => out.push(s),
            Err(why) => warnings.push(format!("Fähigkeit „{}“ übergangen: {why}", n.chars().take(64).collect::<String>())),
        }
    }
    out
}

fn skill(file: &Path, dir_name: &str) -> std::result::Result<PluginSkill, String> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    std::fs::File::open(file).and_then(|f| f.take(MAX_SKILL_FILE + 1).read_to_end(&mut buf)).map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_SKILL_FILE {
        return Err("SKILL.md ist größer als 64 KB".into());
    }
    let text = String::from_utf8(buf).map_err(|_| "SKILL.md ist kein UTF-8-Text".to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let rest = text.strip_prefix("---").ok_or("Kopfzeilen (---) fehlen")?;
    let (head, body) = rest.split_once("\n---").ok_or("Kopfzeilen werden nicht geschlossen")?;
    let field = |key: &str| {
        head.lines().find_map(|l| l.strip_prefix(key).and_then(|r| r.trim_start().strip_prefix(':')).map(|v| v.trim().trim_matches(['"', '\'']).to_string()))
    };
    let name = field("name").ok_or("name fehlt")?;
    let description = field("description").filter(|d| !d.is_empty()).ok_or("description fehlt")?;
    if !valid_name(&name) || name.contains('.') || name != dir_name {
        return Err("name muss dem Ordnernamen entsprechen (a-z, 0-9, -)".into());
    }
    let body = body.trim_start_matches(['-', '\r', '\n']).trim();
    Ok(PluginSkill { name, description: description.chars().take(1024).collect(), body: body.chars().take(MAX_SKILL_BODY).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_names() {
        for ok in ["kalender", "my-plugin.v2", "a", "a1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", "A", "a--b", "a..b", "a_b", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn server_addresses() {
        assert!(check_url("https://mcp.example.org/mcp").is_ok());
        assert!(check_url("http://127.0.0.1:8080/mcp").is_ok());
        assert!(check_url("http://mcp.example.org/mcp").is_err());
        assert!(check_url("https://user:pw@example.org/").is_err());
        assert!(check_url("https://example.org/#x").is_err());
        assert!(check_url("file:///etc/passwd").is_err());
    }
}
