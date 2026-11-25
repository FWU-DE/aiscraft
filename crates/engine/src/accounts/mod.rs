//! Connected apps (#26, like Nod's „Apps verbinden“): mail, calendar and files with a sign-in in
//! the browser instead of an MCP configuration.
//!
//! A built-in catalog lists what can be connected, by category (Mail, Kalender, Dateien). Most
//! entries run inside the engine: Microsoft 365 through Microsoft Graph and Google through its
//! REST APIs, both signed in with OAuth 2.0 (Authorization Code + PKCE, loopback redirect, the
//! system browser; `aiscraft_backends::oauth`), and CalDAV calendars with a user name and an app
//! password. Two entries start well-known MCP servers that sign in on their own (Softeria's
//! `ms-365-mcp-server`, `workspace-mcp`); they become ordinary connectors (`connector.add`).
//!
//! Every connected app is a connector in the state (`app-<entry>-<n>`), so its tools go through
//! the same sources, trust check, approvals, claims and action log as MCP tools (`tools.rs`).
//! Read-only by default: write tools (a mail draft, a calendar event) and their scopes exist only
//! when the person ticks „Auch schreiben“, and they never run without the owner's yes. Client
//! secrets, tokens and passwords live in the keychain (`secrets.rs`), never in the database.

mod providers;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aiscraft_backends::oauth;
use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{ConnectorTool, ConnectorView, Ctx, NoticeLevel, new_id};

/// AIS.ChatCraft has no app registered with Microsoft yet; until the owner registers one, the
/// person enters the client ID of their school's (or their own) app registration.
pub const BUILTIN_MICROSOFT_CLIENT_ID: Option<&str> = None;
/// The same for Google (a „Desktop-App“ OAuth client).
pub const BUILTIN_GOOGLE_CLIENT_ID: Option<&str> = None;

/// Keychain entry with the list of connected apps (no secrets in it).
const LIST_KEY: &str = "accounts";
/// Prefix of the ids of connected apps, which are connector ids too.
pub(crate) const PREFIX: &str = "app-";
/// How long the sign-in in the browser may take.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

// ---- catalog --------------------------------------------------------------------------------

/// A field of the setup form.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogField {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// Kept in the keychain and drawn as a password field.
    pub secret: bool,
    pub required: bool,
}

/// One app in the catalog.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    /// „Mail“, „Kalender“, „Dateien“; an MCP server for a whole suite has several.
    pub categories: &'static [&'static str],
    /// „microsoft“, „google“, „caldav“ or „mcp“.
    pub provider: &'static str,
    pub description: &'static str,
    /// What it can read.
    pub reads: &'static [&'static str],
    /// What it can do when writing is ticked (always with the owner's yes); empty: read only.
    pub writes: &'static [&'static str],
    /// „oauth“ (sign-in in the browser), „password“ (user name and app password) or „own“ (the
    /// MCP server signs in by itself).
    pub auth: &'static str,
    /// How it runs, for the person.
    pub runs: &'static str,
    pub fields: &'static [CatalogField],
    #[serde(skip)]
    pub read_scopes: &'static [&'static str],
    #[serde(skip)]
    pub write_scopes: &'static [&'static str],
    /// Where to register an app or create an app password.
    pub setup_hint: &'static str,
    #[serde(skip)]
    pub keywords: &'static str,
}

