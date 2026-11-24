//! Live collaboration on artifacts: the engine side of the `collab` plug-in.
//!
//! Without a relay set, nothing here runs and every artifact stays local. Sharing makes a room on
//! the relay (room id = artifact id) and starts an [`aiscraft_collab::Session`]; every save of a
//! shared artifact goes through [`merge_local`], changes from the room arrive in [`on_remote`].
//! Room tokens and codes live in the keychain (`collab:<artifact id>`), the server key under
//! `collab-server-key`; the store only knows which artifacts are shared where.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aiscraft_collab::{CollabError, NewRoom, Options, Server, Session, Status, api, code};
use aiscraft_core::{Artifact, ArtifactKind, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::artifacts::{self, MAX_STATE};
use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{CollabView, Ctx, NoticeLevel, now_ms};

const SERVER_KEY: &str = "collab-server-key";

#[derive(Default)]
pub(crate) struct Hub {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

/// What the store keeps (no secrets).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    server: String,
    shares: Vec<Share>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Share {
    artifact_id: String,
    server: String,
    owner: bool,
}

/// What the keychain keeps per shared artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Keys {
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_token: Option<String>,
    code: String,
}

fn err(e: CollabError) -> Error {
    match e {
        CollabError::Invalid(m) => Error::invalid(m),
        CollabError::Network(m) => Error::network(m),
        CollabError::Refused(m) => Error::refused(m),
        CollabError::NotFound(m) => Error::not_found(m),
        CollabError::Protocol(m) => Error::protocol(m),
    }
}

macro_rules! cmd {
    ($id:literal, $label:literal, $params:literal, $f:expr) => {
        CommandSpec { id: $id, label: $label, group: "Zusammenarbeit", shortcut: None, params: $params, palette: false, run: $f }
    };
}

pub(crate) static COMMANDS: &[CommandSpec] = &[
    cmd!("collab.set_server", "Server für die Zusammenarbeit", "url: string (leer = keiner), key?: string (leer = entfernen)", |c, p| Box::pin(set_server(
        c, p
    ))),
    cmd!("collab.share", "Teilen", "id: string", |c, p| Box::pin(share(c, p))),
    cmd!("collab.join", "Beitreten", "code?: string, link?: string", |c, p| Box::pin(join(c, p))),
    cmd!("collab.leave", "Nicht mehr teilen", "id: string, end?: bool (Raum für alle beenden, nur wer ihn angelegt hat)", |c, p| Box::pin(leave(c, p))),
    cmd!("collab.status", "Stand der Zusammenarbeit", "id?: string", |c, p| Box::pin(status(c, p))),
];

fn hub(c: &Ctx) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
    c.0.collab.sessions.lock().unwrap_or_else(PoisonError::into_inner)
}

fn session(c: &Ctx, id: &str) -> Option<Arc<Session>> {
    hub(c).get(id).cloned()
}

