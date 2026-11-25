//! Connected apps (#26): the catalog, the OAuth sign-in against a local authorization server
//! (PKCE, refresh, wrong state, timeout), Microsoft Graph and CalDAV against scripted servers.
//! No browser opens: the tests play the browser by requesting the loopback address.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

#[derive(Default)]
struct Srv {
    /// The PKCE challenge the "browser" saw on the sign-in page.
    challenge: Option<String>,
    redirect_uri: Option<String>,
    /// The first access token no longer works (the server revoked it).
    reject_first: bool,
    /// Every request: method, path and body.
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Srv>>;

async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = sock.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let s = String::from_utf8_lossy(&buf).to_string();
        if let Some(end) = s.find("\r\n\r\n") {
            let len = s[..end]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, ctype: &str, body: &str) {
    let _ = sock
        .write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes())
        .await;
}

fn form_value(body: &str, key: &str) -> String {
    aiscraft_backends::oauth::query_pairs(body).into_iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or_default()
}

const CALDAV_AUTH: &str = "lehrerin:app-geheim";

/// One server for the authorization server, Graph and CalDAV.
fn server(engine: &Engine, shared: Shared) -> String {
    engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let req = read_request(&mut sock).await;
                    let head = req.split("\r\n\r\n").next().unwrap_or("").to_string();
                    let body = req.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
                    let line = head.lines().next().unwrap_or("").to_string();
                    let mut parts = line.split(' ');
                    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                    let path = target.split('?').next().unwrap_or("").to_string();
                    let auth = head.lines().find_map(|l| l.to_ascii_lowercase().starts_with("authorization:").then(|| l[14..].trim().to_string())).unwrap_or_default();
                    shared.lock().unwrap().seen.push(format!("{method} {target} {body}"));
                    // The authorization server.
                    if method == "POST" && path.ends_with("/token") {
                        let grant = form_value(&body, "grant_type");
                        let (status, out) = if grant == "authorization_code" {
                            let s = shared.lock().unwrap();
                            let verifier = form_value(&body, "code_verifier");
                            let pkce_ok = s.challenge.as_deref() == Some(aiscraft_backends::oauth::challenge(&verifier).as_str());
                            let redirect_ok = s.redirect_uri.as_deref() == Some(form_value(&body, "redirect_uri").as_str());
                            if form_value(&body, "code") == "CODE1" && pkce_ok && redirect_ok && form_value(&body, "client_id") == "cid" {
                                ("200 OK", json!({"access_token": "AT1", "refresh_token": "RT1", "expires_in": 3600, "token_type": "Bearer"}))
                            } else {
                                ("400 Bad Request", json!({"error": "invalid_grant", "error_description": "PKCE oder Code falsch"}))
                            }
                        } else if grant == "refresh_token" && form_value(&body, "refresh_token") == "RT1" {
                            ("200 OK", json!({"access_token": "AT2", "expires_in": 3600}))
                        } else {
                            ("400 Bad Request", json!({"error": "invalid_grant"}))
                        };
                        respond(&mut sock, status, "application/json", &out.to_string()).await;
                        return;
                    }
                    // CalDAV, with basic auth.
                    if path.starts_with("/dav/") {
                        let want = format!("basic {}", base64::engine::general_purpose::STANDARD.encode(CALDAV_AUTH));
                        if !auth.eq_ignore_ascii_case(&want) {
                            respond(&mut sock, "401 Unauthorized", "text/plain", "nein").await;
                            return;
                        }
                        let xml = if method == "PROPFIND" && path == "/dav/" {
                            r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<d:response><d:href>/dav/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/dav/schule/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><C:calendar/></d:resourcetype><d:displayname>Schule</d:displayname></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
</d:multistatus>"#
                        } else if method == "REPORT" && path == "/dav/schule/" && body.contains("time-range") {
                            "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:response><d:href>/dav/schule/1.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Elternabend 7b\r\nDTSTART:20261203T170000Z\r\nDTEND:20261203T183000Z\r\nLOCATION:Aula\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n</c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"
                        } else {
                            respond(&mut sock, "404 Not Found", "text/plain", "").await;
                            return;
                        };
                        respond(&mut sock, "207 Multi-Status", "application/xml; charset=utf-8", xml).await;
                        return;
                    }
                    // Microsoft Graph.
                    let reject_first = shared.lock().unwrap().reject_first;
                    let token_ok = auth == "Bearer AT2" || (auth == "Bearer AT1" && !reject_first);
                    if !token_ok {
                        respond(&mut sock, "401 Unauthorized", "application/json", r#"{"error":{"code":"InvalidAuthenticationToken","message":"Token abgelaufen"}}"#).await;
                        return;
                    }
                    let out = match (method.as_str(), path.as_str()) {
                        ("GET", "/v1.0/me/mailFolders/inbox/messages") => json!({"value": [{"id": "m1", "subject": "Klassenfahrt 7b", "receivedDateTime": "2026-11-20T08:00:00Z",
                            "from": {"emailAddress": {"name": "Schulleitung", "address": "leitung@schule.example"}}, "bodyPreview": "Liebe Kolleginnen"}]}),
                        ("GET", "/v1.0/me/calendarView") => json!({"value": [{"subject": "Zeugniskonferenz", "isAllDay": false,
                            "start": {"dateTime": "2026-12-04T13:00:00.0000000", "timeZone": "UTC"}, "end": {"dateTime": "2026-12-04T15:00:00.0000000", "timeZone": "UTC"},
                            "location": {"displayName": "Lehrerzimmer"}}]}),
                        ("POST", "/v1.0/me/events") => json!({"id": "ev1"}),
                        _ => {
                            respond(&mut sock, "404 Not Found", "application/json", r#"{"error":{"message":"gibt es nicht"}}"#).await;
                            return;
                        }
                    };
                    respond(&mut sock, "200 OK", "application/json", &out.to_string()).await;
                });
            }
        });
        format!("http://{addr}")
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Browser {
    /// Signs in and comes back with the code and the right state.
    SignsIn,
    /// Comes back with a state that belongs to another sign-in.
    WrongState,
    /// The person cancels.
    Cancels,
    /// Never comes back.
    Gone,
}

