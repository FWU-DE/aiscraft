//! The tools of connected apps (#26): Microsoft Graph (Outlook, OneDrive), Google (Gmail,
//! Calendar, Drive) and CalDAV. Each tool reads a little and gives text for the model; the two
//! kinds of writing (a mail draft, a calendar event) exist only for apps connected with „Auch
//! schreiben“ and run only after the owner's yes (`tools.rs`).

use std::time::Duration;

use aiscraft_backends::http;
use aiscraft_backends::oauth::encode;
use aiscraft_core::{Error, Result};
use base64::Engine as _;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};

use super::{Account, AccountSecret, access_token, endpoint, load_secret};
use crate::Ctx;

/// Largest file read as text.
const MAX_FILE: usize = 1024 * 1024;
/// Largest JSON answer.
const MAX_JSON: usize = 4 * 1024 * 1024;
/// Most entries one list shows.
const MAX_LIST: u64 = 25;
/// The longest span of time one calendar request covers.
const MAX_DAYS: i64 = 366;

pub(crate) struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub write: bool,
    pub schema: fn() -> Value,
}

fn s_mail_list() -> Value {
    json!({"type": "object", "properties": {
        "query": {"type": "string", "description": "Suchwörter (Absender, Betreff, Inhalt); leer: die neuesten"},
        "count": {"type": "integer", "minimum": 1, "maximum": MAX_LIST, "description": "wie viele (Standard 10)"}
    }, "additionalProperties": false})
}
fn s_id() -> Value {
    json!({"type": "object", "properties": {"id": {"type": "string", "description": "ID aus der Liste"}}, "required": ["id"], "additionalProperties": false})
}
fn s_draft() -> Value {
    json!({"type": "object", "properties": {
        "to": {"type": "string", "description": "Empfänger, mit Komma getrennt"},
        "subject": {"type": "string"}, "body": {"type": "string", "description": "Text der Mail"}
    }, "required": ["to", "subject", "body"], "additionalProperties": false})
}
fn s_none() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}
fn s_events() -> Value {
    json!({"type": "object", "properties": {
        "from": {"type": "string", "description": "Beginn, z. B. 2026-12-01 oder 2026-12-01T08:00 (Ortszeit); Standard: heute"},
        "to": {"type": "string", "description": "Ende (ausschließlich); Standard: 7 Tage nach Beginn"},
        "calendar": {"type": "string", "description": "Name oder ID eines Kalenders; leer: alle bzw. der Hauptkalender"}
    }, "additionalProperties": false})
}
fn s_event_create() -> Value {
    json!({"type": "object", "properties": {
        "title": {"type": "string"}, "start": {"type": "string", "description": "2026-12-01T10:00 (Ortszeit)"},
        "end": {"type": "string"}, "location": {"type": "string"}
    }, "required": ["title", "start", "end"], "additionalProperties": false})
}
fn s_search() -> Value {
    json!({"type": "object", "properties": {"query": {"type": "string", "description": "Suchwörter"}}, "required": ["query"], "additionalProperties": false})
}
fn s_folder() -> Value {
    json!({"type": "object", "properties": {"path": {"type": "string", "description": "Ordnerpfad, z. B. Unterricht/Mathe; leer: oberste Ebene"}}, "additionalProperties": false})
}

const MAIL_LIST: ToolDef = ToolDef { name: "mail_list", title: "Mails auflisten", description: "Listet Mails (neueste zuerst) oder sucht nach Wörtern. Liefert Datum, Absender, Betreff, Vorschau und ID.", write: false, schema: s_mail_list };
const MAIL_READ: ToolDef = ToolDef { name: "mail_read", title: "Mail lesen", description: "Liest eine Mail ganz (Absender, Empfänger, Datum, Text).", write: false, schema: s_id };
const MAIL_DRAFT: ToolDef = ToolDef { name: "mail_draft", title: "Mail-Entwurf anlegen", description: "Legt einen Entwurf im Postfach an. Es wird nichts gesendet; die Person prüft und sendet selbst.", write: true, schema: s_draft };
const CAL_LIST: ToolDef = ToolDef { name: "calendar_list", title: "Kalender auflisten", description: "Listet die Kalender mit Namen und ID.", write: false, schema: s_none };
const CAL_EVENTS: ToolDef = ToolDef { name: "calendar_events", title: "Termine lesen", description: "Liest die Termine eines Zeitraums (Ortszeit).", write: false, schema: s_events };
const CAL_CREATE: ToolDef = ToolDef { name: "calendar_event_create", title: "Termin anlegen", description: "Legt einen Termin im Hauptkalender an.", write: true, schema: s_event_create };
const FILES_SEARCH: ToolDef = ToolDef { name: "files_search", title: "Dateien suchen", description: "Sucht Dateien nach Namen und Inhalt. Liefert Name, Art, Änderungsdatum und ID.", write: false, schema: s_search };
const FILES_LIST: ToolDef = ToolDef { name: "files_list", title: "Ordner auflisten", description: "Listet die Dateien und Ordner eines Ordners.", write: false, schema: s_folder };
const FILE_READ: ToolDef = ToolDef { name: "file_read", title: "Datei lesen", description: "Liest eine Datei als Text (Textdateien bis 1 MB, Google Docs, Tabellen als CSV).", write: false, schema: s_id };

