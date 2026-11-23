use std::sync::{Arc, Mutex};
use std::time::Duration;

use aiscraft_collab::{NewRoom, Options, Server, Session, Status, api};
use yrs::{GetString, Text, Transact};

use super::*;

fn data_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aiscraft-collab-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

async fn relay(cfg: Config) -> Running {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    start(l, cfg).await.unwrap()
}

async fn relay_at(addr: SocketAddr, cfg: Config) -> Running {
    let l = tokio::net::TcpListener::bind(addr).await.unwrap();
    start(l, cfg).await.unwrap()
}

fn text_state(s: &str) -> Vec<u8> {
    let doc = yrs::Doc::new();
    doc.get_or_insert_text("content").insert(&mut doc.transact_mut(), 0, s);
    proto::full_state(&doc)
}

/// Append `s` to the text in `state`, as a new edit.
fn edit(state: &[u8], s: &str) -> Vec<u8> {
    let doc = yrs::Doc::new();
    proto::apply(&doc, state).unwrap();
    let t = doc.get_or_insert_text("content");
    let len = t.len(&doc.transact());
    t.insert(&mut doc.transact_mut(), len, s);
    proto::full_state(&doc)
}

fn text_of(state: &[u8]) -> String {
    let doc = yrs::Doc::new();
    proto::apply(&doc, state).unwrap();
    doc.get_or_insert_text("content").get_string(&doc.transact())
}

type Seen = Arc<Mutex<Vec<u8>>>;

fn session(server: &Server, room: &str, token: &str, initial: &[u8]) -> (Session, Seen) {
    let seen: Seen = Arc::new(Mutex::new(initial.to_vec()));
    let s2 = seen.clone();
    let opts = Options { min_backoff: Duration::from_millis(100), max_backoff: Duration::from_millis(400) };
    let s = Session::start(server, room, token, initial, Arc::new(move |full| *s2.lock().unwrap() = full), Arc::new(|_| {}), opts).unwrap();
    (s, seen)
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_join_edit_restart_and_end() {
    let dir = data_dir("main");
    let r = relay(Config::new(dir.clone())).await;
    let addr = r.addr;
    let server = Server::parse(&format!("http://{addr}")).unwrap();

    // Health check and a plain answer that closes its connection.
    let resp = reqwest::get(format!("http://{addr}/healthz")).await.unwrap();
    assert_eq!(resp.headers().get("connection").unwrap(), "close");
    assert_eq!(resp.text().await.unwrap(), "ok");

    let created = api::create_room(&server, None, &NewRoom { room: "doc-1".into(), kind: "document".into(), title: "Plan".into() }).await.unwrap();
    assert!(api::create_room(&server, None, &NewRoom { room: "doc-1".into(), kind: "document".into(), title: "x".into() }).await.is_err(), "taken");
    assert!(api::create_room(&server, None, &NewRoom { room: "../x".into(), kind: "document".into(), title: "x".into() }).await.is_err());

    // A browser sees the code, never the token.
    let html = reqwest::get(server.join_link(&created.code)).await.unwrap().text().await.unwrap();
    assert!(html.contains(&code::display(&created.code)) && !html.contains(&created.token));

    let info = api::resolve(&server, &code::display(&created.code).to_lowercase()).await.unwrap();
    assert_eq!((info.room.as_str(), info.kind.as_str(), info.title.as_str()), ("doc-1", "document", "Plan"));
    assert_eq!(info.token, created.token);
    assert!(matches!(api::resolve(&server, "00000-00000").await, Err(aiscraft_collab::CollabError::NotFound(_))));

    let (a, a_seen) = session(&server, "doc-1", &created.owner_token, &text_state("Hallo"));
    assert!(a.wait_synced(Duration::from_secs(5)).await);
    let (b, b_seen) = session(&server, "doc-1", &info.token, &[]);
    assert!(b.wait_synced(Duration::from_secs(5)).await);
    until("b gets a's text", || text_of(&b_seen.lock().unwrap()) == "Hallo").await;
    until("two in the room", || matches!(a.status(), Status::Online { peers: 2, .. })).await;

    // Both edit; both converge.
    let mine = b.push(&edit(&b.state(), " Welt")).unwrap();
    assert_eq!(text_of(&mine), "Hallo Welt");
    until("a gets b's edit", || text_of(&a_seen.lock().unwrap()) == "Hallo Welt").await;

    // The relay restarts; while it is down, a edits; afterwards b has it.
    r.shutdown().await;
    until("a notices", || !matches!(a.status(), Status::Online { .. })).await;
    a.push(&edit(&a.state(), "!")).unwrap();
    let r = relay_at(addr, Config::new(dir.clone())).await;
    until("b gets the offline edit", || text_of(&b_seen.lock().unwrap()) == "Hallo Welt!").await;

    // Wrong token: refused, and the session gives up.
    let (bad, _) = session(&server, "doc-1", "0000", &[]);
    until("refused", || matches!(bad.status(), Status::Ended { .. })).await;

    // Only the owner ends the room; everyone in it is told.
    assert!(api::delete_room(&server, "doc-1", &info.token).await.is_err());
    api::delete_room(&server, "doc-1", &created.owner_token).await.unwrap();
    until("b sees the end", || matches!(b.status(), Status::Ended { .. })).await;
    assert!(api::resolve(&server, &created.code).await.is_err());
    r.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_limits_and_hostile_frames() {
    let dir = data_dir("limits");
    let mut cfg = Config::new(dir.clone());
    cfg.create_key = Some("geheim".into());
    cfg.max_room_bytes = 4096;
    let r = relay(cfg).await;
    let server = Server::parse(&format!("http://{}", r.addr)).unwrap();
    let room = NewRoom { room: "b1".into(), kind: "board".into(), title: "Board".into() };
    assert!(matches!(api::create_room(&server, None, &room).await, Err(aiscraft_collab::CollabError::Refused(_))));
    assert!(api::create_room(&server, Some("falsch"), &room).await.is_err());
    let created = api::create_room(&server, Some("geheim"), &room).await.unwrap();

    // Garbage over the socket closes that connection, the server stays up.
    let url = server.room_ws("b1", &created.token);
    let (mut ws, _) = tokio_tungstenite_connect(&url).await;
    ws.send(tokio_tungstenite::tungstenite::Message::binary(vec![0u8, 1, 255, 255, 255, 255, 15])).await.unwrap();
    let mut closed = false;
    while let Some(m) = ws.next().await {
        if let Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) = m {
            closed = true;
            break;
        }
    }
    assert!(closed);

    // Growing past the room limit ends the session with a reason.
    let (s, _) = session(&server, "b1", &created.token, &[]);
    assert!(s.wait_synced(Duration::from_secs(5)).await);
    s.push(&text_state(&"x".repeat(8000))).unwrap();
    until("too large", || matches!(s.status(), Status::Ended { .. })).await;
    assert_eq!(reqwest::get(format!("http://{}/healthz", r.addr)).await.unwrap().status(), 200);

    // Too many wrong codes from one address: refused for a while.
    let mut last = None;
    for _ in 0..=MAX_MISSES {
        last = Some(api::resolve(&server, "ZZZZZ-ZZZZZ").await);
    }
    assert!(matches!(last, Some(Err(aiscraft_collab::CollabError::Refused(_)))));
    r.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

async fn tokio_tungstenite_connect(url: &str) -> (tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, ()) {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    (ws, ())
}
