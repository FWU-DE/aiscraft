//! The vault (as in Nod): logins and TOTP secrets that AIS.ChatCraft fills in itself, so the
//! model never sees them. The model writes placeholders such as `{{vault:schulportal.password}}`
//! into a connector call; the values are put in right before the call (which then always needs
//! the owner's yes) and taken out of the result again. Entries live in the OS keychain.

use aiscraft_core::{Error, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id};

const KEY: &str = "vault";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The placeholder name: lowercase letters, digits, - and _.
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Base32 TOTP secret.
    #[serde(default)]
    pub totp: String,
    #[serde(default)]
    pub url: String,
}

/// What the UI and the model may know about an entry: no secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryInfo {
    pub id: String,
    pub name: String,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    pub url: String,
}

fn info(e: &Entry) -> EntryInfo {
    EntryInfo {
        id: e.id.clone(),
        name: e.name.clone(),
        has_username: !e.username.is_empty(),
        has_password: !e.password.is_empty(),
        has_totp: !e.totp.is_empty(),
        url: e.url.clone(),
    }
}

fn load(c: &Ctx) -> Vec<Entry> {
    c.0.secrets.get(KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(c: &Ctx, all: &[Entry]) -> Result<()> {
    c.0.secrets.set(KEY, Some(&serde_json::to_string(all)?))?;
    let infos: Vec<EntryInfo> = all.iter().map(info).collect();
    c.update(|s| s.vault = infos);
    Ok(())
}

pub(crate) fn refresh(c: &Ctx) {
    let infos: Vec<EntryInfo> = load(c).iter().map(info).collect();
    c.update(|s| s.vault = infos);
}

fn slug(s: &str) -> String {
    let mut out: String = s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(40).collect()
}

// ---- TOTP (RFC 6238, SHA-1, 30 s, 6 digits) ------------------------------------------------

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut out = Vec::new();
    for ch in s.chars().filter(|c| !c.is_whitespace() && *c != '=' && *c != '-') {
        let v = alphabet.iter().position(|&a| a == ch.to_ascii_uppercase() as u8)? as u64;
        bits = (bits << 5) | v;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push(((bits >> nbits) & 0xff) as u8);
        }
        if out.len() > 256 {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

pub fn totp_at(secret_b32: &str, unix_secs: u64) -> Option<String> {
    let key = base32_decode(secret_b32)?;
    let counter = (unix_secs / 30).to_be_bytes();
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    mac.update(&counter);
    let h = mac.finalize().into_bytes();
    let off = (*h.get(19)? & 0x0f) as usize;
    let b = h.get(off..off + 4)?;
    let code = (u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]) & 0x7fff_ffff) % 1_000_000;
    Some(format!("{code:06}"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---- placeholders ---------------------------------------------------------------------------

/// Put vault values into tool arguments. Returns the arguments and the secret values used (to
/// take out of the result again), or an error naming an unknown placeholder.
pub(crate) fn fill(c: &Ctx, args: &Value) -> Result<(Value, Vec<String>)> {
    let text = args.to_string();
    if !text.contains("{{vault:") {
        return Ok((args.clone(), Vec::new()));
    }
    let entries = load(c);
    let mut used = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find("{{vault:") {
        out.push_str(rest.get(..start).unwrap_or(""));
        let after = rest.get(start + 8..).unwrap_or("");
        let Some(end) = after.find("}}") else { return Err(Error::invalid("Platzhalter ohne }}")) };
        let spec = after.get(..end).unwrap_or("");
        let (name, field) = spec.split_once('.').unwrap_or((spec, "password"));
        let e = entries.iter().find(|e| e.name == name).ok_or_else(|| Error::invalid(format!("Tresor-Eintrag „{name}“ gibt es nicht")))?;
        let value = match field {
            "username" | "user" => e.username.clone(),
            "password" => e.password.clone(),
            "totp" => totp_at(&e.totp, now_secs()).ok_or_else(|| Error::invalid(format!("„{name}“ hat kein gültiges TOTP-Geheimnis")))?,
            "url" => e.url.clone(),
            other => return Err(Error::invalid(format!("Feld „{other}“ gibt es nicht (username, password, totp, url)"))),
        };
        if value.is_empty() {
            return Err(Error::invalid(format!("„{name}.{field}“ ist leer")));
        }
        // Inside a JSON string: escape like JSON does.
        let escaped = serde_json::to_string(&value).unwrap_or_default();
        out.push_str(escaped.get(1..escaped.len().saturating_sub(1)).unwrap_or(""));
        if field != "url" {
            used.push(value);
        }
        rest = after.get(end + 2..).unwrap_or("");
    }
    out.push_str(rest);
    let filled = serde_json::from_str(&out).map_err(|e| Error::invalid(format!("Platzhalter: {e}")))?;
    Ok((filled, used))
}

/// Whether `value` is the user name of a vault entry (the browser enters it as the name, not as a
/// password, #107).
pub(crate) fn is_username(c: &Ctx, value: &str) -> bool {
    !value.is_empty() && load(c).iter().any(|e| e.username == value)
}

/// Take secret values out of a tool result before the model sees it.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> String {
    let mut t = text.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 3) {
        t = t.replace(s.as_str(), "[Tresor]");
    }
    t
}

/// A note for the model listing the entries it may refer to.
pub(crate) fn prompt_note(c: &Ctx) -> Option<String> {
    let v = &c.state().vault;
    if v.is_empty() {
        return None;
    }
    let list: Vec<String> = v
        .iter()
        .map(|e| {
            let mut f = Vec::new();
            if e.has_username {
                f.push("username");
            }
            if e.has_password {
                f.push("password");
            }
            if e.has_totp {
                f.push("totp");
            }
            format!("{} ({})", e.name, f.join(", "))
        })
        .collect();
    Some(format!(
        "Tresor: Für Zugangsdaten in Werkzeug-Argumenten schreibe Platzhalter wie {{{{vault:NAME.password}}}}; AIS.ChatCraft setzt die Werte selbst ein, du siehst sie nie. Einträge: {}.",
        list.join("; ")
    ))
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) async fn add(c: Ctx, p: Value) -> Result<Value> {
    let name = slug(&req_str(&p, "name")?);
    if name.is_empty() {
        return Err(Error::invalid("Name fehlt"));
    }
    let totp = opt_str(&p, "totp").unwrap_or_default().replace(' ', "");
    if !totp.is_empty() && base32_decode(&totp).is_none() {
        return Err(Error::invalid("Das TOTP-Geheimnis ist kein Base32"));
    }
    let mut all = load(&c);
    if all.iter().any(|e| e.name == name) {
        return Err(Error::invalid(format!("„{name}“ gibt es schon")));
    }
    let e = Entry {
        id: new_id(),
        name,
        username: opt_str(&p, "username").unwrap_or_default(),
        password: opt_str(&p, "password").unwrap_or_default(),
        totp,
        url: opt_str(&p, "url").unwrap_or_default(),
    };
    let id = e.id.clone();
    all.push(e);
    save(&c, &all)?;
    Ok(json!({"id": id}))
}

pub(crate) async fn remove(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut all = load(&c);
    let before = all.len();
    all.retain(|e| e.id != id);
    if all.len() == before {
        return Err(Error::not_found("Eintrag nicht gefunden"));
    }
    save(&c, &all)?;
    Ok(json!({}))
}

/// The current code, for the owner (never for the model).
pub(crate) async fn code(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let e = load(&c).into_iter().find(|e| e.id == id).ok_or_else(|| Error::not_found("Eintrag nicht gefunden"))?;
    let code = totp_at(&e.totp, now_secs()).ok_or_else(|| Error::invalid("Kein TOTP-Geheimnis"))?;
    Ok(json!({"code": code, "valid_for": 30 - now_secs() % 30}))
}

/// One CSV line into fields (RFC 4180 quoting).
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quoted, ch) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => out.push(std::mem::take(&mut cur)),
            (_, c) => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Import a Bitwarden CSV export (login items).
pub(crate) async fn import_bitwarden(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let meta = std::fs::metadata(&path).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    if meta.len() > 10 * 1024 * 1024 {
        return Err(Error::invalid("Die Datei ist zu groß"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    let mut lines = text.lines();
    let header = csv_fields(lines.next().unwrap_or(""));
    let col = |n: &str| header.iter().position(|h| h.trim() == n);
    let (Some(ci_name), Some(ci_user), Some(ci_pass)) = (col("name"), col("login_username"), col("login_password")) else {
        return Err(Error::invalid("Das ist keine Bitwarden-CSV (Spalten name, login_username, login_password fehlen)"));
    };
    let (ci_totp, ci_uri) = (col("login_totp"), col("login_uri"));
    let mut all = load(&c);
    let mut added = 0;
    for line in lines.take(5000) {
        let f = csv_fields(line);
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).cloned().unwrap_or_default();
        let name = slug(&get(Some(ci_name)));
        if name.is_empty() || all.iter().any(|e| e.name == name) {
            continue;
        }
        let totp = get(ci_totp).trim_start_matches("otpauth://").to_string();
        let totp = if totp.contains("secret=") { totp.split("secret=").nth(1).and_then(|s| s.split('&').next()).unwrap_or("").to_string() } else { totp };
        all.push(Entry {
            id: new_id(),
            name,
            username: get(Some(ci_user)),
            password: get(Some(ci_pass)),
            totp: if base32_decode(&totp).is_some() { totp } else { String::new() },
            url: get(ci_uri),
        });
        added += 1;
    }
    save(&c, &all)?;
    c.notice(NoticeLevel::Success, format!("{added} Einträge in den Tresor übernommen. Die CSV-Datei enthält Passwörter im Klartext; bitte löschen."));
    Ok(json!({"added": added}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_rfc6238_vectors() {
        // RFC 6238 test secret "12345678901234567890" in base32.
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(totp_at(secret, 59).as_deref(), Some("287082"));
        assert_eq!(totp_at(secret, 1111111109).as_deref(), Some("081804"));
        assert_eq!(totp_at(secret, 2000000000).as_deref(), Some("279037"));
        assert!(totp_at("not base32!", 59).is_none());
    }

    #[test]
    fn csv_and_slugs() {
        assert_eq!(csv_fields(r#"a,"b, c","d ""x""",e"#), vec!["a", "b, c", "d \"x\"", "e"]);
        assert_eq!(slug("Schul Portal (NRW)!"), "schul-portal-nrw");
        assert_eq!(scrub("token geheim123 ok", &["geheim123".into()]), "token [Tresor] ok");
    }
}