/// Plays the browser: reads the sign-in address and requests the loopback redirect.
fn browser(shared: Shared, how: Browser) -> crate::accounts::Opener {
    Arc::new(move |url: &str| {
        let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
        let get = |k: &str| aiscraft_backends::oauth::query_pairs(q).into_iter().find(|(n, _)| n == k).map(|(_, v)| v).unwrap_or_default();
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("response_type"), "code");
        let redirect = get("redirect_uri");
        assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
        {
            let mut s = shared.lock().unwrap();
            s.challenge = Some(get("code_challenge"));
            s.redirect_uri = Some(redirect.clone());
            s.seen.push(format!("SCOPE {}", get("scope")));
        }
        let back = match how {
            Browser::SignsIn => format!("{redirect}?code=CODE1&state={}", aiscraft_backends::oauth::encode(&get("state"))),
            Browser::WrongState => format!("{redirect}?code=CODE1&state=fremd"),
            Browser::Cancels => format!("{redirect}?error=access_denied&error_description=abgebrochen"),
            Browser::Gone => return Ok(()),
        };
        std::thread::spawn(move || {
            let rest = back.strip_prefix("http://").unwrap();
            let (host, path) = rest.split_at(rest.find('/').unwrap());
            let mut s = std::net::TcpStream::connect(host).unwrap();
            // A favicon request first, as browsers do: it must not end the sign-in.
            let _ = s.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n");
            let mut sink = String::new();
            let _ = s.read_to_string(&mut sink);
            let mut s = std::net::TcpStream::connect(host).unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes()).unwrap();
            let mut page = String::new();
            let _ = s.read_to_string(&mut page);
        });
        Ok(())
    })
}

