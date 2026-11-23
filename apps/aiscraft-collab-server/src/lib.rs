//! The AIS.ChatCraft collaboration relay: one small binary that keeps y-sync rooms for documents
//! and whiteboards. A room is one artifact. Its owner makes it (`POST /rooms`) and gets an owner
//! token and a join code; others turn the code into the room token (`GET /join/<code>`) and sync
//! over `GET /r/<room>?token=…` (WebSocket, y-sync as y-websocket speaks it). `GET /healthz`
//! answers `ok`. Logs never contain tokens, codes or content.

mod rooms;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use aiscraft_collab::{Created, JoinInfo, NewRoom, code, proto};
use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use yrs::sync::{Message, SyncMessage};

use rooms::{CodeKey, Index, Live, Meta, Out};

/// How the relay runs.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// When set, making rooms needs `Authorization: Bearer <key>`.
    pub create_key: Option<String>,
    pub max_rooms: usize,
    pub max_room_bytes: usize,
    pub max_peers: usize,
    /// Rooms nobody opened for this many days are deleted (0 = keep forever).
    pub idle_days: u64,
    /// Fold the log into a snapshot once it is this large.
    pub compact_after: usize,
    /// Take the client address from `X-Forwarded-For` (behind a reverse proxy).
    pub trust_proxy: bool,
}

impl Config {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            create_key: None,
            max_rooms: 5000,
            max_room_bytes: proto::MAX_ROOM_BYTES,
            max_peers: 64,
            idle_days: 180,
            compact_after: 1024 * 1024,
            trust_proxy: false,
        }
    }
}

type LiveRef = Arc<tokio::sync::Mutex<Live>>;

struct App {
    cfg: Config,
    index: Mutex<Index>,
    live: tokio::sync::Mutex<HashMap<String, LiveRef>>,
    /// Wrong codes per address: (count, window start).
    misses: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    next_peer: AtomicU64,
    active: AtomicUsize,
    shutdown: watch::Sender<bool>,
}

/// Wrong codes an address may try per window.
const MAX_MISSES: u32 = 20;
const MISS_WINDOW: Duration = Duration::from_secs(600);

/// A started relay.
pub struct Running {
    pub addr: SocketAddr,
    app: Arc<App>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    /// Tell everyone the server restarts, stop accepting, save and compact every open room.
    pub async fn shutdown(self) {
        self.app.shutdown.send_replace(true);
        let _ = self.task.await;
        for _ in 0..100 {
            if self.app.active.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let live: Vec<LiveRef> = self.app.live.lock().await.drain().map(|(_, l)| l).collect();
        for l in live {
            if let Err(e) = l.lock().await.compact() {
                log::warn!("event=compact_failed error={e}");
            }
        }
    }
}

fn now() -> u64 {
    rooms::now_secs()
}

/// Start the relay on `listener`.
pub async fn start(listener: tokio::net::TcpListener, cfg: Config) -> std::io::Result<Running> {
    let index = rooms::load_index(&cfg.data_dir)?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let app = Arc::new(App {
        cfg,
        index: Mutex::new(index),
        live: tokio::sync::Mutex::new(HashMap::new()),
        misses: Mutex::new(HashMap::new()),
        next_peer: AtomicU64::new(1),
        active: AtomicUsize::new(0),
        shutdown,
    });
    expire(&app).await;
    let addr = listener.local_addr()?;
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/rooms", post(create_room))
        .route("/rooms/{room}", delete(delete_room))
        .route("/join/{code}", get(join))
        .route("/r/{room}", get(ws))
        .layer(axum::middleware::map_response(close_connection))
        .with_state(app.clone());
    let sweeper = app.clone();
    tokio::spawn(async move {
        let mut rx = sweeper.shutdown.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(6 * 3600)) => expire(&sweeper).await,
                _ = async { let _ = rx.wait_for(|s| *s).await; } => return,
            }
        }
    });
    let task = tokio::spawn(async move {
        let serve = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|s| *s).await;
        });
        if let Err(e) = serve.await {
            log::error!("event=serve_failed error={e}");
        }
    });
    Ok(Running { addr, app, task })
}

