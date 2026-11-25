//! The relay's HTTP side. Rooms are made with `POST /rooms`, codes resolved with
//! `GET /join/<code>`, rooms ended with `DELETE /rooms/<room>`; a room's WebSocket is
//! `GET /r/<room>?token=…`. Error bodies are `{"error": "<German message>"}`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{CollabError, Result, code};

/// A relay's address, kept as its HTTP base (no trailing slash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    http: String,
}

impl Server {
    /// `https://collab.example.org`, `wss://…`, `http://127.0.0.1:47910/collab`; no scheme means https.
    pub fn parse(url: &str) -> Result<Server> {
        let url = url.trim().trim_end_matches('/');
        let bad =
            || CollabError::Invalid(format!("„{}“ ist keine Server-Adresse (z. B. https://collab.example.org)", url.chars().take(80).collect::<String>()));
        let (scheme, rest) = match url.split_once("://") {
            Some((s, r)) => (s.to_ascii_lowercase(), r),
            None => ("https".to_string(), url),
        };
        let scheme = match scheme.as_str() {
            "http" | "ws" => "http",
            "https" | "wss" => "https",
            _ => return Err(bad()),
        };
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty()
            || host.ends_with(':')
            || host.starts_with(':')
            || rest.len() > 200
            || rest.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '@' | '\\'))
        {
            return Err(bad());
        }
        Ok(Server { http: format!("{scheme}://{rest}") })
    }

    pub fn http(&self) -> &str {
        &self.http
    }

    /// The room's WebSocket address; the app sends the token as `Authorization: Bearer`.
    pub fn room_ws_url(&self, room: &str) -> String {
        let ws = if let Some(r) = self.http.strip_prefix("https://") { format!("wss://{r}") } else { self.http.replacen("http://", "ws://", 1) };
        format!("{ws}/r/{room}")
    }

    /// The room's WebSocket address with the token in the query, for browsers (they cannot set
    /// headers) and older relays, which read only the query.
    pub fn room_ws(&self, room: &str, token: &str) -> String {
        format!("{}?token={token}", self.room_ws_url(room))
    }

    /// The link to hand on: `<server>/join/<code>`. Opened in a browser it shows the code.
    pub fn join_link(&self, code: &str) -> String {
        format!("{}/join/{}", self.http, code::display(code))
    }
}

/// A join link's server and code, if `link` is one.
pub fn parse_link(link: &str) -> Option<(Server, String)> {
    let link = link.trim().trim_end_matches('/');
    let (base, c) = link.rsplit_once("/join/")?;
    let code = code::normalize(c)?;
    Some((Server::parse(base).ok()?, code))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRoom {
    pub room: String,
    /// `document`, `board`, …: what the joiner's app should make of it.
    pub kind: String,
    pub title: String,
}

/// A new room: the owner keeps `owner_token`, everyone joins with `code` (or `token`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Created {
    pub room: String,
    pub owner_token: String,
    pub token: String,
    /// Ten symbols, without the dash.
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinInfo {
    pub room: String,
    pub token: String,
    pub kind: String,
    pub title: String,
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder().timeout(Duration::from_secs(20)).pool_max_idle_per_host(0).build().map_err(|e| CollabError::Network(format!("HTTP-Client: {e}")))
}

fn net(e: reqwest::Error) -> CollabError {
    CollabError::Network(if e.is_timeout() {
        "Der Server für die Zusammenarbeit antwortet nicht".to_string()
    } else {
        "Der Server für die Zusammenarbeit ist nicht erreichbar".to_string()
    })
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let said = capped(resp, 16 * 1024)
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|s| s.chars().take(200).collect::<String>()));
    let msg = |fallback: &str| said.clone().unwrap_or_else(|| fallback.to_string());
    Err(match status.as_u16() {
        401 | 403 => CollabError::Refused(msg("Der Server lässt das nicht zu (Schlüssel oder Zugang falsch)")),
        404 => CollabError::NotFound(msg("Diesen Code oder Raum kennt der Server nicht")),
        409 => CollabError::Invalid(msg("Dieses Artefakt ist auf dem Server schon geteilt")),
        429 => CollabError::Refused(msg("Zu viele Versuche. Bitte später noch einmal.")),
        _ => CollabError::Protocol(msg(&format!("Der Server antwortet mit {status}"))),
    })
}