const MS_FIELDS: &[CatalogField] = &[
    CatalogField {
        key: "client_id",
        label: "Client-ID (Anwendungs-ID)",
        hint: "aus der App-Registrierung in Microsoft Entra",
        secret: false,
        required: BUILTIN_MICROSOFT_CLIENT_ID.is_none(),
    },
    CatalogField { key: "tenant", label: "Mandant", hint: "organizations (Schulkonto), common oder die Mandanten-ID", secret: false, required: false },
];
const MS_HINT: &str = "Microsoft Entra Admin Center › App-Registrierungen › Neue Registrierung: Plattform „Mobile und Desktopanwendungen“ mit der Umleitungs-URI http://127.0.0.1/callback (der Port wechselt, Microsoft übergeht ihn), „Öffentliche Clientflows“ erlauben. Die Anwendungs-ID ist die Client-ID; oft richtet das die Schul-IT ein.";
const GOOGLE_FIELDS: &[CatalogField] = &[
    CatalogField { key: "client_id", label: "Client-ID", hint: "…apps.googleusercontent.com", secret: false, required: BUILTIN_GOOGLE_CLIENT_ID.is_none() },
    CatalogField { key: "client_secret", label: "Clientschlüssel", hint: "gehört bei Desktop-Apps zur Client-ID", secret: true, required: false },
];
const GOOGLE_HINT: &str = "Google Cloud Console › APIs und Dienste › Anmeldedaten › OAuth-Client-ID vom Typ „Desktop-App“; die genutzte API (Gmail, Calendar, Drive) dort aktivieren.";
const MS_MAIL: &[&str] = &["offline_access", "User.Read", "Mail.Read"];
const MS_CAL: &[&str] = &["offline_access", "User.Read", "Calendars.Read"];
const MS_FILES: &[&str] = &["offline_access", "User.Read", "Files.Read"];
const G_MAIL: &[&str] = &["https://www.googleapis.com/auth/gmail.readonly"];
const G_CAL: &[&str] = &["https://www.googleapis.com/auth/calendar.readonly"];
const G_DRIVE: &[&str] = &["https://www.googleapis.com/auth/drive.readonly"];

static CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "outlook-mail",
        name: "Outlook-Mail (Microsoft 365)",
        categories: &["Mail"],
        provider: "microsoft",
        description: "Mails im Postfach deines Schul- oder Microsoft-Kontos suchen und lesen.",
        reads: &["Mails suchen und auflisten", "Eine Mail lesen"],
        writes: &["Entwürfe anlegen (es wird nichts gesendet)"],
        auth: "oauth",
        runs: "eingebaut (Microsoft Graph)",
        fields: MS_FIELDS,
        read_scopes: MS_MAIL,
        write_scopes: &["Mail.ReadWrite"],
        setup_hint: MS_HINT,
        keywords: "microsoft office exchange e-mail postfach",
    },
    CatalogEntry {
        id: "outlook-calendar",
        name: "Outlook-Kalender (Microsoft 365)",
        categories: &["Kalender"],
        provider: "microsoft",
        description: "Termine aus deinen Microsoft-365-Kalendern lesen.",
        reads: &["Kalender auflisten", "Termine eines Zeitraums"],
        writes: &["Termine anlegen"],
        auth: "oauth",
        runs: "eingebaut (Microsoft Graph)",
        fields: MS_FIELDS,
        read_scopes: MS_CAL,
        write_scopes: &["Calendars.ReadWrite"],
        setup_hint: MS_HINT,
        keywords: "microsoft office exchange termine teams",
    },
    CatalogEntry {
        id: "onedrive",
        name: "OneDrive (Microsoft 365)",
        categories: &["Dateien"],
        provider: "microsoft",
        description: "Dateien in OneDrive suchen, Ordner auflisten und Textdateien lesen.",
        reads: &["Dateien suchen", "Ordner auflisten", "Textdateien lesen"],
        writes: &[],
        auth: "oauth",
        runs: "eingebaut (Microsoft Graph)",
        fields: MS_FIELDS,
        read_scopes: MS_FILES,
        write_scopes: &[],
        setup_hint: MS_HINT,
        keywords: "microsoft office sharepoint cloud",
    },
    CatalogEntry {
        id: "gmail",
        name: "Gmail (Google)",
        categories: &["Mail"],
        provider: "google",
        description: "Mails in Gmail suchen und lesen.",
        reads: &["Mails suchen und auflisten", "Eine Mail lesen"],
        writes: &["Entwürfe anlegen (es wird nichts gesendet)"],
        auth: "oauth",
        runs: "eingebaut (Gmail-API)",
        fields: GOOGLE_FIELDS,
        read_scopes: G_MAIL,
        write_scopes: &["https://www.googleapis.com/auth/gmail.compose"],
        setup_hint: GOOGLE_HINT,
        keywords: "google workspace e-mail postfach",
    },
    CatalogEntry {
        id: "google-calendar",
        name: "Google Kalender",
        categories: &["Kalender"],
        provider: "google",
        description: "Termine aus deinen Google-Kalendern lesen.",
        reads: &["Kalender auflisten", "Termine eines Zeitraums"],
        writes: &["Termine anlegen"],
        auth: "oauth",
        runs: "eingebaut (Calendar-API)",
        fields: GOOGLE_FIELDS,
        read_scopes: G_CAL,
        write_scopes: &["https://www.googleapis.com/auth/calendar.events"],
        setup_hint: GOOGLE_HINT,
        keywords: "google workspace termine",
    },
    CatalogEntry {
        id: "google-drive",
        name: "Google Drive",
        categories: &["Dateien"],
        provider: "google",
        description: "Dateien in Google Drive suchen und lesen (Google Docs als Text).",
        reads: &["Dateien suchen", "Dateien und Google Docs als Text lesen"],
        writes: &[],
        auth: "oauth",
        runs: "eingebaut (Drive-API)",
        fields: GOOGLE_FIELDS,
        read_scopes: G_DRIVE,
        write_scopes: &[],
        setup_hint: GOOGLE_HINT,
        keywords: "google workspace docs cloud",
    },
    CatalogEntry {
        id: "caldav",
        name: "CalDAV-Kalender",
        categories: &["Kalender"],
        provider: "caldav",
        description: "Kalender von Nextcloud, iCloud, mailbox.org, Posteo und anderen CalDAV-Servern lesen.",
        reads: &["Kalender auflisten", "Termine eines Zeitraums"],
        writes: &[],
        auth: "password",
        runs: "eingebaut (CalDAV)",
        fields: &[
            CatalogField { key: "url", label: "Adresse", hint: "https://… (CalDAV-Adresse aus den Einstellungen des Kalenders)", secret: false, required: true },
            CatalogField { key: "username", label: "Benutzername", hint: "", secret: false, required: true },
            CatalogField { key: "password", label: "App-Passwort", hint: "ein eigenes App-Passwort, nicht das Hauptpasswort", secret: true, required: true },
        ],
        read_scopes: &[],
        write_scopes: &[],
        setup_hint: "Die CalDAV-Adresse und ein App-Passwort findest du in den Einstellungen deines Kalenders (Nextcloud: Einstellungen › Sicherheit › Geräte; iCloud: appleid.apple.com › App-spezifische Passwörter).",
        keywords: "nextcloud icloud apple mailbox posteo ical webdav termine",
    },
    CatalogEntry {
        id: "ms365-mcp",
        name: "Microsoft 365 über MCP-Server",
        categories: &["Mail", "Kalender", "Dateien"],
        provider: "mcp",
        description: "Startet den MCP-Server @softeria/ms-365-mcp-server (Node) im Nur-Lesen-Modus. Er meldet sich selbst an (Gerätecode) und speichert sein Token selbst.",
        reads: &["Mail, Kalender, OneDrive, Kontakte, Aufgaben"],
        writes: &[],
        auth: "own",
        runs: "MCP-Server über npx (Node nötig)",
        fields: &[
            CatalogField { key: "client_id", label: "Client-ID (optional)", hint: "leer: die App des Servers", secret: false, required: false },
            CatalogField { key: "tenant", label: "Mandant (optional)", hint: "für Schulkonten: die Mandanten-ID", secret: false, required: false },
        ],
        read_scopes: &[],
        write_scopes: &[],
        setup_hint: "Nach dem Verbinden fragt das Werkzeug „login“ des Servers nach einem Gerätecode; den gibst du unter microsoft.com/devicelogin ein.",
        keywords: "microsoft office outlook onedrive softeria",
    },
    CatalogEntry {
        id: "google-workspace-mcp",
        name: "Google Workspace über MCP-Server",
        categories: &["Mail", "Kalender", "Dateien"],
        provider: "mcp",
        description: "Startet den MCP-Server workspace-mcp (Python, uvx) mit Gmail, Kalender und Drive im Nur-Lesen-Modus. Er meldet sich selbst im Browser an.",
        reads: &["Gmail, Kalender, Drive"],
        writes: &[],
        auth: "own",
        runs: "MCP-Server über uvx (uv nötig)",
        fields: GOOGLE_FIELDS,
        read_scopes: &[],
        write_scopes: &[],
        setup_hint: GOOGLE_HINT,
        keywords: "google gmail drive kalender workspace",
    },
];