static MAIL: &[ToolDef] = &[MAIL_LIST, MAIL_READ, MAIL_DRAFT];
static CALENDAR: &[ToolDef] = &[CAL_LIST, CAL_EVENTS, CAL_CREATE];
static CALDAV: &[ToolDef] = &[CAL_LIST, CAL_EVENTS];
static ONEDRIVE: &[ToolDef] = &[FILES_SEARCH, FILES_LIST, FILE_READ];
static DRIVE: &[ToolDef] = &[FILES_SEARCH, FILE_READ];

/// The tools of a catalog entry (write tools included; the caller drops them for read-only apps).
pub(crate) fn tools(entry: &str) -> &'static [ToolDef] {
    match entry {
        "outlook-mail" | "gmail" => MAIL,
        "outlook-calendar" | "google-calendar" => CALENDAR,
        "caldav" => CALDAV,
        "onedrive" => ONEDRIVE,
        "google-drive" => DRIVE,
        _ => &[],
    }
}

pub(crate) async fn run(c: &Ctx, acc: &Account, tool: &str, a: &Value) -> Result<String> {
    match (acc.entry.as_str(), tool) {
        ("outlook-mail", "mail_list") => graph_mail_list(c, acc, a).await,
        ("outlook-mail", "mail_read") => graph_mail_read(c, acc, a).await,
        ("outlook-mail", "mail_draft") => graph_mail_draft(c, acc, a).await,
        ("outlook-calendar", "calendar_list") => graph_calendars(c, acc).await,
        ("outlook-calendar", "calendar_events") => graph_events(c, acc, a).await,
        ("outlook-calendar", "calendar_event_create") => graph_event_create(c, acc, a).await,
        ("onedrive", "files_search") => onedrive_search(c, acc, a).await,
        ("onedrive", "files_list") => onedrive_list(c, acc, a).await,
        ("onedrive", "file_read") => onedrive_read(c, acc, a).await,
        ("gmail", "mail_list") => gmail_list(c, acc, a).await,
        ("gmail", "mail_read") => gmail_read(c, acc, a).await,
        ("gmail", "mail_draft") => gmail_draft(c, acc, a).await,
        ("google-calendar", "calendar_list") => gcal_calendars(c, acc).await,
        ("google-calendar", "calendar_events") => gcal_events(c, acc, a).await,
        ("google-calendar", "calendar_event_create") => gcal_event_create(c, acc, a).await,
        ("google-drive", "files_search") => drive_search(c, acc, a).await,
        ("google-drive", "file_read") => drive_read(c, acc, a).await,
        ("caldav", "calendar_list") => {
            let cals = caldav_calendars(c, acc, &load_secret(c, &acc.id)).await?;
            Ok(if cals.is_empty() { "Keine Kalender gefunden.".into() } else { cals.iter().map(|(href, name)| format!("- {name} (ID {href})")).collect::<Vec<_>>().join("\n") })
        }
        ("caldav", "calendar_events") => caldav_events(c, acc, a).await,
        _ => Err(Error::not_found(format!("Werkzeug {tool} gibt es nicht"))),
    }
}

// ---- helpers --------------------------------------------------------------------------------

fn arg(a: &Value, key: &str) -> String {
    a.get(key).and_then(Value::as_str).map(|s| s.trim().chars().take(4000).collect()).unwrap_or_default()
}

fn req(a: &Value, key: &str) -> Result<String> {
    Some(arg(a, key)).filter(|s| !s.is_empty()).ok_or_else(|| Error::invalid(format!("{key} fehlt")))
}

/// Text for one header line: no line breaks (they would start a new header).
fn one_line(s: &str) -> String {
    s.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect::<String>().trim().to_string()
}

fn count(a: &Value) -> u64 {
    a.get("count").and_then(Value::as_u64).unwrap_or(10).clamp(1, MAX_LIST)
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr).and_then(Value::as_str).map(one_line).unwrap_or_default()
}

/// A time the person or the model wrote, in local time unless it carries a zone.
pub(crate) fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return Local.from_local_datetime(&n).earliest().map(|t| t.with_timezone(&Utc));
        }
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().or_else(|| NaiveDate::parse_from_str(s, "%d.%m.%Y").ok())?;
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest().map(|t| t.with_timezone(&Utc))
}