fn setup(how: Browser, timeout: Duration) -> (Engine, Shared, String) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let shared: Shared = Arc::default();
    let origin = server(&engine, shared.clone());
    engine.ctx().0.accounts.set_test(Some(origin.clone()), Some(browser(shared.clone(), how)), timeout);
    // No model behind the trust check in these tests.
    engine.call("settings.set", json!({"key": "trust_check", "value": false})).unwrap();
    (engine, shared, origin)
}

fn tool_name(engine: &Engine, id: &str, tool: &str) -> String {
    engine.state().connectors.iter().find(|k| k.id == id).unwrap().tools.iter().find(|t| t.name == tool).unwrap().llm_name.clone()
}

#[test]
fn connector_catalog_lists_filters_and_every_entry_is_well_formed() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let all = engine.call("connector.catalog", json!({})).unwrap();
    assert_eq!(all["categories"], json!(["Mail", "Kalender", "Dateien"]));
    let entries = all["entries"].as_array().unwrap();
    assert!(entries.len() >= 7);
    let mut ids = std::collections::HashSet::new();
    for e in crate::accounts::catalog("", "") {
        assert!(ids.insert(e.id), "{} twice", e.id);
        assert!(!e.name.is_empty() && !e.description.is_empty() && !e.runs.is_empty() && !e.setup_hint.is_empty() && !e.reads.is_empty(), "{}", e.id);
        assert!(!e.categories.is_empty() && e.categories.iter().all(|c| crate::accounts::catalog_categories().contains(c)), "{}", e.id);
        let keys: std::collections::HashSet<_> = e.fields.iter().map(|f| f.key).collect();
        assert_eq!(keys.len(), e.fields.len(), "{}: field keys unique", e.id);
        match e.provider {
            "microsoft" | "google" => {
                assert_eq!(e.auth, "oauth", "{}", e.id);
                assert!(!e.read_scopes.is_empty() && keys.contains("client_id"), "{}", e.id);
                // Read-only by default: no scope that writes unless writing is ticked.
                assert!(e.read_scopes.iter().all(|s| !s.contains("ReadWrite") && !s.contains("compose") && !s.ends_with("calendar.events")), "{}", e.id);
            }
            "caldav" => assert_eq!(e.auth, "password"),
            "mcp" => assert_eq!(e.auth, "own"),
            other => panic!("unknown provider {other}"),
        }
        assert_eq!(e.writes.is_empty(), e.write_scopes.is_empty(), "{}: writes and write scopes go together", e.id);
        let tools = crate::accounts::tool_names(e.id);
        if e.provider != "mcp" {
            assert!(tools.iter().any(|(_, w)| !w), "{}: reads something", e.id);
            assert_eq!(tools.iter().any(|(_, w)| *w), !e.writes.is_empty(), "{}: a write tool only with writes", e.id);
        }
    }
    let cal = engine.call("connector.catalog", json!({"category": "kalender"})).unwrap();
    let cal = cal["entries"].as_array().unwrap();
    assert!(cal.len() >= 3 && cal.iter().all(|e| e["categories"].as_array().unwrap().contains(&json!("Kalender"))));
    let found = engine.call("connector.catalog", json!({"query": "Nextcloud"})).unwrap();
    assert_eq!(found["entries"][0]["id"], "caldav");
    assert_eq!(found["entries"].as_array().unwrap().len(), 1);
    let both = engine.call("connector.catalog", json!({"query": "outlook", "category": "Mail"})).unwrap();
    let ids: Vec<&str> = both["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"outlook-mail") && !ids.contains(&"outlook-calendar"), "{ids:?}");
    assert!(engine.call("connector.catalog", json!({"query": "gibtsnicht"})).unwrap()["entries"].as_array().unwrap().is_empty());
    assert!(engine.call("connector.catalog", json!({"category": "Fotos"})).is_err());
    // No secrets in what agents read.
    assert!(!all.to_string().contains("scope"));
}