/// Plain HTTP answers close their connection (WebSocket upgrades keep theirs).
async fn close_connection(mut resp: Response) -> Response {
    if resp.status() != StatusCode::SWITCHING_PROTOCOLS {
        resp.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    resp
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

fn client_ip(app: &App, headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    if app.cfg.trust_proxy
        && let Some(ip) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).and_then(|v| v.trim().parse().ok())
    {
        return ip;
    }
    peer.ip()
}

async fn expire(app: &App) {
    if app.cfg.idle_days == 0 {
        return;
    }
    let cutoff = now().saturating_sub(app.cfg.idle_days * 86_400);
    let live = app.live.lock().await;
    let old: Vec<Meta> = {
        let index = app.index.lock().unwrap_or_else(PoisonError::into_inner);
        index.rooms.values().filter(|m| m.last_open < cutoff && !live.contains_key(&m.room)).cloned().collect()
    };
    for m in old {
        remove_room(app, &m);
        log::info!("event=expired room={}", m.room);
    }
}

fn remove_room(app: &App, m: &Meta) {
    {
        let mut index = app.index.lock().unwrap_or_else(PoisonError::into_inner);
        index.rooms.remove(&m.room);
        index.codes.remove(&m.code_hash);
    }
    if let Err(e) = std::fs::remove_dir_all(rooms::room_dir(&app.cfg.data_dir, &m.room)) {
        log::warn!("event=remove_failed room={} error={e}", m.room);
    }
}

async fn create_room(State(app): State<Arc<App>>, headers: HeaderMap, Json(req): Json<NewRoom>) -> Response {
    if let Some(key) = &app.cfg.create_key
        && !bearer(&headers).is_some_and(|b| rooms::same(&rooms::sha_hex(b), &rooms::sha_hex(key)))
    {
        return err(StatusCode::UNAUTHORIZED, "Dieser Server braucht einen Schlüssel zum Teilen");
    }
    if !proto::valid_room(&req.room) {
        return err(StatusCode::BAD_REQUEST, "Ungültige Raum-Kennung");
    }
    if req.kind.is_empty() || req.kind.len() > 20 || !req.kind.bytes().all(|b| b.is_ascii_lowercase()) {
        return err(StatusCode::BAD_REQUEST, "Ungültige Art des Artefakts");
    }
    let title: String = req.title.chars().filter(|c| !c.is_control()).take(200).collect();
    let made = (|| -> std::io::Result<(Meta, Created)> {
        let owner = rooms::hex(&rooms::random::<24>()?);
        let token_raw = rooms::random::<24>()?;
        let token = rooms::hex(&token_raw);
        let code = code::from_random(rooms::random::<8>()?);
        let key = CodeKey::new(&code);
        let t = now();
        let meta = Meta {
            room: req.room.clone(),
            kind: req.kind.clone(),
            title: title.clone(),
            created_at: t,
            last_open: t,
            owner_hash: rooms::sha_hex(&owner),
            token_hash: rooms::sha_hex(&token),
            code_hash: key.hash.clone(),
            token_sealed: key.seal(&token_raw),
        };
        Ok((meta, Created { room: req.room.clone(), owner_token: owner, token, code }))
    })();
    let (meta, created) = match tokio::task::spawn_blocking(move || made).await {
        Ok(Ok(m)) => m,
        _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "Der Server konnte keinen Raum anlegen"),
    };
    {
        let mut index = app.index.lock().unwrap_or_else(PoisonError::into_inner);
        if index.rooms.contains_key(&meta.room) {
            return err(StatusCode::CONFLICT, "Dieses Artefakt ist auf dem Server schon geteilt");
        }
        if index.rooms.len() >= app.cfg.max_rooms {
            return err(StatusCode::INSUFFICIENT_STORAGE, "Der Server hat keinen Platz für weitere Räume");
        }
        if index.codes.contains_key(&meta.code_hash) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "Bitte noch einmal versuchen");
        }
        if let Err(e) = rooms::write_meta(&app.cfg.data_dir, &meta) {
            log::error!("event=write_failed room={} error={e}", meta.room);
            return err(StatusCode::INTERNAL_SERVER_ERROR, "Der Server konnte den Raum nicht speichern");
        }
        index.codes.insert(meta.code_hash.clone(), meta.room.clone());
        index.rooms.insert(meta.room.clone(), meta.clone());
    }
    log::info!("event=created room={} kind={}", meta.room, meta.kind);
    (StatusCode::CREATED, Json(created)).into_response()
}