/// The span `from`..`to` of a calendar request.
pub(crate) fn span(a: &Value) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let from = match arg(a, "from") {
        f if f.is_empty() => {
            let today = Local::now().date_naive().and_hms_opt(0, 0, 0).ok_or_else(|| Error::invalid("Datum"))?;
            Local.from_local_datetime(&today).earliest().map(|t| t.with_timezone(&Utc)).ok_or_else(|| Error::invalid("Datum"))?
        }
        f => parse_time(&f).ok_or_else(|| Error::invalid(format!("Unbekanntes Datum „{f}“ (z. B. 2026-12-01 oder 2026-12-01T08:00)")))?,
    };
    let to = match arg(a, "to") {
        t if t.is_empty() => from + chrono::Duration::days(7),
        t => parse_time(&t).ok_or_else(|| Error::invalid(format!("Unbekanntes Datum „{t}“")))?,
    };
    if to <= from {
        return Err(Error::invalid("Das Ende liegt vor dem Beginn"));
    }
    if to - from > chrono::Duration::days(MAX_DAYS) {
        return Err(Error::invalid("Höchstens ein Jahr auf einmal"));
    }
    Ok((from, to))
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string()
}

fn zulu(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// An authorized request; one retry with a fresh token after a 401.
async fn send(c: &Ctx, acc: &Account, method: reqwest::Method, url: &str, body: Option<&Value>, headers: &[(&str, &str)]) -> Result<reqwest::Response> {
    let client = http::client(true)?;
    let mut token = access_token(c, acc, false).await?;
    for attempt in 0..2 {
        let mut rb = client.request(method.clone(), url).bearer_auth(&token).header("accept", "application/json").timeout(Duration::from_secs(60));
        for (k, v) in headers {
            rb = rb.header(*k, *v);
        }
        if let Some(b) = body {
            rb = rb.json(b);
        }
        let resp = rb.send().await.map_err(http::net_err)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            token = access_token(c, acc, true).await?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(http::status_error(resp, &acc.name).await);
        }
        return Ok(resp);
    }
    Err(Error::auth(format!("{}: Anmeldung abgelehnt – bitte neu anmelden", acc.name)))
}

async fn get(c: &Ctx, acc: &Account, url: &str, headers: &[(&str, &str)]) -> Result<Value> {
    let resp = send(c, acc, reqwest::Method::GET, url, None, headers).await?;
    let text = http::read_capped(resp, MAX_JSON).await?;
    serde_json::from_str(&text).map_err(|_| Error::protocol(format!("{}: keine gültige Antwort", acc.name)))
}

async fn post(c: &Ctx, acc: &Account, url: &str, body: &Value) -> Result<Value> {
    let resp = send(c, acc, reqwest::Method::POST, url, Some(body), &[]).await?;
    let text = http::read_capped(resp, MAX_JSON).await?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn get_text(c: &Ctx, acc: &Account, url: &str) -> Result<String> {
    let resp = send(c, acc, reqwest::Method::GET, url, None, &[]).await?;
    let bytes = http::read_bytes_capped(resp, MAX_FILE, "Die Datei").await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn items(v: &Value, key: &str) -> Vec<Value> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().take(200).cloned().collect()).unwrap_or_default()
}

fn text_like(name: &str, mime: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    mime.starts_with("text/")
        || matches!(mime, "application/json" | "application/xml" | "application/x-yaml")
        || matches!(ext.as_str(), "txt" | "md" | "csv" | "tsv" | "json" | "xml" | "html" | "htm" | "ics" | "vcf" | "yaml" | "yml" | "log" | "tex")
}

fn recipients(to: &str) -> Result<Vec<String>> {
    let list: Vec<String> = to.split([',', ';']).map(one_line).filter(|x| !x.is_empty()).take(50).collect();
    if list.is_empty() || list.iter().any(|x| !x.contains('@') || x.contains(char::is_whitespace) && !x.contains('<')) {
        return Err(Error::invalid("Empfänger fehlen oder sind keine Mailadressen"));
    }
    Ok(list)
}

// ---- Microsoft Graph ------------------------------------------------------------------------

fn graph(c: &Ctx, path: &str) -> String {
    endpoint(c, &format!("https://graph.microsoft.com/v1.0{path}"))
}

const PREFER_TEXT: (&str, &str) = ("prefer", "outlook.body-content-type=\"text\"");

fn graph_who(v: &Value) -> String {
    let name = s(v, "/emailAddress/name");
    let addr = s(v, "/emailAddress/address");
    if name.is_empty() { addr } else { format!("{name} <{addr}>") }
}

fn graph_time(v: &Value) -> String {
    let raw = s(v, "/dateTime");
    let zone = s(v, "/timeZone");
    match NaiveDateTime::parse_from_str(&raw, "%Y-%m-%dT%H:%M:%S%.f") {
        Ok(n) if zone.eq_ignore_ascii_case("UTC") || zone.is_empty() => local(Utc.from_utc_datetime(&n)),
        Ok(n) => format!("{} ({zone})", n.format("%d.%m.%Y %H:%M")),
        Err(_) => raw,
    }
}

