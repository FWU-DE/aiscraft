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

    /// The room's WebSocket address (the token rides in the query: browsers cannot set headers).
    pub fn room_ws(&self, room: &str, token: &str) -> String {
        let ws = if let Some(r) = self.http.strip_prefix("https://") { format!("wss://{r}") } else { self.http.replacen("http://", "ws://", 1) };
        format!("{ws}/r/{room}?token={token}")
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
    let said =
        resp.json::<serde_json::Value>().await.ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|s| s.chars().take(200).collect::<String>()));
    let msg = |fallback: &str| said.clone().unwrap_or_else(|| fallback.to_string());
    Err(match status.as_u16() {
        401 | 403 => CollabError::Refused(msg("Der Server lässt das nicht zu (Schlüssel oder Zugang falsch)")),
        404 => CollabError::NotFound(msg("Diesen Code oder Raum kennt der Server nicht")),
        409 => CollabError::Invalid(msg("Dieses Artefakt ist auf dem Server schon geteilt")),
        429 => CollabError::Refused(msg("Zu viele Versuche. Bitte später noch einmal.")),
        _ => CollabError::Protocol(msg(&format!("Der Server antwortet mit {status}"))),
    })
}

async fn json<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let bytes = resp.bytes().await.map_err(net)?;
    if bytes.len() > 64 * 1024 {
        return Err(CollabError::Protocol("Antwort des Servers ist zu groß".into()));
    }
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
}
