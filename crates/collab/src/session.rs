//! One artifact kept in sync with its room.
//!
//! The session keeps a mirror `yrs::Doc` of the artifact. Local saves are merged into the mirror
//! and what was new goes to the relay as a y-sync update; updates from the relay are merged into
//! the mirror and handed to the owner of the session as the new full state. While offline, local
//! saves only reach the mirror: on reconnect both sides send their state vector (step 1) and
//! answer the other's with exactly what it lacks (step 2), so nothing has to be queued.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{self, Message as WsMessage, protocol::WebSocketConfig};
use yrs::sync::{Message, SyncMessage};

use crate::proto::{self, CLOSE_BUSY, CLOSE_ENDED, CLOSE_FULL, PEERS_TAG};
use crate::{CollabError, Result, Server};

/// Where a session stands, for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Status {
    Connecting,
    /// Connected; `synced` once the relay has answered our state vector.
    Online {
        synced: bool,
        peers: u32,
    },
    /// Lost or not reached; the session tries again by itself.
    Offline {
        reason: String,
    },
    /// Over for good (room ended, access refused): the session no longer tries.
    Ended {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Options {
    /// First wait before reconnecting; doubles up to `max_backoff`.
    pub min_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self { min_backoff: Duration::from_secs(1), max_backoff: Duration::from_secs(30) }
    }
}

/// Called with the artifact's new full state after a change from the room.
pub type OnRemote = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
/// Called when the status changes.
pub type OnStatus = Arc<dyn Fn(&Status) + Send + Sync>;

struct Shared {
    doc: Mutex<yrs::Doc>,
    status: Mutex<Status>,
    synced: watch::Sender<bool>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    on_remote: OnRemote,
    on_status: OnStatus,
}

impl Shared {
    fn set_status(&self, s: Status) {
        {
            let mut cur = self.status.lock().unwrap_or_else(PoisonError::into_inner);
            if *cur == s {
                return;
            }
            *cur = s.clone();
        }
        if !matches!(s, Status::Online { synced: true, .. }) {
            self.synced.send_replace(false);
        }
        (self.on_status)(&s);
    }

    fn with_doc<R>(&self, f: impl FnOnce(&yrs::Doc) -> R) -> R {
        let guard = self.doc.lock().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }
}

/// A running session. Dropping it (or [`Session::stop`]) disconnects.
pub struct Session {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
}