/// The categories in the order the catalog shows them.
pub fn catalog_categories() -> &'static [&'static str] {
    &["Mail", "Kalender", "Dateien"]
}

fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss")
}

/// The catalog, filtered by a search and a category (both optional, case does not matter).
pub fn catalog(query: &str, category: &str) -> Vec<CatalogEntry> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    let category = fold(category.trim());
    CATALOG
        .iter()
        .filter(|e| category.is_empty() || e.categories.iter().any(|c| fold(c) == category))
        .filter(|e| {
            let hay = fold(&format!("{} {} {} {} {}", e.name, e.description, e.keywords, e.categories.join(" "), e.runs));
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .cloned()
        .collect()
}

fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// `connector.catalog`.
pub(crate) async fn catalog_cmd(_c: Ctx, p: Value) -> Result<Value> {
    let query = opt_str(&p, "query").unwrap_or_default();
    let category = opt_str(&p, "category").unwrap_or_default();
    if !category.trim().is_empty() && !catalog_categories().iter().any(|c| fold(c) == fold(category.trim())) {
        return Err(Error::invalid(format!("Unbekannte Kategorie „{}“ (Mail, Kalender oder Dateien)", category.chars().take(40).collect::<String>())));
    }
    Ok(json!({"categories": catalog_categories(), "entries": catalog(&query, &category)}))
}

// ---- runtime and storage --------------------------------------------------------------------

pub(crate) type Opener = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;

pub(crate) struct Runtime {
    /// Opens the sign-in page; `None` in engines in memory (tests inject their own).
    opener: Mutex<Option<Opener>>,
    /// Test hook: every provider address goes to this local origin instead.
    origin: Mutex<Option<String>>,
    timeout: Mutex<Duration>,
    /// One token refresh at a time, so two calls do not use up a refresh token twice.
    refresh: tokio::sync::Mutex<()>,
}

impl Runtime {
    pub(crate) fn new(browser: bool) -> Self {
        let opener: Option<Opener> = browser
            .then(|| Arc::new(|url: &str| webbrowser::open(url).map_err(|e| Error::unsupported(format!("Browser ließ sich nicht öffnen: {e}")))) as Opener);
        Self { opener: Mutex::new(opener), origin: Mutex::new(None), timeout: Mutex::new(SIGN_IN_TIMEOUT), refresh: tokio::sync::Mutex::new(()) }
    }

    /// Tests: a local server for every provider address, an opener that plays the browser and
    /// a short wait for the redirect.
    #[cfg(test)]
    pub(crate) fn set_test(&self, origin: Option<String>, opener: Option<Opener>, timeout: Duration) {
        *self.origin.lock().unwrap_or_else(PoisonError::into_inner) = origin;
        *self.opener.lock().unwrap_or_else(PoisonError::into_inner) = opener;
        *self.timeout.lock().unwrap_or_else(PoisonError::into_inner) = timeout;
    }
}

/// A provider address, or the test server's in its place (same path).
pub(crate) fn endpoint(c: &Ctx, url: &str) -> String {
    let origin = c.0.accounts.origin.lock().unwrap_or_else(PoisonError::into_inner).clone();
    match origin {
        Some(o) => {
            let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
            let path = rest.find('/').and_then(|i| rest.get(i..)).unwrap_or("/");
            format!("{}{path}", o.trim_end_matches('/'))
        }
        None => url.to_string(),
    }
}

/// A connected app, as stored (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Account {
    pub id: String,
    pub entry: String,
    pub name: String,
    pub enabled: bool,
    /// The person ticked „Auch schreiben“.
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub tenant: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
}