fn saved(c: &Ctx) -> Saved {
    c.store(|st| st.setting("collab")).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

fn save(c: &Ctx, s: &Saved) -> Result<()> {
    c.store(|st| st.set_setting("collab", &serde_json::to_value(s)?))
}

fn keys(c: &Ctx, id: &str) -> Option<Keys> {
    c.0.secrets.get(&format!("collab:{id}")).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

/// On startup: show the plug-in and reconnect every shared artifact.
pub(crate) fn load(c: &Ctx) {
    let s = saved(c);
    c.update(|st| {
        st.collab.available = true;
        st.collab.server = s.server.clone();
    });
    for share in s.shares {
        let Some(k) = keys(c, &share.artifact_id) else { continue };
        if c.state().artifacts.iter().all(|a| a.id != share.artifact_id) {
            continue;
        }
        if let Err(e) = connect(c, &share, &k) {
            c.notice(NoticeLevel::Warning, format!("Zusammenarbeit: {}", e.message));
        }
    }
}

fn view(share: &Share, k: &Keys) -> CollabView {
    let link = Server::parse(&share.server).map(|s| s.join_link(&k.code)).unwrap_or_default();
    CollabView {
        artifact_id: share.artifact_id.clone(),
        server: share.server.clone(),
        code: code::display(&k.code),
        link,
        owner: share.owner,
        status: "connecting".into(),
        detail: "Verbinde …".into(),
        peers: 0,
    }
}

fn describe(s: &Status) -> (&'static str, String, u32) {
    match s {
        Status::Connecting => ("connecting", "Verbinde …".into(), 0),
        Status::Online { synced: false, peers } => ("connecting", "Gleiche ab …".into(), *peers),
        Status::Online { synced: true, peers } => (
            "online",
            match peers {
                0 | 1 => "Verbunden, sonst ist niemand im Raum".into(),
                n => format!("Verbunden, {n} Personen im Raum"),
            },
            *peers,
        ),
        Status::Offline { reason } => ("offline", format!("{reason}. Neuer Versuch läuft."), 0),
        Status::Ended { reason } => ("ended", reason.clone(), 0),
    }
}

/// Start the session for a share and show it.
fn connect(c: &Ctx, share: &Share, k: &Keys) -> Result<Arc<Session>> {
    let server = Server::parse(&share.server).map_err(err)?;
    let id = share.artifact_id.clone();
    let initial = artifacts::state_of(c, &id)?;
    let (ctx_r, id_r) = (c.clone(), id.clone());
    let (ctx_s, id_s) = (c.clone(), id.clone());
    let on_remote = Arc::new(move |full: Vec<u8>| on_remote(&ctx_r, &id_r, full));
    let on_status = Arc::new(move |s: &Status| {
        let (state, detail, peers) = describe(s);
        ctx_s.update(|st| {
            if let Some(v) = st.collab.shares.iter_mut().find(|v| v.artifact_id == id_s) {
                v.status = state.into();
                v.detail = detail;
                v.peers = peers;
            }
        });
    });
    let token = k.owner_token.as_deref().unwrap_or(&k.token);
    let _guard = c.0.handle.enter();
    let session = Arc::new(Session::start(&server, &id, token, &initial, on_remote, on_status, Options::default()).map_err(err)?);
    let mut v = view(share, k);
    c.update(|st| {
        st.collab.shares.retain(|x| x.artifact_id != id);
        st.collab.shares.push(v.clone());
    });
    hub(c).insert(id.clone(), session.clone());
    // A status that came before the view existed.
    let (state, detail, peers) = describe(&session.status());
    (v.status, v.detail, v.peers) = (state.into(), detail, peers);
    c.update(|st| {
        if let Some(x) = st.collab.shares.iter_mut().find(|x| x.artifact_id == id) {
            *x = v;
        }
    });
    Ok(session)
}

/// A save of `id`: merged into its session if it is shared. `None` = not shared, save as is.
pub(crate) fn merge_local(c: &Ctx, id: &str, state: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(s) = session(c, id) else { return Ok(None) };
    s.push(state).map(Some).map_err(err)
}

/// A change from the room: store the session's full state and let open views reload.
fn on_remote(c: &Ctx, id: &str, full: Vec<u8>) {
    let Ok(mut a) = artifacts::get_artifact(c, id) else { return };
    a.updated_at = now_ms();
    let r = c.store(|st| {
        let state = session(c, id).map(|s| s.state()).unwrap_or(full);
        if state.len() > MAX_STATE {
            return Err(Error::invalid("Artefakt ist zu groß (über 32 MB)"));
        }
        st.save_artifact(&a, Some(&state))
    });
    if let Err(e) = r {
        log::warn!("Zusammenarbeit: {e}");
        return;
    }
    c.update(|s| {
        if let Some(x) = s.artifacts.iter_mut().find(|x| x.id == id) {
            *x = a;
        }
        s.artifacts.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
        s.artifact_rev = s.artifact_rev.wrapping_add(1);
    });
}

/// Stop sharing `id` here (the artifact was deleted or left).
pub(crate) fn forget(c: &Ctx, id: &str) {
    if let Some(s) = hub(c).remove(id) {
        s.stop();
    }
    let mut sv = saved(c);
    if sv.shares.iter().any(|s| s.artifact_id == id) {
        sv.shares.retain(|s| s.artifact_id != id);
        if let Err(e) = save(c, &sv) {
            log::warn!("Zusammenarbeit: {e}");
        }
    }
    let _ = c.0.secrets.set(&format!("collab:{id}"), None);
    c.update(|st| st.collab.shares.retain(|v| v.artifact_id != id));
}

fn remember(c: &Ctx, share: &Share, k: &Keys) -> Result<()> {
    c.0.secrets.set(&format!("collab:{}", share.artifact_id), Some(&serde_json::to_string(k)?))?;
    let mut sv = saved(c);
    sv.shares.retain(|s| s.artifact_id != share.artifact_id);
    sv.shares.push(share.clone());
    save(c, &sv)
}

async fn set_server(c: Ctx, p: Value) -> Result<Value> {
    let url = req_str(&p, "url")?;
    let server = if url.trim().is_empty() { String::new() } else { Server::parse(&url).map_err(err)?.http().to_string() };
    if let Some(key) = opt_str(&p, "key") {
        let key = key.trim();
        if key.len() > 500 {
            return Err(Error::invalid("Der Schlüssel ist zu lang"));
        }
        c.0.secrets.set(SERVER_KEY, (!key.is_empty()).then_some(key))?;
    }
    let mut sv = saved(&c);
    sv.server = server.clone();
    save(&c, &sv)?;
    c.update(|st| st.collab.server = server.clone());
    Ok(json!({"server": server}))
}

async fn share(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let a = artifacts::get_artifact(&c, &id)?;
    if let Some(v) = c.state().share(&id) {
        return Ok(json!({"code": v.code, "link": v.link, "room": id}));
    }
    let url = c.state().collab.server.clone();
    if url.is_empty() {
        return Err(Error::invalid("Kein Server für die Zusammenarbeit eingestellt. Unter „Teilen“ die Adresse eintragen."));
    }
    let server = Server::parse(&url).map_err(err)?;
    let key = c.0.secrets.get(SERVER_KEY)?;
    let made =
        api::create_room(&server, key.as_deref(), &NewRoom { room: id.clone(), kind: a.kind.as_str().into(), title: a.title.clone() }).await.map_err(err)?;
    let share = Share { artifact_id: id.clone(), server: server.http().into(), owner: true };
    let k = Keys { token: made.token, owner_token: Some(made.owner_token), code: made.code };
    remember(&c, &share, &k)?;
    let session = connect(&c, &share, &k)?;
    session.wait_synced(Duration::from_secs(10)).await;
    let v = view(&share, &k);
    Ok(json!({"code": v.code, "link": v.link, "room": id}))
}

async fn join(c: Ctx, p: Value) -> Result<Value> {
    let given = opt_str(&p, "link").or_else(|| opt_str(&p, "code")).ok_or_else(|| Error::invalid("Code oder Link fehlt"))?;
    let (server, code) = match api::parse_link(&given) {
        Some(x) => x,
        None => {
            let code = code::normalize(&given).ok_or_else(|| Error::invalid("Das ist weder ein Code (XXXXX-XXXXX) noch ein Link zum Beitreten"))?;
            let url = c.state().collab.server.clone();
            if url.is_empty() {
                return Err(Error::invalid("Kein Server für die Zusammenarbeit eingestellt. Den ganzen Link einfügen oder die Adresse eintragen."));
            }
            (Server::parse(&url).map_err(err)?, code)
        }
    };
    let info = api::resolve(&server, &code).await.map_err(err)?;
    let kind = ArtifactKind::parse(&info.kind).ok_or_else(|| Error::unsupported("Diese Art von Artefakt kann AIS.ChatCraft noch nicht öffnen"))?;
    let id = info.room.clone();
    if c.state().share(&id).is_some() {
        c.update(|s| s.open_artifact = Some(id.clone()));
        return Ok(json!({"id": id, "synced": true}));
    }
    match artifacts::get_artifact(&c, &id) {
        Ok(a) if a.kind != kind => return Err(Error::invalid("Hier gibt es schon ein anderes Artefakt mit dieser Kennung")),
        Ok(_) => {}
        Err(_) => {
            let now = now_ms();
            let state = match kind {
                ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p => artifacts::doc_new("")?,
                ArtifactKind::Board => Vec::new(),
            };
            let title = if info.title.trim().is_empty() { kind.label().to_string() } else { info.title.chars().take(200).collect() };
            let a = Artifact { id: id.clone(), kind, title, conversation_id: None, project_id: None, created_at: now, updated_at: now };
            c.store(|st| st.save_artifact(&a, Some(&state)))?;
            c.update(|s| {
                s.artifacts.insert(0, a);
                s.artifact_rev = s.artifact_rev.wrapping_add(1);
            });
        }
    }
    let share = Share { artifact_id: id.clone(), server: server.http().into(), owner: false };
    let k = Keys { token: info.token, owner_token: None, code };
    remember(&c, &share, &k)?;
    let session = connect(&c, &share, &k)?;
    let synced = session.wait_synced(Duration::from_secs(10)).await;
    c.update(|s| s.open_artifact = Some(id.clone()));
    if !synced {
        c.notice(NoticeLevel::Warning, "Beigetreten, aber der Server hat den Stand noch nicht geschickt. Er kommt, sobald die Verbindung steht.");
    }
    Ok(json!({"id": id, "synced": synced}))
}

async fn leave(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let share = saved(&c).shares.into_iter().find(|s| s.artifact_id == id).ok_or_else(|| Error::not_found("Dieses Artefakt ist nicht geteilt"))?;
    let end = p.get("end").and_then(Value::as_bool).unwrap_or(false);
    let owner = keys(&c, &id).and_then(|k| k.owner_token);
    if end && owner.is_none() {
        return Err(Error::refused("Beenden kann nur, wer den Raum angelegt hat"));
    }
    forget(&c, &id);
    if let (true, Some(owner)) = (end, owner) {
        let server = Server::parse(&share.server).map_err(err)?;
        api::delete_room(&server, &id, &owner).await.map_err(err)?;
    }
    Ok(json!({"ended": end}))
}

async fn status(c: Ctx, p: Value) -> Result<Value> {
    let st = c.state();
    match opt_str(&p, "id") {
        Some(id) => Ok(match st.share(&id) {
            Some(v) => json!({"shared": true, "share": v}),
            None => json!({"shared": false}),
        }),
        None => Ok(serde_json::to_value(&st.collab)?),
    }
}