impl Session {
    /// Start syncing `initial` (the artifact's full state) with `room`. Needs a Tokio runtime.
    pub fn start(server: &Server, room: &str, token: &str, initial: &[u8], on_remote: OnRemote, on_status: OnStatus, opts: Options) -> Result<Session> {
        if !proto::valid_room(room) {
            return Err(CollabError::Invalid("Ungültige Raum-Kennung".into()));
        }
        if token.is_empty() || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(CollabError::Invalid("Ungültiger Zugang zum Raum".into()));
        }
        let doc = yrs::Doc::with_options(yrs::Options { offset_kind: yrs::OffsetKind::Utf16, ..yrs::Options::default() });
        if !initial.is_empty() {
            proto::apply(&doc, initial).map_err(|e| CollabError::Invalid(format!("Artefakt beschädigt: {e}")))?;
        }
        let handle = tokio::runtime::Handle::try_current().map_err(|_| CollabError::Invalid("Keine Laufzeit für die Zusammenarbeit".into()))?;
        let (out, out_rx) = mpsc::unbounded_channel();
        let (synced, _) = watch::channel(false);
        let shared = Arc::new(Shared { doc: Mutex::new(doc), status: Mutex::new(Status::Connecting), synced, out, on_remote, on_status });
        let (stop, stop_rx) = watch::channel(false);
        let url = server.room_ws(room, token);
        handle.spawn(run(shared.clone(), url, stop_rx, out_rx, opts));
        Ok(Session { shared, stop })
    }

    /// Merge a local full state into the session; what is new goes to the room. Returns the
    /// merged full state (local and everything from the room), to store.
    pub fn push(&self, full: &[u8]) -> Result<Vec<u8>> {
        let (delta, merged) = self.shared.with_doc(|doc| {
            let delta = proto::apply(doc, full).map_err(|e| CollabError::Invalid(format!("Artefakt beschädigt: {e}")))?;
            Ok::<_, CollabError>((delta, proto::full_state(doc)))
        })?;
        if let Some(d) = delta {
            // Offline: the mirror has it; the next handshake sends it.
            let _ = self.shared.out.send(d);
        }
        Ok(merged)
    }

    /// The full state: local and everything from the room.
    pub fn state(&self) -> Vec<u8> {
        self.shared.with_doc(proto::full_state)
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Wait until the room has answered our state (or `timeout` passes).
    pub async fn wait_synced(&self, timeout: Duration) -> bool {
        let mut rx = self.shared.synced.subscribe();
        tokio::time::timeout(timeout, rx.wait_for(|s| *s)).await.is_ok_and(|r| r.is_ok())
    }

    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

enum Exit {
    Stop,
    Ended(String),
    Lost(String),
}

async fn run(shared: Arc<Shared>, url: String, mut stop: watch::Receiver<bool>, mut out: mpsc::UnboundedReceiver<Vec<u8>>, opts: Options) {
    let mut backoff = opts.min_backoff;
    loop {
        if *stop.borrow() {
            return;
        }
        shared.set_status(Status::Connecting);
        let config = WebSocketConfig::default().max_message_size(Some(proto::MAX_FRAME)).max_frame_size(Some(proto::MAX_FRAME));
        let connect = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false));
        let res = tokio::select! {
            r = connect => r,
            _ = stop.changed() => return,
        };
        let reason = match res {
            Ok(Ok((ws, _))) => {
                backoff = opts.min_backoff;
                match connected(&shared, ws, &mut stop, &mut out).await {
                    Exit::Stop => return,
                    Exit::Ended(reason) => {
                        shared.set_status(Status::Ended { reason });
                        return;
                    }
                    Exit::Lost(reason) => reason,
                }
            }
            Ok(Err(tungstenite::Error::Http(resp))) => {
                let reason = match resp.status().as_u16() {
                    401 | 403 => "Der Zugang zu diesem Raum gilt nicht mehr".to_string(),
                    404 | 410 => "Diesen Raum gibt es nicht mehr".to_string(),
                    s => format!("Der Server antwortet mit {s}"),
                };
                if matches!(resp.status().as_u16(), 401 | 403 | 404 | 410) {
                    shared.set_status(Status::Ended { reason });
                    return;
                }
                reason
            }
            Ok(Err(_)) => "Server nicht erreichbar".to_string(),
            Err(_) => "Der Server antwortet nicht".to_string(),
        };
        shared.set_status(Status::Offline { reason });
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = stop.changed() => return,
        }
        backoff = (backoff * 2).min(opts.max_backoff);
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connected(shared: &Shared, ws: Ws, stop: &mut watch::Receiver<bool>, out: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> Exit {
    let (mut sink, mut stream) = ws.split();
    // What piled up while offline is in the mirror; the handshake below carries it.
    while out.try_recv().is_ok() {}
    let mut peers = 0;
    let mut got_step2 = false;
    shared.set_status(Status::Online { synced: false, peers });
    if sink.send(WsMessage::binary(shared.with_doc(proto::step1))).await.is_err() {
        return Exit::Lost("Verbindung getrennt".into());
    }
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    loop {
        tokio::select! {
            _ = stop.changed() => {
                let _ = sink.send(WsMessage::Close(None)).await;
                return Exit::Stop;
            }
            u = out.recv() => {
                let Some(u) = u else { return Exit::Stop };
                if sink.send(WsMessage::binary(proto::update(u))).await.is_err() {
                    return Exit::Lost("Verbindung getrennt".into());
                }
            }
            _ = ping.tick() => {
                if sink.send(WsMessage::Ping(Vec::new().into())).await.is_err() {
                    return Exit::Lost("Verbindung getrennt".into());
                }
            }
            m = stream.next() => match m {
                None => return Exit::Lost("Verbindung getrennt".into()),
                Some(Err(_)) => return Exit::Lost("Verbindung abgebrochen".into()),
                Some(Ok(WsMessage::Binary(frame))) => {
                    let msgs = match proto::decode_all(&frame) {
                        Ok(m) => m,
                        Err(_) => return Exit::Lost("Der Server hat Unlesbares geschickt".into()),
                    };
                    for msg in msgs {
                        got_step2 |= matches!(msg, Message::Sync(SyncMessage::SyncStep2(_)));
                        match msg {
                            Message::Sync(SyncMessage::SyncStep1(sv)) => {
                                let reply = shared.with_doc(|d| proto::step2(d, &sv));
                                if sink.send(WsMessage::binary(reply)).await.is_err() {
                                    return Exit::Lost("Verbindung getrennt".into());
                                }
                            }
                            Message::Sync(SyncMessage::SyncStep2(u)) | Message::Sync(SyncMessage::Update(u)) => {
                                let applied = shared.with_doc(|d| proto::apply(d, &u).map(|delta| delta.map(|_| proto::full_state(d))));
                                match applied {
                                    Ok(Some(full)) => (shared.on_remote)(full),
                                    Ok(None) => {}
                                    Err(e) => log::warn!("Zusammenarbeit: Änderung vom Server nicht lesbar: {e}"),
                                }
                            }
                            Message::Custom(PEERS_TAG, data) => {
                                if let Some(n) = proto::read_peers(&data) {
                                    peers = n;
                                }
                            }
                            _ => {}
                        }
                    }
                    // Synced once the relay has answered our step 1 with its step 2.
                    shared.set_status(Status::Online { synced: got_step2, peers });
                    if got_step2 {
                        shared.synced.send_replace(true);
                    }
                }
                Some(Ok(WsMessage::Close(frame))) => {
                    let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(1000);
                    return match code {
                        CLOSE_ENDED => Exit::Ended("Der Raum wurde beendet".into()),
                        CLOSE_FULL => Exit::Ended("Das Artefakt ist für den Server zu groß geworden".into()),
                        CLOSE_BUSY => Exit::Lost("Der Raum ist voll".into()),
                        1012 => Exit::Lost("Der Server startet neu".into()),
                        _ => Exit::Lost("Verbindung getrennt".into()),
                    };
                }
                Some(Ok(_)) => {}
            }
        }
    }
}