async fn graph_mail_list(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let n = count(a);
    let select = "id,subject,from,receivedDateTime,bodyPreview";
    let q = arg(a, "query");
    let url = if q.is_empty() {
        graph(c, &format!("/me/mailFolders/inbox/messages?$top={n}&$select={select}&$orderby=receivedDateTime%20desc"))
    } else {
        graph(c, &format!("/me/messages?$top={n}&$select={select}&$search={}", encode(&format!("\"{}\"", q.replace('"', " ")))))
    };
    let v = get(c, acc, &url, &[]).await?;
    let list = items(&v, "value");
    if list.is_empty() {
        return Ok("Keine Mails gefunden.".into());
    }
    Ok(list
        .iter()
        .map(|m| {
            let when = DateTime::parse_from_rfc3339(&s(m, "/receivedDateTime")).map(|t| local(t.with_timezone(&Utc))).unwrap_or_default();
            format!("- {when} · {} · {} (ID {})\n  {}", graph_who(m.get("from").unwrap_or(&Value::Null)), s(m, "/subject"), s(m, "/id"), s(m, "/bodyPreview").chars().take(200).collect::<String>())
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn graph_mail_read(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let id = req(a, "id")?;
    let url = graph(c, &format!("/me/messages/{}?$select=subject,from,toRecipients,ccRecipients,receivedDateTime,body", encode(&id)));
    let m = get(c, acc, &url, &[PREFER_TEXT]).await?;
    let list = |key: &str| items(&m, key).iter().map(graph_who).collect::<Vec<_>>().join(", ");
    let when = DateTime::parse_from_rfc3339(&s(&m, "/receivedDateTime")).map(|t| local(t.with_timezone(&Utc))).unwrap_or_default();
    let body = m.pointer("/body/content").and_then(Value::as_str).unwrap_or("");
    let body = if s(&m, "/body/contentType").eq_ignore_ascii_case("html") { strip_html(body) } else { body.to_string() };
    Ok(format!(
        "Betreff: {}\nVon: {}\nAn: {}\nCc: {}\nDatum: {when}\n\n{}",
        s(&m, "/subject"),
        graph_who(m.get("from").unwrap_or(&Value::Null)),
        list("toRecipients"),
        list("ccRecipients"),
        body.trim()
    ))
}

async fn graph_mail_draft(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let to = recipients(&req(a, "to")?)?;
    let body = json!({
        "subject": one_line(&arg(a, "subject")),
        "body": {"contentType": "Text", "content": a.get("body").and_then(Value::as_str).unwrap_or("")},
        "toRecipients": to.iter().map(|x| json!({"emailAddress": {"address": x}})).collect::<Vec<_>>(),
    });
    let v = post(c, acc, &graph(c, "/me/messages"), &body).await?;
    Ok(format!("Entwurf angelegt (nicht gesendet), ID {}. Die Person findet ihn im Ordner Entwürfe.", s(&v, "/id")))
}

async fn graph_calendars(c: &Ctx, acc: &Account) -> Result<String> {
    let v = get(c, acc, &graph(c, "/me/calendars?$select=id,name&$top=100"), &[]).await?;
    let list = items(&v, "value");
    Ok(if list.is_empty() { "Keine Kalender gefunden.".into() } else { list.iter().map(|k| format!("- {} (ID {})", s(k, "/name"), s(k, "/id"))).collect::<Vec<_>>().join("\n") })
}

async fn graph_events(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let (from, to) = span(a)?;
    let cal = arg(a, "calendar");
    let base = if cal.is_empty() { "/me/calendarView".to_string() } else { format!("/me/calendars/{}/calendarView", encode(&cal)) };
    let url = graph(
        c,
        &format!("{base}?startDateTime={}&endDateTime={}&$select=subject,start,end,location,isAllDay&$orderby=start/dateTime&$top=200", encode(&zulu(from)), encode(&zulu(to))),
    );
    let v = get(c, acc, &url, &[]).await?;
    let list = items(&v, "value");
    if list.is_empty() {
        return Ok(format!("Keine Termine zwischen {} und {}.", local(from), local(to)));
    }
    Ok(list
        .iter()
        .map(|e| {
            let place = s(e, "/location/displayName");
            let when = if e.get("isAllDay").and_then(Value::as_bool).unwrap_or(false) {
                format!("{} (ganztägig)", graph_time(e.get("start").unwrap_or(&Value::Null)).split(' ').next().unwrap_or(""))
            } else {
                format!("{} bis {}", graph_time(e.get("start").unwrap_or(&Value::Null)), graph_time(e.get("end").unwrap_or(&Value::Null)))
            };
            format!("- {when}: {}{}", s(e, "/subject"), if place.is_empty() { String::new() } else { format!(" ({place})") })
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn event_times(a: &Value) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = parse_time(&req(a, "start")?).ok_or_else(|| Error::invalid("Unbekannter Beginn (z. B. 2026-12-01T10:00)"))?;
    let end = parse_time(&req(a, "end")?).ok_or_else(|| Error::invalid("Unbekanntes Ende"))?;
    if end <= start {
        return Err(Error::invalid("Das Ende liegt vor dem Beginn"));
    }
    Ok((start, end))
}

async fn graph_event_create(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let (start, end) = event_times(a)?;
    let body = json!({
        "subject": one_line(&req(a, "title")?),
        "start": {"dateTime": start.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "end": {"dateTime": end.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "location": {"displayName": one_line(&arg(a, "location"))},
    });
    let v = post(c, acc, &graph(c, "/me/events"), &body).await?;
    Ok(format!("Termin angelegt: {} bis {} (ID {}).", local(start), local(end), s(&v, "/id")))
}

fn drive_item(i: &Value) -> String {
    let kind = if i.get("folder").is_some() { "Ordner".to_string() } else { s(i, "/file/mimeType") };
    let when = DateTime::parse_from_rfc3339(&s(i, "/lastModifiedDateTime")).map(|t| local(t.with_timezone(&Utc))).unwrap_or_default();
    format!("- {} ({kind}, geändert {when}) ID {}", s(i, "/name"), s(i, "/id"))
}

async fn onedrive_search(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let q = req(a, "query")?.replace('\'', "''");
    let v = get(c, acc, &graph(c, &format!("/me/drive/root/search(q='{}')?$top={MAX_LIST}", encode(&q))), &[]).await?;
    let list = items(&v, "value");
    Ok(if list.is_empty() { "Keine Dateien gefunden.".into() } else { list.iter().map(drive_item).collect::<Vec<_>>().join("\n") })
}

async fn onedrive_list(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let path = arg(a, "path");
    let segs: Vec<String> = path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..").map(encode).collect();
    let url = if segs.is_empty() { graph(c, "/me/drive/root/children?$top=200") } else { graph(c, &format!("/me/drive/root:/{}:/children?$top=200", segs.join("/"))) };
    let v = get(c, acc, &url, &[]).await?;
    let list = items(&v, "value");
    Ok(if list.is_empty() { "Der Ordner ist leer.".into() } else { list.iter().map(drive_item).collect::<Vec<_>>().join("\n") })
}

async fn onedrive_read(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let id = encode(&req(a, "id")?);
    let meta = get(c, acc, &graph(c, &format!("/me/drive/items/{id}?$select=id,name,size,file,folder")), &[]).await?;
    let (name, mime) = (s(&meta, "/name"), s(&meta, "/file/mimeType"));
    if meta.get("folder").is_some() {
        return Err(Error::invalid(format!("{name} ist ein Ordner (files_list)")));
    }
    let size = meta.get("size").and_then(Value::as_u64).unwrap_or(0);
    if !text_like(&name, &mime) || size > MAX_FILE as u64 {
        return Err(Error::unsupported(format!("{name} lässt sich hier nicht als Text lesen ({mime}, {} KB). Textdateien bis 1 MB gehen.", size / 1024)));
    }
    let text = get_text(c, acc, &graph(c, &format!("/me/drive/items/{id}/content"))).await?;
    Ok(format!("Datei: {name}\n\n{text}"))
}

// ---- Google ---------------------------------------------------------------------------------

fn gmail(c: &Ctx, path: &str) -> String {
    endpoint(c, &format!("https://gmail.googleapis.com/gmail/v1/users/me{path}"))
}

fn header(m: &Value, name: &str) -> String {
    items(m.pointer("/payload").unwrap_or(&Value::Null), "headers")
        .iter()
        .find(|h| h.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|h| s(h, "/value"))
        .unwrap_or_default()
}

async fn gmail_list(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let n = count(a);
    let q = arg(a, "query");
    let mut url = gmail(c, &format!("/messages?maxResults={n}"));
    if !q.is_empty() {
        url.push_str(&format!("&q={}", encode(&q)));
    }
    let v = get(c, acc, &url, &[]).await?;
    let ids: Vec<String> = items(&v, "messages").iter().map(|m| s(m, "/id")).filter(|i| !i.is_empty()).take(n as usize).collect();
    if ids.is_empty() {
        return Ok("Keine Mails gefunden.".into());
    }
    let mut out = Vec::new();
    for id in ids {
        let m = get(c, acc, &gmail(c, &format!("/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From&metadataHeaders=Date", encode(&id))), &[]).await?;
        out.push(format!("- {} · {} · {} (ID {id})\n  {}", header(&m, "Date"), header(&m, "From"), header(&m, "Subject"), s(&m, "/snippet").chars().take(200).collect::<String>()));
    }
    Ok(out.join("\n"))
}

fn b64url(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data.trim().trim_end_matches('=')).ok()
}

/// The text of a message: the first text/plain part, else text/html without tags.
fn gmail_body(part: &Value, depth: usize) -> Option<(bool, String)> {
    if depth > 12 {
        return None;
    }
    let mime = s(part, "/mimeType");
    if let Some(data) = part.pointer("/body/data").and_then(Value::as_str)
        && (mime == "text/plain" || mime == "text/html")
        && let Some(bytes) = b64url(data)
    {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        return Some((mime == "text/plain", if mime == "text/html" { strip_html(&text) } else { text }));
    }
    let parts = items(part, "parts");
    let found: Vec<(bool, String)> = parts.iter().filter_map(|p| gmail_body(p, depth + 1)).collect();
    found.iter().find(|(plain, _)| *plain).or(found.first()).cloned()
}

async fn gmail_read(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let id = req(a, "id")?;
    let m = get(c, acc, &gmail(c, &format!("/messages/{}?format=full", encode(&id))), &[]).await?;
    let body = m.get("payload").and_then(|p| gmail_body(p, 0)).map(|(_, t)| t).unwrap_or_else(|| s(&m, "/snippet"));
    Ok(format!(
        "Betreff: {}\nVon: {}\nAn: {}\nCc: {}\nDatum: {}\n\n{}",
        header(&m, "Subject"),
        header(&m, "From"),
        header(&m, "To"),
        header(&m, "Cc"),
        header(&m, "Date"),
        body.trim()
    ))
}

async fn gmail_draft(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let to = recipients(&req(a, "to")?)?;
    let b64 = &base64::engine::general_purpose::STANDARD;
    let subject = format!("=?UTF-8?B?{}?=", b64.encode(one_line(&arg(a, "subject"))));
    let body = b64.encode(a.get("body").and_then(Value::as_str).unwrap_or(""));
    let wrapped: Vec<&str> = body.as_bytes().chunks(76).filter_map(|ch| std::str::from_utf8(ch).ok()).collect();
    let raw = format!(
        "To: {}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
        to.join(", "),
        wrapped.join("\r\n")
    );
    let v = post(c, acc, &gmail(c, "/drafts"), &json!({"message": {"raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)}})).await?;
    Ok(format!("Entwurf angelegt (nicht gesendet), ID {}. Die Person findet ihn in Gmail unter Entwürfe.", s(&v, "/id")))
}

fn gcal(c: &Ctx, path: &str) -> String {
    endpoint(c, &format!("https://www.googleapis.com/calendar/v3{path}"))
}

async fn gcal_calendars(c: &Ctx, acc: &Account) -> Result<String> {
    let v = get(c, acc, &gcal(c, "/users/me/calendarList?maxResults=100"), &[]).await?;
    let list = items(&v, "items");
    Ok(if list.is_empty() { "Keine Kalender gefunden.".into() } else { list.iter().map(|k| format!("- {} (ID {})", s(k, "/summary"), s(k, "/id"))).collect::<Vec<_>>().join("\n") })
}

fn gcal_time(v: &Value) -> String {
    if let Ok(t) = DateTime::parse_from_rfc3339(&s(v, "/dateTime")) {
        return local(t.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(&s(v, "/date"), "%Y-%m-%d").map(|d| format!("{} (ganztägig)", d.format("%d.%m.%Y"))).unwrap_or_default()
}

async fn gcal_events(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let (from, to) = span(a)?;
    let cal = Some(arg(a, "calendar")).filter(|x| !x.is_empty()).unwrap_or_else(|| "primary".into());
    let url = gcal(
        c,
        &format!("/calendars/{}/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=250", encode(&cal), encode(&zulu(from)), encode(&zulu(to))),
    );
    let v = get(c, acc, &url, &[]).await?;
    let list = items(&v, "items");
    if list.is_empty() {
        return Ok(format!("Keine Termine zwischen {} und {}.", local(from), local(to)));
    }
    Ok(list
        .iter()
        .map(|e| {
            let place = s(e, "/location");
            format!(
                "- {} bis {}: {}{}",
                gcal_time(e.get("start").unwrap_or(&Value::Null)),
                gcal_time(e.get("end").unwrap_or(&Value::Null)),
                s(e, "/summary"),
                if place.is_empty() { String::new() } else { format!(" ({place})") }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn gcal_event_create(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let (start, end) = event_times(a)?;
    let body = json!({
        "summary": one_line(&req(a, "title")?),
        "location": one_line(&arg(a, "location")),
        "start": {"dateTime": zulu(start)},
        "end": {"dateTime": zulu(end)},
    });
    let v = post(c, acc, &gcal(c, "/calendars/primary/events"), &body).await?;
    Ok(format!("Termin angelegt: {} bis {} (ID {}).", local(start), local(end), s(&v, "/id")))
}

fn drive(c: &Ctx, path: &str) -> String {
    endpoint(c, &format!("https://www.googleapis.com/drive/v3{path}"))
}

async fn drive_search(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let q = req(a, "query")?.replace('\\', "\\\\").replace('\'', "\\'");
    let query = format!("fullText contains '{q}' and trashed = false");
    let v = get(c, acc, &drive(c, &format!("/files?q={}&pageSize={MAX_LIST}&fields={}", encode(&query), encode("files(id,name,mimeType,modifiedTime)"))), &[]).await?;
    let list = items(&v, "files");
    if list.is_empty() {
        return Ok("Keine Dateien gefunden.".into());
    }
    Ok(list
        .iter()
        .map(|f| {
            let when = DateTime::parse_from_rfc3339(&s(f, "/modifiedTime")).map(|t| local(t.with_timezone(&Utc))).unwrap_or_default();
            format!("- {} ({}, geändert {when}) ID {}", s(f, "/name"), s(f, "/mimeType"), s(f, "/id"))
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn drive_read(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let id = encode(&req(a, "id")?);
    let meta = get(c, acc, &drive(c, &format!("/files/{id}?fields=id,name,mimeType,size")), &[]).await?;
    let (name, mime) = (s(&meta, "/name"), s(&meta, "/mimeType"));
    let export = match mime.as_str() {
        "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        _ => None,
    };
    let text = if let Some(to) = export {
        get_text(c, acc, &drive(c, &format!("/files/{id}/export?mimeType={}", encode(to)))).await?
    } else {
        let size = meta.get("size").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|x| x.parse().ok()))).unwrap_or(0);
        if !text_like(&name, &mime) || size > MAX_FILE as u64 {
            return Err(Error::unsupported(format!("{name} lässt sich hier nicht als Text lesen ({mime}). Google Docs und Textdateien bis 1 MB gehen.")));
        }
        get_text(c, acc, &drive(c, &format!("/files/{id}?alt=media"))).await?
    };
    Ok(format!("Datei: {name}\n\n{text}"))
}

/// Text of HTML: tags out, the common entities back, blank lines squeezed.
fn strip_html(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    let bytes = html.len();
    while i < bytes {
        let rest = lower.get(i..).unwrap_or("");
        if rest.starts_with("<script") || rest.starts_with("<style") {
            let close = if rest.starts_with("<script") { "</script>" } else { "</style>" };
            i += rest.find(close).map(|e| e + close.len()).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with('<') {
            let tag_end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            let tag = rest.get(..tag_end).unwrap_or("");
            if tag.starts_with("<br") || tag.starts_with("<p") || tag.starts_with("</p") || tag.starts_with("<div") || tag.starts_with("<li") || tag.starts_with("<tr") {
                out.push('\n');
            }
            i += tag_end;
            continue;
        }
        let ch = html.get(i..).and_then(|r| r.chars().next()).unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    let out = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    let mut lines: Vec<&str> = Vec::new();
    for l in out.lines().map(str::trim_end) {
        if l.trim().is_empty() && lines.last().is_some_and(|p| p.trim().is_empty()) {
            continue;
        }
        lines.push(l);
    }
    lines.join("\n").trim().to_string()
}

// ---- CalDAV ---------------------------------------------------------------------------------

/// https only; plain http only on this computer.
pub(crate) fn check_caldav_url(url: &str) -> Result<()> {
    let u = reqwest::Url::parse(url.trim()).map_err(|_| Error::invalid("Die CalDAV-Adresse ist keine gültige Adresse (https://…)"))?;
    let local = u.host_str().is_some_and(aiscraft_core::is_loopback_host);
    if u.scheme() == "https" || (u.scheme() == "http" && local) {
        Ok(())
    } else {
        Err(Error::invalid("Die CalDAV-Adresse muss mit https:// beginnen"))
    }
}

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><d:current-user-principal/><c:calendar-home-set/></d:prop></d:propfind>"#;

async fn dav(acc: &Account, secret: &AccountSecret, method: &str, url: &str, depth: &str, body: String) -> Result<craft_xml::El> {
    check_caldav_url(url)?;
    let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| Error::invalid("Methode"))?;
    let resp = http::client(true)?
        .request(m, url)
        .basic_auth(&acc.username, secret.password.as_deref())
        .header("depth", depth)
        .header("content-type", "application/xml; charset=utf-8")
        .timeout(Duration::from_secs(60))
        .body(body)
        .send()
        .await
        .map_err(http::net_err)?;
    match resp.status().as_u16() {
        207 => {}
        401 | 403 => return Err(Error::auth(format!("{}: Benutzername oder App-Passwort stimmt nicht", acc.name))),
        _ => return Err(http::status_error(resp, &acc.name).await),
    }
    let bytes = http::read_bytes_capped(resp, craft_dav::MAX_BYTES, "Die Antwort des Kalenders").await?;
    craft_dav::parse_multistatus(&bytes).map_err(|e| Error::protocol(format!("{}: keine gültige CalDAV-Antwort ({e})", acc.name)))
}

fn join(base: &str, href: &str) -> Option<String> {
    reqwest::Url::parse(base).ok()?.join(href).ok().map(|u| u.to_string())
}

fn prop_href(response: &craft_xml::El, name: &str) -> Option<String> {
    craft_dav::properties(response).find(|p| p.name == name).and_then(|p| p.child_text("d:href")).map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

/// The calendars at the address: the collections there, else through the principal's
/// calendar home (RFC 4791 §6.2.1). (address, name) pairs.
pub(crate) async fn caldav_calendars(_c: &Ctx, acc: &Account, secret: &AccountSecret) -> Result<Vec<(String, String)>> {
    let mut url = acc.url.trim().to_string();
    for _ in 0..3 {
        let root = dav(acc, secret, "PROPFIND", &url, "1", PROPFIND.into()).await?;
        let mut cals = Vec::new();
        let (mut home, mut principal) = (None, None);
        for r in craft_dav::responses(&root) {
            let Some(href) = craft_dav::href(r) else { continue };
            let is_cal = craft_dav::properties(r).any(|p| p.name == "d:resourcetype" && p.child("c:calendar").is_some());
            if is_cal && let Some(abs) = join(&url, &href) {
                let name = craft_dav::properties(r).find(|p| p.name == "d:displayname").map(|p| one_line(&p.text())).filter(|n| !n.is_empty());
                let fallback = href.trim_end_matches('/').rsplit('/').next().unwrap_or("Kalender").to_string();
                cals.push((abs, name.unwrap_or(fallback)));
            }
            home = home.or_else(|| prop_href(r, "c:calendar-home-set"));
            principal = principal.or_else(|| prop_href(r, "d:current-user-principal"));
        }
        if !cals.is_empty() {
            cals.truncate(100);
            return Ok(cals);
        }
        let next = home.or(principal).and_then(|h| join(&url, &h)).filter(|n| *n != url);
        match next {
            Some(n) => url = n,
            None => break,
        }
    }
    Ok(Vec::new())
}

async fn caldav_events(c: &Ctx, acc: &Account, a: &Value) -> Result<String> {
    let (from, to) = span(a)?;
    let secret = load_secret(c, &acc.id);
    let want = arg(a, "calendar");
    let cals: Vec<(String, String)> =
        caldav_calendars(c, acc, &secret).await?.into_iter().filter(|(href, name)| want.is_empty() || *href == want || name.eq_ignore_ascii_case(&want)).collect();
    if cals.is_empty() {
        return Err(Error::not_found(if want.is_empty() { "Keine Kalender gefunden".to_string() } else { format!("Kalender „{want}“ nicht gefunden") }));
    }
    let range = format!(r#"start="{}" end="{}""#, from.format("%Y%m%dT%H%M%SZ"), to.format("%Y%m%dT%H%M%SZ"));
    let query = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-data><c:expand {range}/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range {range}/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
    );
    let mut out = Vec::new();
    for (href, name) in cals.iter().take(20) {
        let root = dav(acc, &secret, "REPORT", href, "1", query.clone()).await?;
        let data: Vec<String> =
            craft_dav::responses(&root).flat_map(|r| craft_dav::properties(r).filter(|p| p.name == "c:calendar-data").map(|p| p.text().into_owned()).collect::<Vec<_>>()).collect();
        let text = crate::attach::docs::calendar(name, data.join("\r\n").as_bytes());
        if !text.is_empty() {
            out.push(text);
        }
    }
    Ok(if out.is_empty() { format!("Keine Termine zwischen {} und {}.", local(from), local(to)) } else { out.join("\n") })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn times_html_and_recipients() {
        assert!(parse_time("2026-12-01").is_some() && parse_time("01.12.2026").is_some() && parse_time("2026-12-01T08:00").is_some());
        assert_eq!(parse_time("2026-12-01T08:00:00Z").map(zulu).as_deref(), Some("2026-12-01T08:00:00Z"));
        assert!(parse_time("morgen").is_none());
        assert!(span(&json!({"from": "2026-12-02", "to": "2026-12-01"})).is_err());
        assert!(span(&json!({"from": "2026-01-01", "to": "2028-01-01"})).is_err());
        assert!(span(&json!({})).is_ok());
        assert_eq!(strip_html("<p>Hallo&nbsp;<b>Welt</b></p><script>x()</script><br>Ende &amp; gut"), "Hallo Welt\n\nEnde & gut");
        assert!(recipients("a@b.de, c@d.de").unwrap().len() == 2);
        assert!(recipients("kein").is_err() && recipients("").is_err());
        assert_eq!(one_line("Betreff\r\nBcc: x@y.de"), "Betreff  Bcc: x@y.de");
        assert!(check_caldav_url("http://example.org/dav").is_err() && check_caldav_url("https://example.org/dav").is_ok());
        assert!(check_caldav_url("http://127.0.0.1:8080/dav").is_ok() && check_caldav_url("kaputt").is_err());
    }
}