async fn join(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(raw): Path<String>) -> Response {
    let wants_json = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("application/json"));
    let Some(c) = code::normalize(&raw) else {
        return if wants_json { err(StatusCode::NOT_FOUND, "Diesen Code kennt der Server nicht") } else { page("") };
    };
    if !wants_json {
        // A link opened in a browser (or a chat preview): show the code, never the token.
        return page(&code::display(&c));
    }
    let ip = client_ip(&app, &headers, peer);
    {
        let misses = app.misses.lock().unwrap_or_else(PoisonError::into_inner);
        if misses.get(&ip).is_some_and(|(n, since)| *n >= MAX_MISSES && since.elapsed() < MISS_WINDOW) {
            return err(StatusCode::TOO_MANY_REQUESTS, "Zu viele falsche Codes. Bitte später noch einmal.");
        }
    }
    let key = match tokio::task::spawn_blocking(move || CodeKey::new(&c)).await {
        Ok(k) => k,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "Bitte noch einmal versuchen"),
    };
    let meta = {
        let index = app.index.lock().unwrap_or_else(PoisonError::into_inner);
        index.codes.get(&key.hash).and_then(|r| index.rooms.get(r)).cloned()
    };
    let token = meta.as_ref().and_then(|m| key.open(&m.token_sealed).filter(|t| rooms::same(&rooms::sha_hex(t), &m.token_hash)));
    match (meta, token) {
        (Some(m), Some(token)) => Json(JoinInfo { room: m.room, token, kind: m.kind, title: m.title }).into_response(),
        _ => {
            let mut misses = app.misses.lock().unwrap_or_else(PoisonError::into_inner);
            if misses.len() > 100_000 {
                misses.retain(|_, (_, since)| since.elapsed() < MISS_WINDOW);
            }
            let e = misses.entry(ip).or_insert((0, Instant::now()));
            if e.1.elapsed() >= MISS_WINDOW {
                *e = (0, Instant::now());
            }
            e.0 += 1;
            log::info!("event=wrong_code ip={ip}");
            err(StatusCode::NOT_FOUND, "Diesen Code kennt der Server nicht")
        }
    }
}

fn page(code: &str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>AIS.ChatCraft</title></head>\
<body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:3rem auto;padding:0 1rem;line-height:1.5\">\
<h1>Gemeinsam bearbeiten</h1><p>Öffne AIS.ChatCraft, wähle <b>Teilen › Beitreten</b> und füge diesen Link oder den Code ein.</p>\
<p style=\"font-size:1.6rem;font-family:monospace\">{code}</p></body></html>"
    );
    Html(body).into_response()
}

async fn delete_room(State(app): State<Arc<App>>, headers: HeaderMap, Path(room): Path<String>) -> Response {
    let meta = app.index.lock().unwrap_or_else(PoisonError::into_inner).rooms.get(&room).cloned();
    let Some(meta) = meta else { return err(StatusCode::NOT_FOUND, "Diesen Raum gibt es nicht") };
    if !bearer(&headers).is_some_and(|b| rooms::same(&rooms::sha_hex(b), &meta.owner_hash)) {
        return err(StatusCode::FORBIDDEN, "Nur wer den Raum angelegt hat, kann ihn beenden");
    }
    let live = app.live.lock().await.remove(&room);
    if let Some(l) = live {
        let l = l.lock().await;
        for tx in l.peers.values() {
            let _ = tx.try_send(Out::Close(proto::CLOSE_ENDED, "Raum beendet".into()));
        }
    }
    remove_room(&app, &meta);
    log::info!("event=ended room={room}");
    StatusCode::NO_CONTENT.into_response()
}

async fn ws(State(app): State<Arc<App>>, Path(room): Path<String>, Query(q): Query<HashMap<String, String>>, upgrade: WebSocketUpgrade) -> Response {
    let meta = app.index.lock().unwrap_or_else(PoisonError::into_inner).rooms.get(&room).cloned();
    let Some(meta) = meta else { return err(StatusCode::NOT_FOUND, "Diesen Raum gibt es nicht") };
    let token = q.get("token").map(String::as_str).unwrap_or("");
    let hash = rooms::sha_hex(token);
    if token.is_empty() || !(rooms::same(&hash, &meta.token_hash) || rooms::same(&hash, &meta.owner_hash)) {
        return err(StatusCode::FORBIDDEN, "Kein Zugang zu diesem Raum");
    }
    upgrade.max_message_size(proto::MAX_FRAME).max_frame_size(proto::MAX_FRAME).on_upgrade(move |socket| peer(app, room, socket))
}

/// The room, loaded if nobody is in it yet.
async fn open_room(app: &App, room: &str) -> Option<LiveRef> {
    let mut live = app.live.lock().await;
    if let Some(l) = live.get(room) {
        return Some(l.clone());
    }
    let dir = rooms::room_dir(&app.cfg.data_dir, room);
    let max = app.cfg.max_room_bytes;
    let loaded = tokio::task::spawn_blocking(move || Live::load(dir, max)).await;
    let l = match loaded {
        Ok(Ok(l)) => Arc::new(tokio::sync::Mutex::new(l)),
        Ok(Err(e)) => {
            log::error!("event=load_failed room={room} error={e}");
            return None;
        }
        Err(_) => return None,
    };
    let meta = {
        let mut index = app.index.lock().unwrap_or_else(PoisonError::into_inner);
        index.rooms.get_mut(room).map(|m| {
            m.last_open = now();
            m.clone()
        })
    };
    if let Some(m) = meta
        && let Err(e) = rooms::write_meta(&app.cfg.data_dir, &m)
    {
        log::warn!("event=write_failed room={room} error={e}");
    }
    live.insert(room.to_string(), l.clone());
    Some(l)
}