#[test]
fn sign_in_with_pkce_reads_mail_and_renews_the_token() {
    let (engine, shared, _) = setup(Browser::SignsIn, Duration::from_secs(20));
    // The client ID comes from the person until AIS.ChatCraft has its own.
    let e = engine.call("connector.connect", json!({"entry": "outlook-mail"})).unwrap_err();
    assert!(e.message.contains("Client-ID") && e.message.contains("App-Registrierungen"), "{}", e.message);
    assert!(engine.call("connector.connect", json!({"entry": "gibtsnicht", "client_id": "cid"})).is_err());

    let out = engine.call("connector.connect", json!({"entry": "outlook-mail", "client_id": "cid", "tenant": "organizations"})).unwrap();
    let id = out["id"].as_str().unwrap().to_string();
    let scope = shared.lock().unwrap().seen.iter().find(|s| s.starts_with("SCOPE")).cloned().unwrap();
    assert!(scope.contains("Mail.Read") && scope.contains("offline_access") && !scope.contains("ReadWrite"), "read-only scopes: {scope}");
    assert!(shared.lock().unwrap().seen.iter().any(|s| s.starts_with("POST /organizations/oauth2/v2.0/token") && s.contains("code_verifier=")));
    let s = engine.state();
    let k = s.connectors.iter().find(|k| k.id == id).unwrap();
    assert_eq!(k.status, "connected");
    assert!(k.tools.iter().all(|t| t.read_only), "read only by default");
    assert!(k.tools.iter().all(|t| t.name != "mail_draft"));
    assert!(s.app_sign_in.is_none());
    // Tokens live in the keychain, not in the state or the database.
    let tokens = crate::accounts::load_secret(engine.ctx(), &id).tokens.unwrap();
    assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_deref()), ("AT1", Some("RT1")));
    let state_json = serde_json::to_string(&*engine.state()).unwrap();
    assert!(!state_json.contains("AT1") && !state_json.contains("RT1"));

    // A read tool: in the composer, for backends without tools.
    let list = tool_name(&engine, &id, "mail_list");
    let r = engine.call("connector.insert", json!({"tool": list})).unwrap();
    assert!(r["text"].as_str().unwrap().contains("Klassenfahrt 7b"), "{r}");
    // The token is revoked: one refresh with the refresh token, then the call goes through.
    shared.lock().unwrap().reject_first = true;
    let r = engine.call("connector.insert", json!({"tool": list, "arguments": {"count": 3}})).unwrap();
    assert!(r["text"].as_str().unwrap().contains("Schulleitung"));
    assert!(shared.lock().unwrap().seen.iter().any(|s| s.contains("grant_type=refresh_token") && s.contains("refresh_token=RT1")));
    let tokens = crate::accounts::load_secret(engine.ctx(), &id).tokens.unwrap();
    assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_deref()), ("AT2", Some("RT1")), "the refresh token is kept");
    // „Neu verbinden“ renews the token.
    engine.call("connector.refresh", json!({"id": id})).unwrap();

    // The code is worth nothing without the PKCE verifier.
    let client = aiscraft_backends::oauth::Client {
        auth_url: String::new(),
        token_url: crate::accounts::endpoint(engine.ctx(), "https://login.microsoftonline.com/x/oauth2/v2.0/token"),
        client_id: "cid".into(),
        client_secret: None,
        scopes: vec![],
        extra: vec![],
    };
    let redirect = shared.lock().unwrap().redirect_uri.clone().unwrap();
    let e = engine.block_on(aiscraft_backends::oauth::exchange(&client, "CODE1", "falscher-verifier", &redirect)).unwrap_err();
    assert!(e.message.contains("PKCE") || e.message.contains("invalid_grant"), "{}", e.message);

    // Switched off, the app does not run; removed, its keychain entry is gone.
    engine.call("connector.toggle", json!({"id": id, "enabled": false})).unwrap();
    assert!(engine.call("connector.insert", json!({"tool": list})).is_err());
    engine.call("connector.remove", json!({"id": id})).unwrap();
    assert!(engine.state().connectors.iter().all(|k| k.id != id));
    assert!(crate::accounts::load_secret(engine.ctx(), &id).tokens.is_none());
}

