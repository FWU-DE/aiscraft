//! MCP bundles (`.mcpb`, formerly Desktop Extensions, #43): a ZIP with `manifest.json` (spec
//! 0.1–0.4, github.com/modelcontextprotocol/mcpb `MANIFEST.md`) and the server's files.

use std::collections::BTreeMap;
use std::path::Path;

use aiscraft_core::{Error, Result};
use serde_json::Value;

use super::{ConfigField, Parsed, ServerSpec, SpecTransport, rules_of, text};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_FIELDS: usize = 30;
const MAX_TOOLS: usize = 200;

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    }
}

/// Read and check `manifest.json` in `root`.
pub(crate) fn parse(root: &Path) -> Result<Parsed> {
    let m = super::read_json(&root.join("manifest.json"), MAX_MANIFEST)?;
    if !m.is_object() {
        return Err(Error::invalid("manifest.json ist kein JSON-Objekt"));
    }
    let version = text(&m, "manifest_version").or_else(|| text(&m, "dxt_version")).unwrap_or_default();
    if !matches!(version.as_str(), "0.1" | "0.2" | "0.3" | "0.4") {
        return Err(Error::unsupported(format!("Diese Paketversion ({}) kennt AIS.ChatCraft nicht", version.chars().take(20).collect::<String>())));
    }
    let name = text(&m, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| Error::invalid("Im Paket fehlt der Name"))?;
    let mut warnings = Vec::new();
    if let Some(platforms) = m.pointer("/compatibility/platforms").and_then(Value::as_array) {
        let ok = platforms.iter().filter_map(Value::as_str).any(|p| p == platform());
        if !ok {
            return Err(Error::unsupported(format!(
                "Das Paket läuft nur auf {} und nicht auf diesem System",
                platforms.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
            )));
        }
    }
    let server = m.get("server").filter(|s| s.is_object()).ok_or_else(|| Error::invalid("Im Paket fehlt die Angabe zum Server"))?;
    let kind = text(server, "type").unwrap_or_default();
    let entry = text(server, "entry_point").unwrap_or_default();
    if !entry.is_empty() && super::package::inside(root, &entry).is_none() {
        return Err(Error::invalid(format!("Der Einstiegspunkt {} liegt nicht im Paket", entry.chars().take(120).collect::<String>())));
    }
    let mut cfg = server.get("mcp_config").cloned().unwrap_or(Value::Null);
    // Platform overrides replace command, args or env for this system.
    if let Some(over) = cfg.pointer(&format!("/platform_overrides/{}", platform())).cloned()
        && let (Some(base), Some(o)) = (cfg.as_object_mut(), over.as_object())
    {
        for (k, v) in o {
            base.insert(k.clone(), v.clone());
        }
    }
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(100).map(str::to_string).collect()).unwrap_or_default()
    };
    let (command, args) = match (text(&cfg, "command"), kind.as_str()) {
        (Some(c), _) => (c, strings(cfg.get("args"))),
        (None, "node") => ("node".to_string(), vec![format!("${{__dirname}}/{entry}")]),
        (None, "python") => ("python3".to_string(), vec![format!("${{__dirname}}/{entry}")]),
        (None, "uv") => ("uv".to_string(), vec!["run".into(), "--directory".into(), "${__dirname}".into(), format!("${{__dirname}}/{entry}")]),
        (None, "binary") => (format!("${{__dirname}}/{entry}"), Vec::new()),
        _ => return Err(Error::invalid("Das Paket sagt nicht, wie der Server startet (mcp_config.command)")),
    };
    // The program must be a bare name or a file inside the package, already now.
    let dirname: BTreeMap<String, Vec<String>> = [("__dirname".to_string(), vec![root.display().to_string()])].into_iter().collect();
    super::package::resolve_command(root, &super::fill(&command, &dirname).join(" "))?;
    let env: BTreeMap<String, String> = cfg
        .get("env")
        .and_then(Value::as_object)
        .map(|o| o.iter().take(100).filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let runtime = match kind.as_str() {
        "node" => Some("Node.js"),
        "python" => Some("Python 3"),
        "uv" => Some("uv (Python)"),
        _ => None,
    };
    if let Some(r) = runtime {
        warnings.push(format!("Braucht {r} auf diesem Rechner."));
    }
    let fields = fields(m.get("user_config"));
    let mut tools: Vec<String> = m
        .get("tools")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_TOOLS).filter_map(|t| text(t, "name")).map(|n| n.chars().take(100).collect()).collect())
        .unwrap_or_default();
    if m.get("tools_generated").and_then(Value::as_bool) == Some(true) {
        tools.push("(weitere, die der Server selbst meldet)".into());
    }
    let mut privacy: Vec<String> = m
        .get("privacy_policies")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter(|u| u.starts_with("https://")).take(10).map(str::to_string).collect())
        .unwrap_or_default();
    privacy.dedup();
    Ok(Parsed {
        kind: "bundle",
        display_name: text(&m, "display_name").unwrap_or_else(|| name.clone()),
        name,
        version: text(&m, "version").unwrap_or_default(),
        description: text(&m, "description").unwrap_or_default(),
        author: m.pointer("/author/name").and_then(Value::as_str).unwrap_or_default().to_string(),
        homepage: text(&m, "homepage").filter(|h| h.starts_with("https://")),
        servers: vec![ServerSpec { name: String::new(), transport: SpecTransport::Stdio { command, args, env, cwd: None } }],
        fields,
        tools,
        skills: Vec::new(),
        rules: rules_of(m.pointer("/_meta/ai.uds.aischatcraft")),
        privacy,
        warnings,
    })
}

/// The settings the manifest asks for (`user_config`).
fn fields(v: Option<&Value>) -> Vec<ConfigField> {
    let Some(o) = v.and_then(Value::as_object) else { return Vec::new() };
    o.iter()
        .take(MAX_FIELDS)
        .filter(|(k, _)| !k.is_empty() && k.len() <= 64 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .map(|(k, f)| {
            let kind = match text(f, "type").as_deref() {
                Some(t @ ("number" | "boolean" | "directory" | "file")) => t.to_string(),
                _ => "string".to_string(),
            };
            ConfigField {
                key: k.clone(),
                title: text(f, "title").unwrap_or_else(|| k.clone()).chars().take(120).collect(),
                description: text(f, "description").unwrap_or_default().chars().take(500).collect(),
                required: f.get("required").and_then(Value::as_bool).unwrap_or(false),
                sensitive: f.get("sensitive").and_then(Value::as_bool).unwrap_or(false),
                multiple: f.get("multiple").and_then(Value::as_bool).unwrap_or(false) && matches!(kind.as_str(), "directory" | "file"),
                default: f.get("default").cloned().unwrap_or(Value::Null),
                min: f.get("min").and_then(Value::as_f64),
                max: f.get("max").and_then(Value::as_f64),
                kind,
            }
        })
        .collect()
}