fn broadcast_peers(l: &Live) {
    let msg = proto::peers(l.peers.len() as u32);
    for tx in l.peers.values() {
        let _ = tx.try_send(Out::Bin(msg.clone()));
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn peer(app: Arc<App>, room: String, socket: WebSocket) {
    app.active.fetch_add(1, Ordering::SeqCst);
    let _active = ActiveGuard(&app.active);
    let (mut sink, mut stream) = socket.split();
    let close = |code: u16, reason: &str| WsMessage::Close(Some(CloseFrame { code, reason: reason.to_string().into() }));
    let Some(live) = open_room(&app, &room).await else {
        let _ = sink.send(close(1011, "Raum nicht lesbar")).await;
        return;
    };
    let id = app.next_peer.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::channel::<Out>(1024);
    {
        let mut l = live.lock().await;
        if l.peers.len() >= app.cfg.max_peers {
            drop(l);
            let _ = sink.send(close(proto::CLOSE_BUSY, "Raum ist voll")).await;
            unload_if_empty(&app, &room).await;
            return;
        }
        let _ = tx.try_send(Out::Bin(proto::step1(&l.doc)));
        l.peers.insert(id, tx.clone());
        broadcast_peers(&l);
    }
    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let r = match out {
                Out::Bin(b) => sink.send(WsMessage::Binary(b.into())).await,
                Out::Close(code, reason) => {
                    let _ = sink.send(close(code, &reason)).await;
                    break;
                }
            };
            if r.is_err() {
                break;
            }
        }
    });
    let mut shutdown = app.shutdown.subscribe();
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = async { let _ = shutdown.wait_for(|s| *s).await; } => {
                let _ = tx.send(Out::Close(1012, "Server startet neu".into())).await;
                break;
            }
        };
        match msg {
            Some(Ok(WsMessage::Binary(frame))) => {
                if let Err((code, reason)) = handle(&app, &live, id, &frame).await {
                    let _ = tx.send(Out::Close(code, reason.into())).await;
                    break;
                }
            }
            Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {}
        }
    }
    {
        let mut l = live.lock().await;
        l.peers.remove(&id);
        broadcast_peers(&l);
    }
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
    unload_if_empty(&app, &room).await;
}

async fn unload_if_empty(app: &App, room: &str) {
    let mut map = app.live.lock().await;
    let Some(l) = map.get(room).cloned() else { return };
    let mut l = l.lock().await;
    if l.peers.is_empty() {
        if let Err(e) = l.compact() {
            log::warn!("event=compact_failed room={room} error={e}");
        }
        map.remove(room);
    }
}

async fn handle(app: &App, live: &LiveRef, id: u64, frame: &[u8]) -> Result<(), (u16, &'static str)> {
    let msgs = proto::decode_all(frame).map_err(|_| (proto::CLOSE_INVALID, "Nachricht nicht lesbar"))?;
    let mut l = live.lock().await;
    for msg in msgs {
        match msg {
            Message::Sync(SyncMessage::SyncStep1(sv)) => {
                let reply = proto::step2(&l.doc, &sv);
                if let Some(tx) = l.peers.get(&id) {
                    let _ = tx.try_send(Out::Bin(reply));
                }
            }
            Message::Sync(SyncMessage::SyncStep2(u)) | Message::Sync(SyncMessage::Update(u)) => {
                if l.size() + u.len() > app.cfg.max_room_bytes {
                    let _ = l.compact();
                    if l.size() + u.len() > app.cfg.max_room_bytes {
                        return Err((proto::CLOSE_FULL, "Raum ist zu groß"));
                    }
                }
                let delta = proto::apply(&l.doc, &u).map_err(|_| (proto::CLOSE_INVALID, "Änderung nicht lesbar"))?;
                if let Some(delta) = delta {
                    if let Err(e) = l.append(&delta) {
                        log::error!("event=write_failed error={e}");
                        return Err((1011, "Speichern fehlgeschlagen"));
                    }
                    let out = proto::update(delta);
                    for (pid, tx) in &l.peers {
                        if *pid != id && tx.try_send(Out::Bin(out.clone())).is_err() {
                            // Too slow to keep up: it catches up by state vector on reconnect.
                            let _ = tx.try_send(Out::Close(1013, "Zu langsam".into()));
                        }
                    }
                    if l.log_bytes > app.cfg.compact_after
                        && let Err(e) = l.compact()
                    {
                        log::warn!("event=compact_failed error={e}");
                    }
                }
            }
            Message::Awareness(a) => {
                let out = proto::encode(&Message::Awareness(a));
                if out.len() <= proto::MAX_AWARENESS {
                    for (pid, tx) in &l.peers {
                        if *pid != id {
                            let _ = tx.try_send(Out::Bin(out.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