#[test]
fn a_wrong_state_a_cancel_a_timeout_and_no_browser_are_refused() {
    let (engine, _, _) = setup(Browser::WrongState, Duration::from_secs(20));
    let e = engine.call("connector.connect", json!({"entry": "outlook-calendar", "client_id": "cid"})).unwrap_err();
    assert_eq!(e.kind, aiscraft_core::ErrorKind::Refused, "{}", e.message);
    assert!(engine.state().connectors.is_empty(), "nothing is kept");

    let (engine, _, _) = setup(Browser::Cancels, Duration::from_secs(20));
    let e = engine.call("connector.connect", json!({"entry": "gmail", "client_id": "cid"})).unwrap_err();
    assert_eq!(e.kind, aiscraft_core::ErrorKind::Auth, "{}", e.message);
    assert!(e.message.contains("abgebrochen"));

    let (engine, _, _) = setup(Browser::Gone, Duration::from_millis(400));
    let e = engine.call("connector.connect", json!({"entry": "outlook-mail", "client_id": "cid"})).unwrap_err();
    assert!(e.message.contains("nicht rechtzeitig"), "{}", e.message);
    assert!(engine.state().app_sign_in.is_none(), "the waiting ends");
    assert!(engine.state().connectors.is_empty());

    // Engines in memory never open a browser.
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let e = engine.call("connector.connect", json!({"entry": "outlook-mail", "client_id": "cid"})).unwrap_err();
    assert_eq!(e.kind, aiscraft_core::ErrorKind::Unsupported);
    assert!(engine.call("connector.connect", json!({"entry": "outlook-mail", "client_id": "cid", "tenant": "a/../b"})).is_err());
}

#[test]
fn writing_needs_the_tick_and_the_owners_yes() {
    let (engine, shared, _) = setup(Browser::SignsIn, Duration::from_secs(20));
    let id =
        engine.call("connector.connect", json!({"entry": "outlook-calendar", "client_id": "cid", "write": true})).unwrap()["id"].as_str().unwrap().to_string();
    let scope = shared.lock().unwrap().seen.iter().find(|s| s.starts_with("SCOPE")).cloned().unwrap();
    assert!(scope.contains("Calendars.ReadWrite"), "{scope}");
    let create = tool_name(&engine, &id, "calendar_event_create");
    let events = tool_name(&engine, &id, "calendar_events");
    let k = engine.state().connectors.iter().find(|k| k.id == id).cloned().unwrap();
    assert!(!k.tools.iter().find(|t| t.name == "calendar_event_create").unwrap().read_only);
    // A write tool never goes into the composer.
    assert!(
        engine.call("connector.insert", json!({"tool": create, "arguments": {"title": "x", "start": "2026-12-01T10:00", "end": "2026-12-01T11:00"}})).is_err()
    );

    let ctx = engine.ctx().clone();
    let call = |name: &str, args: Value| aiscraft_core::ToolCall { id: crate::new_id(), name: name.into(), arguments: args, outcome: None };
    // Reading runs; without a person present, writing is refused.
    let ev = crate::trust::Evidence::default();
    let (text, failed, _) = engine.block_on(crate::tools::run(&ctx, "c1", &call(&events, json!({"from": "2026-12-01", "to": "2026-12-08"})), &ev, false));
    assert!(!failed && text.contains("Zeugniskonferenz") && text.contains("untrusted_tool_result"), "{text}");
    let args = json!({"title": "Elternsprechtag", "start": "2026-12-10T15:00", "end": "2026-12-10T18:00"});
    let (text, failed, label) = engine.block_on(crate::tools::run(&ctx, "c1", &call(&create, args.clone()), &ev, false));
    assert!(failed && label == "denied", "{text}");
    assert!(!shared.lock().unwrap().seen.iter().any(|s| s.starts_with("POST /v1.0/me/events")));
    // With the person there, the call asks; the yes runs it once.
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let (engine2, ctx, c) = (engine.clone(), ctx.clone(), call(&create, args));
        std::thread::spawn(move || {
            let _ = tx.send(engine2.block_on(crate::tools::run(&ctx, "c1", &c, &crate::trust::Evidence::default(), true)));
        });
    }
    assert!(engine.wait_for(Duration::from_secs(20), |s| !s.approvals.is_empty()), "asks the owner");
    let a = engine.state().approvals[0].clone();
    assert!(a.label.contains("Termin anlegen"), "{}", a.label);
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    let (text, failed, _) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    assert!(!failed && text.contains("Termin angelegt"), "{text}");
    let posts: Vec<String> = shared.lock().unwrap().seen.iter().filter(|s| s.starts_with("POST /v1.0/me/events")).cloned().collect();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].contains("Elternsprechtag"));
}