/// The body, read piece by piece and refused once it grows past `max` bytes.
async fn capped(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        if out.len().saturating_add(chunk.len()) > max {
            return Err(CollabError::Protocol("Antwort des Servers ist zu groß".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

async fn json<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let bytes = capped(resp, 64 * 1024).await?;
    serde_json::from_slice(&bytes).map_err(|_| CollabError::Protocol("Antwort des Servers ist unlesbar".into()))
}

/// Make a room for an artifact. `key` is the server's key, when it asks for one.
pub async fn create_room(server: &Server, key: Option<&str>, room: &NewRoom) -> Result<Created> {
    let mut req = client()?.post(format!("{}/rooms", server.http)).json(room);
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let created: Created = json(check(req.send().await.map_err(net)?).await?).await?;
    if created.room != room.room || code::normalize(&created.code).is_none() {
        return Err(CollabError::Protocol("Der Server hat einen anderen Raum angelegt".into()));
    }
    Ok(created)
}

/// What a code opens.
pub async fn resolve(server: &Server, code: &str) -> Result<JoinInfo> {
    let code = code::normalize(code).ok_or_else(|| CollabError::Invalid("Der Code hat nicht die Form XXXXX-XXXXX".into()))?;
    let resp = client()?.get(format!("{}/join/{code}", server.http)).header("accept", "application/json").send().await.map_err(net)?;
    let info: JoinInfo = json(check(resp).await?).await?;
    if !crate::proto::valid_room(&info.room) || info.token.is_empty() || info.token.len() > 200 {
        return Err(CollabError::Protocol("Antwort des Servers ist unlesbar".into()));
    }
    Ok(info)
}

/// End a room for everyone (owner only).
pub async fn delete_room(server: &Server, room: &str, owner_token: &str) -> Result<()> {
    let resp = client()?.delete(format!("{}/rooms/{room}", server.http)).bearer_auth(owner_token).send().await.map_err(net)?;
    check(resp).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_and_links() {
        let s = Server::parse("wss://collab.example.org/").unwrap();
        assert_eq!(s.http(), "https://collab.example.org");
        assert_eq!(s.room_ws("r1", "t"), "wss://collab.example.org/r/r1?token=t");
        assert_eq!(Server::parse("collab.example.org").unwrap().http(), "https://collab.example.org");
        let local = Server::parse("http://127.0.0.1:47910/x").unwrap();
        assert_eq!(local.room_ws("r", "t"), "ws://127.0.0.1:47910/x/r/r?token=t");
        for bad in ["", "ftp://x", "https://", "https://a b", "https://u@host", "https://h?x=1"] {
            assert!(Server::parse(bad).is_err(), "{bad}");
        }
        let link = s.join_link("ABCDEFGHJK");
        assert_eq!(link, "https://collab.example.org/join/ABCDE-FGHJK");
        let (srv, code) = parse_link(&link).unwrap();
        assert_eq!((srv, code.as_str()), (s, "ABCDEFGHJK"));
        assert!(parse_link("ABCDE-FGHJK").is_none());
        assert!(parse_link("https://x/join/nope").is_none());
    }

    /// A relay that answers with an endless body: errors and answers are read only up to their
    /// cap, never whole.
    #[tokio::test]
    async fn endless_answers_are_cut_off() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let status = if buf.get(..n).is_some_and(|b| b.starts_with(b"DELETE")) { "500 Internal Server Error" } else { "200 OK" };
                    let head = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{{\"error\": \"");
                    if s.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    let chunk = vec![b'x'; 64 * 1024];
                    // 64 MiB at most, so a client that reads everything still ends the test.
                    for _ in 0..1024 {
                        if s.write_all(&chunk).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let server = Server::parse(&format!("http://{addr}")).unwrap();
        let e = delete_room(&server, "r1", "t").await.unwrap_err();
        assert!(matches!(e, CollabError::Protocol(ref m) if m.contains("500")), "{e:?}");
        let e = resolve(&server, "ABCDE-FGHJK").await.unwrap_err();
        assert!(matches!(e, CollabError::Protocol(ref m) if m.contains("zu groß")), "{e:?}");
    }
}