/// The secrets of a connected app, one keychain entry each.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AccountSecret {
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub tokens: Option<oauth::Tokens>,
}

fn secret_key(id: &str) -> String {
    format!("account:{id}")
}

fn load_accounts(c: &Ctx) -> Vec<Account> {
    c.0.secrets.get(LIST_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_accounts(c: &Ctx, all: &[Account]) -> Result<()> {
    c.0.secrets.set(LIST_KEY, Some(&serde_json::to_string(all)?))
}

pub(crate) fn load_secret(c: &Ctx, id: &str) -> AccountSecret {
    c.0.secrets.get(&secret_key(id)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_secret(c: &Ctx, id: &str, s: &AccountSecret) -> Result<()> {
    c.0.secrets.set(&secret_key(id), Some(&serde_json::to_string(s)?))
}

pub(crate) fn is_account(id: &str) -> bool {
    id.starts_with(PREFIX)
}

fn account(c: &Ctx, id: &str) -> Result<Account> {
    load_accounts(c).into_iter().find(|a| a.id == id).ok_or_else(|| Error::not_found("Verbundene App nicht gefunden"))
}

fn tool_view(acc: &Account, t: &providers::ToolDef) -> ConnectorTool {
    ConnectorTool {
        llm_name: aiscraft_mcp::llm_tool_name(&acc.id, t.name),
        name: t.name.into(),
        title: t.title.into(),
        description: t.description.into(),
        read_only: !t.write,
        destructive: false,
        input_schema: (t.schema)(),
        ui_resource: None,
        app_only: false,
        app_callable: false,
    }
}

fn view_of(c: &Ctx, acc: &Account) -> ConnectorView {
    let e = entry(&acc.entry);
    let mode = if acc.write { "lesen und mit Freigabe schreiben" } else { "nur lesen" };
    let runs = e.map(|e| e.runs).unwrap_or("eingebaut");
    let target = if acc.url.is_empty() { format!("{runs} · {mode}") } else { format!("{runs} · {} · {mode}", acc.url) };
    let secret = load_secret(c, &acc.id);
    let signed_in = secret.tokens.is_some() || secret.password.is_some();
    let tools = providers::tools(&acc.entry).iter().filter(|t| acc.write || !t.write).map(|t| tool_view(acc, t)).collect();
    ConnectorView {
        id: acc.id.clone(),
        name: acc.name.clone(),
        enabled: acc.enabled,
        target,
        status: if !acc.enabled {
            "off".into()
        } else if signed_in {
            "connected".into()
        } else {
            "error".into()
        },
        error: (acc.enabled && !signed_in).then(|| "Nicht angemeldet – bitte neu anmelden".to_string()),
        tools,
        extension: None,
    }
}

/// At start: the connected apps as connectors, without any request (tokens are checked on use).
pub(crate) fn load(c: &Ctx) {
    let views: Vec<ConnectorView> = load_accounts(c).iter().map(|a| view_of(c, a)).collect();
    c.update(|s| {
        s.connectors.retain(|v| !is_account(&v.id));
        s.connectors.extend(views);
    });
}

fn put_view(c: &Ctx, v: ConnectorView) {
    c.update(|s| match s.connectors.iter_mut().find(|x| x.id == v.id) {
        Some(x) => *x = v,
        None => s.connectors.push(v),
    });
}

fn set_error(c: &Ctx, id: &str, msg: &str) {
    let (id, msg) = (id.to_string(), msg.to_string());
    c.update(move |s| {
        if let Some(v) = s.connectors.iter_mut().find(|v| v.id == id) {
            v.status = "error".into();
            v.error = Some(msg);
        }
    });
}

// ---- OAuth ----------------------------------------------------------------------------------

/// The OAuth client of an account.
fn oauth_client(c: &Ctx, e: &CatalogEntry, acc: &Account, secret: &AccountSecret) -> Result<oauth::Client> {
    let mut scopes: Vec<String> = e.read_scopes.iter().map(|s| s.to_string()).collect();
    if acc.write {
        scopes.extend(e.write_scopes.iter().map(|s| s.to_string()));
    }
    match e.provider {
        "microsoft" => {
            let tenant = if acc.tenant.trim().is_empty() { "organizations".to_string() } else { acc.tenant.trim().to_string() };
            if !tenant.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_')) || tenant.len() > 100 {
                return Err(Error::invalid("Der Mandant ist ungültig (organizations, common, consumers, eine ID oder eine Domain)"));
            }
            Ok(oauth::Client {
                auth_url: endpoint(c, &format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize")),
                token_url: endpoint(c, &format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token")),
                client_id: acc.client_id.clone(),
                client_secret: None,
                scopes,
                extra: vec![("prompt".into(), "select_account".into())],
            })
        }
        "google" => Ok(oauth::Client {
            auth_url: endpoint(c, "https://accounts.google.com/o/oauth2/v2/auth"),
            token_url: endpoint(c, "https://oauth2.googleapis.com/token"),
            client_id: acc.client_id.clone(),
            client_secret: secret.client_secret.clone(),
            scopes,
            // Without these Google gives no refresh token.
            extra: vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
        }),
        _ => Err(Error::invalid("Diese App meldet sich nicht über den Browser an")),
    }
}

/// Sign in in the browser and keep the tokens.
async fn sign_in(c: &Ctx, e: &CatalogEntry, acc: &Account, secret: &mut AccountSecret) -> Result<()> {
    let opener = c.0.accounts.opener.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(opener) = opener else {
        return Err(Error::unsupported("Die Anmeldung im Browser geht nur in der App (nicht in Tests oder Skripten)."));
    };
    let client = oauth_client(c, e, acc, secret)?;
    let pending = oauth::start(&client).await?;
    opener(pending.url())?;
    let name = e.name.to_string();
    c.update(|s| s.app_sign_in = Some(name));
    c.notice(NoticeLevel::Info, format!("Anmeldung für {} im Browser geöffnet …", e.name));
    let timeout = *c.0.accounts.timeout.lock().unwrap_or_else(PoisonError::into_inner);
    let r = pending.finish(&client, timeout).await;
    c.update(|s| s.app_sign_in = None);
    secret.tokens = Some(r?);
    Ok(())
}

/// A valid access token; refreshed when it ran out or `force` (after a 401).
pub(crate) async fn access_token(c: &Ctx, acc: &Account, force: bool) -> Result<String> {
    let e = entry(&acc.entry).ok_or_else(|| Error::not_found("Unbekannte App"))?;
    let _one = c.0.accounts.refresh.lock().await;
    let mut secret = load_secret(c, &acc.id);
    let Some(tokens) = secret.tokens.clone() else {
        return Err(Error::auth(format!("{} ist nicht angemeldet. Bitte in den Einstellungen neu anmelden.", acc.name)));
    };
    if !force && !tokens.expired(crate::now_ms()) {
        return Ok(tokens.access_token);
    }
    let Some(rt) = tokens.refresh_token.clone() else {
        set_error(c, &acc.id, "Anmeldung abgelaufen – bitte neu anmelden");
        return Err(Error::auth(format!("Die Anmeldung bei {} ist abgelaufen. Bitte in den Einstellungen neu anmelden.", acc.name)));
    };
    let client = oauth_client(c, e, acc, &secret)?;
    match oauth::refresh(&client, &rt).await {
        Ok(t) => {
            let access = t.access_token.clone();
            secret.tokens = Some(t);
            save_secret(c, &acc.id, &secret)?;
            Ok(access)
        }
        Err(err) => {
            if err.kind == aiscraft_core::ErrorKind::Auth {
                set_error(c, &acc.id, "Anmeldung abgelaufen – bitte neu anmelden");
            }
            Err(err)
        }
    }
}

// ---- commands -------------------------------------------------------------------------------

fn field(p: &Value, key: &str) -> String {
    opt_str(p, key).map(|s| s.trim().chars().take(2000).collect()).unwrap_or_default()
}

/// `connector.connect`: set up a catalog entry (fields, then the sign-in), or sign in again
/// (`id` of a connected app).
pub(crate) async fn connect(c: Ctx, p: Value) -> Result<Value> {
    if let Some(id) = opt_str(&p, "id").filter(|i| !i.is_empty()) {
        return sign_in_again(&c, &id).await;
    }
    let entry_id = req_str(&p, "entry")?;
    let e = entry(&entry_id).ok_or_else(|| Error::not_found(format!("„{}“ steht nicht im Katalog", entry_id.chars().take(60).collect::<String>())))?;
    for f in e.fields.iter().filter(|f| f.required) {
        let builtin = f.key == "client_id"
            && match e.provider {
                "microsoft" => BUILTIN_MICROSOFT_CLIENT_ID.is_some(),
                "google" => BUILTIN_GOOGLE_CLIENT_ID.is_some(),
                _ => false,
            };
        if field(&p, f.key).is_empty() && !builtin {
            return Err(Error::invalid(format!("{} fehlt. {}", f.label, e.setup_hint)));
        }
    }
    if e.provider == "mcp" {
        return connect_mcp(c, e, &p).await;
    }
    let write = p.get("write").and_then(Value::as_bool).unwrap_or(false) && !e.writes.is_empty();
    let name = Some(field(&p, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| e.name.to_string());
    let client_id = Some(field(&p, "client_id")).filter(|x| !x.is_empty()).or_else(|| {
        match e.provider {
            "microsoft" => BUILTIN_MICROSOFT_CLIENT_ID,
            "google" => BUILTIN_GOOGLE_CLIENT_ID,
            _ => None,
        }
        .map(str::to_string)
    });
    let acc = Account {
        id: format!("{PREFIX}{}-{}", e.id, new_id().chars().filter(char::is_ascii_alphanumeric).take(6).collect::<String>()),
        entry: e.id.into(),
        name: name.chars().take(100).collect(),
        enabled: true,
        write,
        client_id: client_id.unwrap_or_default(),
        tenant: field(&p, "tenant"),
        url: field(&p, "url"),
        username: field(&p, "username"),
    };
    let mut secret = AccountSecret {
        client_secret: Some(field(&p, "client_secret")).filter(|s| !s.is_empty()),
        password: Some(field(&p, "password")).filter(|s| !s.is_empty()),
        tokens: None,
    };
    if e.provider == "caldav" {
        providers::check_caldav_url(&acc.url)?;
        // Credentials that do not work are not kept.
        let n = providers::caldav_calendars(&c, &acc, &secret).await?.len();
        c.notice(NoticeLevel::Success, format!("{} verbunden: {n} Kalender", acc.name));
    } else {
        sign_in(&c, e, &acc, &mut secret).await?;
        c.notice(NoticeLevel::Success, format!("{} verbunden ({})", acc.name, if acc.write { "lesen, schreiben mit Freigabe" } else { "nur lesen" }));
    }
    save_secret(&c, &acc.id, &secret)?;
    let mut all = load_accounts(&c);
    all.push(acc.clone());
    if let Err(err) = save_accounts(&c, &all) {
        let _ = c.0.secrets.set(&secret_key(&acc.id), None);
        return Err(err);
    }
    put_view(&c, view_of(&c, &acc));
    Ok(json!({"id": acc.id, "tools": providers::tools(&acc.entry).iter().filter(|t| acc.write || !t.write).count()}))
}

async fn sign_in_again(c: &Ctx, id: &str) -> Result<Value> {
    let acc = account(c, id)?;
    let e = entry(&acc.entry).ok_or_else(|| Error::not_found("Unbekannte App"))?;
    let mut secret = load_secret(c, id);
    if e.provider == "caldav" {
        providers::caldav_calendars(c, &acc, &secret).await?;
    } else {
        sign_in(c, e, &acc, &mut secret).await?;
        save_secret(c, id, &secret)?;
    }
    put_view(c, view_of(c, &acc));
    c.notice(NoticeLevel::Success, format!("{} neu angemeldet", acc.name));
    Ok(json!({"id": id}))
}

/// An MCP server from the catalog: an ordinary connector, read-only, signing in by itself.
async fn connect_mcp(c: Ctx, e: &CatalogEntry, p: &Value) -> Result<Value> {
    let mut env = serde_json::Map::new();
    let (client_id, secret, tenant) = (field(p, "client_id"), field(p, "client_secret"), field(p, "tenant"));
    let command = match e.id {
        "ms365-mcp" => {
            if !client_id.is_empty() {
                env.insert("MS365_MCP_CLIENT_ID".into(), json!(client_id));
            }
            if !tenant.is_empty() {
                env.insert("MS365_MCP_TENANT_ID".into(), json!(tenant));
            }
            format!("npx -y @softeria/ms-365-mcp-server --read-only{}", if tenant.is_empty() { "" } else { " --org-mode" })
        }
        _ => {
            env.insert("GOOGLE_OAUTH_CLIENT_ID".into(), json!(client_id));
            if !secret.is_empty() {
                env.insert("GOOGLE_OAUTH_CLIENT_SECRET".into(), json!(secret));
            }
            "uvx workspace-mcp --read-only --tools gmail calendar drive".to_string()
        }
    };
    crate::tools::add(c, json!({"name": e.name, "command": command, "env": env})).await
}

pub(crate) async fn remove(c: &Ctx, id: &str) -> Result<Value> {
    let mut all = load_accounts(c);
    let before = all.len();
    all.retain(|a| a.id != id);
    if all.len() == before {
        return Err(Error::not_found("Verbundene App nicht gefunden"));
    }
    save_accounts(c, &all)?;
    c.0.secrets.set(&secret_key(id), None)?;
    let id = id.to_string();
    c.update(move |s| s.connectors.retain(|v| v.id != id));
    Ok(json!({}))
}

pub(crate) async fn toggle(c: &Ctx, id: &str, enabled: bool) -> Result<Value> {
    let mut all = load_accounts(c);
    let acc = all.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::not_found("Verbundene App nicht gefunden"))?;
    acc.enabled = enabled;
    let acc = acc.clone();
    save_accounts(c, &all)?;
    put_view(c, view_of(c, &acc));
    Ok(json!({}))
}

/// `connector.refresh` of a connected app: renew the token or check the password.
pub(crate) async fn refresh(c: &Ctx, id: &str) -> Result<Value> {
    let acc = account(c, id)?;
    let r = if acc.entry == "caldav" {
        providers::caldav_calendars(c, &acc, &load_secret(c, id)).await.map(|_| ())
    } else {
        access_token(c, &acc, true).await.map(|_| ())
    };
    match r {
        Ok(()) => {
            put_view(c, view_of(c, &acc));
            Ok(json!({}))
        }
        Err(e) => {
            set_error(c, id, &e.message);
            Err(e)
        }
    }
}

/// Run a tool of a connected app (after `tools.rs` decided it may run).
pub(crate) async fn execute(c: &Ctx, id: &str, tool: &str, args: &Value) -> Result<String> {
    let acc = account(c, id)?;
    if !acc.enabled {
        return Err(Error::invalid(format!("{} ist ausgeschaltet", acc.name)));
    }
    let def = providers::tools(&acc.entry).iter().find(|t| t.name == tool).ok_or_else(|| Error::not_found(format!("Werkzeug {tool} gibt es nicht")))?;
    if def.write && !acc.write {
        return Err(Error::refused(format!("{} ist nur zum Lesen verbunden", acc.name)));
    }
    providers::run(c, &acc, tool, args).await
}

#[cfg(test)]
pub(crate) fn tool_names(entry: &str) -> Vec<(&'static str, bool)> {
    providers::tools(entry).iter().map(|t| (t.name, t.write)).collect()
}