#[test]
fn caldav_calendars_and_events_from_a_scripted_server() {
    let (engine, shared, origin) = setup(Browser::Gone, Duration::from_secs(5));
    let url = format!("{origin}/dav/");
    let e = engine.call("connector.connect", json!({"entry": "caldav", "url": url, "username": "lehrerin", "password": "falsch"})).unwrap_err();
    assert_eq!(e.kind, aiscraft_core::ErrorKind::Auth, "{}", e.message);
    assert!(engine.state().connectors.is_empty(), "a password that does not work is not kept");
    assert!(
        engine.call("connector.connect", json!({"entry": "caldav", "url": "http://example.org/dav/", "username": "a", "password": "b"})).is_err(),
        "https only"
    );
    assert!(engine.call("connector.connect", json!({"entry": "caldav", "url": url})).is_err(), "user name and password needed");

    let id = engine.call("connector.connect", json!({"entry": "caldav", "url": url, "username": "lehrerin", "password": "app-geheim", "write": true})).unwrap()
        ["id"]
        .as_str()
        .unwrap()
        .to_string();
    let k = engine.state().connectors.iter().find(|k| k.id == id).cloned().unwrap();
    assert!(k.tools.iter().all(|t| t.read_only) && k.tools.len() == 2, "CalDAV only reads");
    assert!(!serde_json::to_string(&*engine.state()).unwrap().contains("app-geheim"));
    let r = engine.call("connector.insert", json!({"tool": tool_name(&engine, &id, "calendar_list")})).unwrap();
    assert!(r["text"].as_str().unwrap().contains("Schule"), "{r}");
    let r = engine
        .call("connector.insert", json!({"tool": tool_name(&engine, &id, "calendar_events"), "arguments": {"from": "2026-12-01", "to": "2026-12-08"}}))
        .unwrap();
    let text = r["text"].as_str().unwrap();
    assert!(text.contains("Elternabend 7b") && text.contains("Aula"), "{text}");
    assert!(shared.lock().unwrap().seen.iter().any(|s| s.starts_with("REPORT /dav/schule/") && s.contains("time-range start=\"2026")));
    assert!(engine.call("connector.insert", json!({"tool": tool_name(&engine, &id, "calendar_events"), "arguments": {"calendar": "Privat"}})).is_err());
    engine.call("connector.refresh", json!({"id": id})).unwrap();
    engine.call("connector.connect", json!({"id": id})).unwrap();
    engine.call("connector.remove", json!({"id": id})).unwrap();
    assert!(engine.state().connectors.is_empty());
}
