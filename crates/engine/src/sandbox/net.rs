//! Network for the workspace (#106): what a command in the private computer may reach.
//!
//! Three modes ([`NetMode`]): *Kein Netz* (the default, `--network none`), *Nur erlaubte Seiten*
//! (the list per profile) and *Ganzes Internet* (only after a warning). With network, a
//! container still gets no route out: it sits on an internal Docker network of its own run
//! whose only neighbour is the relay ([`RELAY_SCRIPT`]). The relay accepts TCP connections on
//! port 3128 and hands each one over its stdin/stdout ([`serve`], frames of [`frame`]) to this
//! engine on the host. Here [`handle`] reads the proxy request (HTTP CONNECT for HTTPS,
//! absolute-form for HTTP), checks the name against the list ([`Policy::admit`]), resolves it
//! and checks every address it resolves to, checks the TLS server name (SNI) or the `Host`
//! header against the name, connects to exactly the checked address and counts the bytes.
//!
//! Always blocked, in every mode and for every name on the list:
//! - private and local addresses ([`blocked`]): RFC 1918, 127/8, 169.254/16 with the cloud
//!   metadata address, 100.64/10, IPv6 ULA and link-local, multicast; a name that resolves to
//!   one of them is refused (no DNS rebinding);
//! - ports other than 80 and 443;
//! - UDP (QUIC, DNS) and ICMP: the relay carries only TCP streams, the internal network has no
//!   route, and the private computer drops everything from its bridges ([`NET_UP_SCRIPT`]).
//!
//! The list lives in the engine's settings; nothing in the private computer, and no tool of the
//! model, can change it. The browser of #107 can use the same pieces: a [`Policy`], [`open`]
//! for a relay on a network of its own, and [`handle`] for any stream.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use super::container as k;
use super::runtime::SandboxRuntime;

/// How a command in the workspace reaches the network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum NetMode {
    /// `--network none`: neither the internet nor the school network.
    #[default]
    #[serde(rename = "none")]
    Off,
    /// Only the sites on the list, through the app's proxy.
    #[serde(rename = "list")]
    List,
    /// Every public site, through the app's proxy (private addresses stay blocked).
    #[serde(rename = "internet")]
    Internet,
}

impl NetMode {
    /// `"none"`, `"list"`, `"internet"`; `false`/`true` from v0.3.0's switch (`true` becomes the
    /// list, the safer of the two modes with network).
    pub fn parse(v: &Value) -> Option<NetMode> {
        match v {
            Value::Bool(false) => Some(NetMode::Off),
            Value::Bool(true) => Some(NetMode::List),
            Value::String(s) => match s.as_str() {
                "none" | "off" => Some(NetMode::Off),
                "list" => Some(NetMode::List),
                "internet" => Some(NetMode::Internet),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            NetMode::Off => "none",
            NetMode::List => "list",
            NetMode::Internet => "internet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NetMode::Off => "Kein Netz",
            NetMode::List => "Nur erlaubte Seiten",
            NetMode::Internet => "Ganzes Internet",
        }
    }
}

impl<'de> Deserialize<'de> for NetMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(NetMode::parse(&Value::deserialize(d)?).unwrap_or_default())
    }
}

/// What the settings say about each mode.
pub const TEXT_OFF: &str = "Kein Netz: Befehle erreichen weder das Internet noch dein Schulnetz.";
/// Shown under the heading in every mode: „Netz“ means the public internet, nothing local.
pub const TEXT_SCOPE: &str = "„Netz“ heißt hier nur öffentliches Internet. Nie erreichbar: dein Schul- und Heimnetz, der Router, dein eigener Computer und seine Dienste (z. B. Ollama), andere WSL-Rechner und Cloud-Metadaten.";
pub const TEXT_LIST: &str = "Nur die Seiten auf der Liste, über den Wächter von AIS.ChatCraft; jede Verbindung steht im Aktionsprotokoll. Schulnetz, Router und Cloud-Metadaten bleiben gesperrt, ebenso UDP und Ping.";
pub const WARNING_INTERNET: &str = "Ganzes Internet: Befehle können jede öffentliche Seite erreichen und Dateien aus dem Arbeitsordner dorthin schicken. Schulnetz, Router und Cloud-Metadaten bleiben gesperrt, jede Verbindung steht im Aktionsprotokoll. Nur einschalten, solange du es brauchst; sonst lieber einzelne Seiten erlauben.";
/// How the teacher allows a site (in every refusal the model sees).
pub const HOW_TO_ALLOW: &str = "Freigeben kann nur die Person: Einstellungen › Fähigkeiten › Eigener Rechner › Erlaubte Seiten.";

/// One-tap suggestions in the settings: a name and its domains.
pub const SUGGESTIONS: &[(&str, &[&str])] = &[
    ("Python-Pakete (pip)", &["pypi.org", "files.pythonhosted.org"]),
    ("GitHub", &["github.com", "codeload.github.com", "raw.githubusercontent.com", "objects.githubusercontent.com"]),
];

/// The most entries on a list.
pub const MAX_LIST: usize = 200;

/// A domain as the list keeps it: lower case, without a trailing dot; `*.example.org` stands for
/// every name below example.org (not example.org itself). A pasted address (`https://x.org/a`)
/// keeps only its name. IP addresses, ports and names without a dot are refused.
pub fn normalize_domain(raw: &str) -> std::result::Result<String, String> {
    let s = raw.trim().to_ascii_lowercase();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let s = s.split(['/', '?', '#']).next().unwrap_or("").trim_end_matches('.');
    let (wild, name) = match s.strip_prefix("*.") {
        Some(r) => (true, r),
        None => (false, s),
    };
    if name.is_empty() {
        return Err("Keine Domain angegeben (z. B. pypi.org oder *.example.org).".into());
    }
    if name.contains(':') || name.contains('@') {
        return Err(format!("{raw}: bitte nur den Namen, ohne Port oder Benutzer (z. B. pypi.org)."));
    }
    if !name.is_ascii() {
        return Err(format!("{raw}: Namen mit Umlauten bitte in der xn--Schreibweise (Punycode) eintragen."));
    }
    if name.len() > 253 || !name.contains('.') {
        return Err(format!("{raw}: keine vollständige Domain (z. B. pypi.org)."));
    }
    let labels: Vec<&str> = name.split('.').collect();
    for l in &labels {
        if l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-') || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!("{raw}: keine gültige Domain."));
        }
    }
    if labels.last().is_some_and(|l| l.chars().all(|c| c.is_ascii_digit())) {
        return Err(format!("{raw}: IP-Adressen lassen sich nicht erlauben, nur Namen."));
    }
    Ok(if wild { format!("*.{name}") } else { name.to_string() })
}

/// Whether `host` is on the list (exact, or below a `*.` entry).
pub fn on_list(list: &[String], host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    list.iter().any(|e| match e.strip_prefix("*.") {
        Some(base) => h.strip_suffix(base).is_some_and(|p| p.len() > 1 && p.ends_with('.')),
        None => *e == h,
    })
}

/// Why an address is never reachable from the workspace, or `None` for a public address.
pub fn blocked(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(a) => blocked_v4(a),
        IpAddr::V6(a) => {
            if let Some(m) = a.to_ipv4_mapped() {
                return blocked_v4(m);
            }
            let [s0, s1, s2, s3, s4, s5, s6, s7] = a.segments();
            let embedded = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
            if a.is_unspecified() || a.is_loopback() {
                Some("lokale Adresse")
            } else if s0 & 0xfe00 == 0xfc00 {
                Some("privates Netz (IPv6 ULA)")
            } else if s0 & 0xffc0 == 0xfe80 || s0 & 0xffc0 == 0xfec0 {
                Some("link-local (IPv6)")
            } else if s0 & 0xff00 == 0xff00 {
                Some("Multicast")
            } else if s0 == 0x64 && s1 == 0xff9b {
                // NAT64: the IPv4 address in the last 32 bits counts.
                blocked_v4(embedded(s6, s7))
            } else if s0 == 0x2002 {
                // 6to4: the IPv4 address in bits 16..48.
                blocked_v4(embedded(s1, s2))
            } else if s0 == 0 && s1 == 0 && s2 == 0 && s3 == 0 && s4 == 0 && s5 == 0 {
                // IPv4-compatible (deprecated): ::a.b.c.d
                blocked_v4(embedded(s6, s7)).or(Some("veraltete IPv6-Form"))
            } else {
                None
            }
        }
    }
}

fn blocked_v4(a: Ipv4Addr) -> Option<&'static str> {
    let [o0, o1, o2, _] = a.octets();
    match (o0, o1, o2) {
        (0, ..) => Some("keine gültige Zieladresse"),
        (10, ..) => Some("privates Netz (10/8)"),
        (100, b, _) if b & 0xc0 == 64 => Some("Netz des Anbieters (100.64/10)"),
        (127, ..) => Some("lokale Adresse (127/8)"),
        (169, 254, _) => Some("link-local und Cloud-Metadaten (169.254/16)"),
        (172, b, _) if b & 0xf0 == 16 => Some("privates Netz (172.16/12)"),
        (192, 168, _) => Some("privates Netz (192.168/16)"),
        (192, 0, 0) => Some("reservierte Adresse (192.0.0/24)"),
        (198, b, _) if b & 0xfe == 18 => Some("Testnetz (198.18/15)"),
        (a, ..) if a >= 224 => Some("Multicast oder reserviert"),
        _ => None,
    }
}

/// Names and addresses for tests: no test resolves a real name or leaves the computer.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct TestNet {
    pub hosts: HashMap<String, Vec<IpAddr>>,
    /// Where every admitted connection goes (a local server).
    pub dial: Option<SocketAddr>,
}

/// What the engine checks every connection against: fresh for each run, so a change of the
/// list holds from the next command on.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub mode: NetMode,
    pub allow: Vec<String>,
    #[cfg(test)]
    pub(crate) test: Option<Arc<TestNet>>,
}

impl Policy {
    pub fn new(mode: NetMode, allow: Vec<String>) -> Policy {
        // In tests the struct has a field more (`test`); the update fills it.
        #[allow(clippy::needless_update)]
        let p = Policy { mode, allow, ..Policy::default() };
        p
    }

    /// Whether `host:port` may be reached, and the checked addresses to connect to. `Err` is
    /// the refusal for the person and the model.
    pub async fn admit(&self, host: &str, port: u16) -> std::result::Result<Vec<SocketAddr>, String> {
        if self.mode == NetMode::Off {
            return Err("Der Arbeitsbereich hat kein Netz.".into());
        }
        if port != 80 && port != 443 {
            return Err(format!("{host}:{port}: nur Webseiten (Port 80 und 443) sind erreichbar."));
        }
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<IpAddr>() {
            if let Some(why) = blocked(ip) {
                return Err(format!("{ip}: {why} – immer gesperrt."));
            }
            if self.mode == NetMode::List {
                return Err(format!("{ip}: Adressen ohne Namen stehen nie auf der Liste erlaubter Seiten. {HOW_TO_ALLOW}"));
            }
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        if normalize_domain(&host).ok().as_deref() != Some(host.as_str()) {
            return Err(format!("„{}“ ist kein gültiger Name.", host.chars().take(80).collect::<String>()));
        }
        if self.mode == NetMode::List && !on_list(&self.allow, &host) {
            return Err(format!("{host} ist nicht auf der Liste erlaubter Seiten. {HOW_TO_ALLOW}"));
        }
        let addrs = self.resolve(&host, port).await;
        if addrs.is_empty() {
            return Err(format!("{host}: Name nicht gefunden."));
        }
        if let Some((ip, why)) = addrs.iter().find_map(|a| blocked(a.ip()).map(|w| (a.ip(), w))) {
            return Err(format!("{host} zeigt auf {ip} ({why}); das ist immer gesperrt (Schutz vor DNS-Rebinding)."));
        }
        Ok(addrs)
    }

    async fn resolve(&self, host: &str, port: u16) -> Vec<SocketAddr> {
        #[cfg(test)]
        if let Some(t) = &self.test {
            return t.hosts.get(host).map(|ips| ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect()).unwrap_or_default();
        }
        match tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((host, port))).await {
            Ok(Ok(it)) => it.take(16).collect(),
            _ => Vec::new(),
        }
    }

    /// Connect to the first checked address that answers (never to a name: no second lookup).
    async fn dial(&self, addrs: &[SocketAddr]) -> std::result::Result<tokio::net::TcpStream, String> {
        #[cfg(test)]
        if let Some(t) = &self.test {
            return match t.dial {
                Some(to) => tokio::net::TcpStream::connect(to).await.map_err(|e| e.to_string()),
                None => Err("keine Verbindung".into()),
            };
        }
        let mut last = "keine Adresse".to_string();
        for a in addrs {
            match tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(a)).await {
                Ok(Ok(s)) => return Ok(s),
                Ok(Err(e)) => last = e.to_string(),
                Err(_) => last = "Zeitüberschreitung".into(),
            }
        }
        Err(last)
    }
}

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Refused,
    /// Allowed, but the site did not answer.
    Failed,
}

/// One connection from the workspace, for the action log and the tool result.
#[derive(Debug, Clone, PartialEq)]
pub struct Conn {
    pub host: String,
    pub port: u16,
    pub outcome: Outcome,
    /// The refusal or failure; empty when it went through.
    pub why: String,
    /// Bytes from the workspace to the site, and back.
    pub sent: u64,
    pub received: u64,
}

impl Conn {
    fn refused(host: &str, port: u16, why: String) -> Conn {
        Conn { host: host.chars().take(253).collect(), port, outcome: Outcome::Refused, why, sent: 0, received: 0 }
    }
}

const MAX_HEAD: usize = 16 * 1024;
const MAX_RECORD: usize = 16 * 1024 + 5;
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// One proxy connection from the workspace: the request, the checks, the tunnel. `None` when
/// the client sent nothing usable before closing.
pub async fn handle<S: AsyncRead + AsyncWrite + Unpin>(mut client: S, policy: &Policy) -> Option<Conn> {
    let (head, rest) = match tokio::time::timeout(READ_TIMEOUT, read_head(&mut client)).await {
        Ok(Ok(h)) => h,
        _ => return None,
    };
    let req = match parse_request(&head) {
        Ok(r) => r,
        Err(why) => {
            refuse(&mut client, &why).await;
            return Some(Conn::refused("?", 0, why));
        }
    };
    let addrs = match policy.admit(&req.host, req.port).await {
        Ok(a) => a,
        Err(why) => {
            refuse(&mut client, &why).await;
            return Some(Conn::refused(&req.host, req.port, why));
        }
    };
    let first = if req.connect {
        if req.port != 443 {
            let why = format!("{}: ein Tunnel (CONNECT) nur zu Port 443.", req.host);
            refuse(&mut client, &why).await;
            return Some(Conn::refused(&req.host, req.port, why));
        }
        if client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.is_err() {
            return None;
        }
        // The first TLS record must name the same server.
        let hello = match tokio::time::timeout(READ_TIMEOUT, read_record(&mut client, rest)).await {
            Ok(Ok(h)) => h,
            Ok(Err(why)) => return Some(Conn::refused(&req.host, req.port, why)),
            Err(_) => return Some(Conn::refused(&req.host, req.port, "Kein TLS-Anfang im Tunnel.".into())),
        };
        let wanted = req.host.trim_end_matches('.').to_ascii_lowercase();
        let named_by_ip = wanted.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>().is_ok();
        match server_name(&hello) {
            Some(n) if n == wanted => {}
            None if named_by_ip => {}
            Some(n) => return Some(Conn::refused(&req.host, req.port, format!("Der TLS-Servername {n} passt nicht zu {wanted}."))),
            None => return Some(Conn::refused(&req.host, req.port, "Der TLS-Anfang nennt keinen Servernamen (SNI).".into())),
        }
        hello
    } else {
        let mut v = req.head.into_bytes();
        v.extend_from_slice(&rest);
        v
    };
    let mut upstream = match policy.dial(&addrs).await {
        Ok(s) => s,
        Err(e) => {
            if !req.connect {
                refuse(&mut client, &format!("{} antwortet nicht: {e}", req.host)).await;
            }
            return Some(Conn {
                host: req.host.clone(),
                port: req.port,
                outcome: Outcome::Failed,
                why: format!("nicht erreichbar: {e}"),
                sent: 0,
                received: 0,
            });
        }
    };
    if upstream.write_all(&first).await.is_err() {
        return Some(Conn { host: req.host, port: req.port, outcome: Outcome::Failed, why: "Verbindung abgebrochen".into(), sent: 0, received: 0 });
    }
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = upstream.into_split();
    let (up, down) = tokio::join!(copy_counted(&mut cr, &mut uw), copy_counted(&mut ur, &mut cw));
    Some(Conn { host: req.host, port: req.port, outcome: Outcome::Allowed, why: String::new(), sent: up + first.len() as u64, received: down })
}

/// Copy until either side ends; the bytes that went through (also when it broke off).
async fn copy_counted<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(r: &mut R, w: &mut W) -> u64 {
    let mut buf = vec![0u8; 16 * 1024];
    let mut n = 0u64;
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(k) => {
                if w.write_all(buf.get(..k).unwrap_or(&[])).await.is_err() {
                    break;
                }
                n += k as u64;
            }
        }
    }
    let _ = w.shutdown().await;
    n
}

async fn refuse<S: AsyncWrite + Unpin>(client: &mut S, why: &str) {
    let body = format!("AIS.ChatCraft: {why}\n");
    let head = format!("HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain; charset=utf-8\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
    let _ = client.write_all(head.as_bytes()).await;
    let _ = client.write_all(body.as_bytes()).await;
    let _ = client.shutdown().await;
}

/// The request head (up to the empty line) and what came after it.
async fn read_head<S: AsyncRead + Unpin>(client: &mut S) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.get(end + 4..).unwrap_or(&[]).to_vec();
            buf.truncate(end + 4);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "head too long"));
        }
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
    }
}

/// One complete TLS record (the ClientHello), starting with `buf`.
async fn read_record<S: AsyncRead + Unpin>(client: &mut S, mut buf: Vec<u8>) -> std::result::Result<Vec<u8>, String> {
    let mut chunk = [0u8; 4096];
    loop {
        if let (Some(&t), Some(&hi), Some(&lo)) = (buf.first(), buf.get(3), buf.get(4)) {
            if t != 0x16 {
                return Err("Durch den Tunnel kam kein TLS.".into());
            }
            let need = 5 + usize::from(u16::from_be_bytes([hi, lo]));
            if need > MAX_RECORD {
                return Err("Der TLS-Anfang ist zu lang.".into());
            }
            if buf.len() >= need {
                return Ok(buf);
            }
        }
        let n = client.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Der Tunnel endete vor dem TLS-Anfang.".into());
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
        if buf.len() > MAX_RECORD + 4096 {
            return Err("Der TLS-Anfang ist zu lang.".into());
        }
    }
}

struct Request {
    connect: bool,
    host: String,
    port: u16,
    /// For HTTP: the head as it goes to the site (origin form, `Connection: close`).
    head: String,
}

fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// `host:port` or `[v6]:port`.
fn authority(a: &str, default_port: u16) -> std::result::Result<(String, u16), String> {
    let bad = || format!("„{}“ ist kein gültiges Ziel.", a.chars().take(80).collect::<String>());
    if a.contains('@') || a.is_empty() {
        return Err(bad());
    }
    if let Some(rest) = a.strip_prefix('[') {
        let (h, tail) = rest.split_once(']').ok_or_else(bad)?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().map_err(|_| bad())?,
            None if tail.is_empty() => default_port,
            None => return Err(bad()),
        };
        return Ok((format!("[{h}]"), port));
    }
    match a.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse().map_err(|_| bad())?)),
        None => Ok((a.to_string(), default_port)),
    }
}

fn parse_request(head: &str) -> std::result::Result<Request, String> {
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err("Keine gültige Anfrage an den Proxy.".into());
    };
    if !token(method) || !version.starts_with("HTTP/1.") {
        return Err("Keine gültige Anfrage an den Proxy.".into());
    }
    if method == "CONNECT" {
        let (host, port) = authority(target, 443)?;
        return Ok(Request { connect: true, host, port, head: String::new() });
    }
    let Some(rest) = target.strip_prefix("http://") else {
        return Err("Der Proxy nimmt nur http://-Adressen und Tunnel (CONNECT) für https an.".into());
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = rest.get(..end).unwrap_or("");
    let path = match rest.get(end..).unwrap_or("") {
        "" => "/".to_string(),
        p if p.starts_with('/') => p.to_string(),
        p => format!("/{p}"),
    };
    let (host, port) = authority(auth, 80)?;
    let mut out = format!("{method} {path} {version}\r\n");
    for line in lines.filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if name == "host" {
            let given = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
            let (h, _) = authority(given, 80)?;
            if !h.eq_ignore_ascii_case(&host) {
                return Err(format!("Host-Kopfzeile {h} passt nicht zu {host}."));
            }
            continue;
        }
        if matches!(name.as_str(), "connection" | "keep-alive" | "proxy-connection" | "proxy-authorization" | "upgrade") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("Host: {auth}\r\nConnection: close\r\n\r\n"));
    Ok(Request { connect: false, host, port, head: out })
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }
    fn u16(&mut self) -> Option<usize> {
        let b = self.take(2)?;
        Some(usize::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
    }
}

/// The server name (SNI) of a TLS ClientHello record, lower case.
pub(crate) fn server_name(record: &[u8]) -> Option<String> {
    let mut r = Reader(record.get(5..)?);
    if r.u8()? != 1 {
        return None;
    }
    r.take(3 + 2 + 32)?;
    let n = usize::from(r.u8()?);
    r.take(n)?;
    let n = r.u16()?;
    r.take(n)?;
    let n = usize::from(r.u8()?);
    r.take(n)?;
    let n = r.u16()?;
    let mut ext = Reader(r.take(n)?);
    while let Some(kind) = ext.u16() {
        let n = ext.u16()?;
        let body = ext.take(n)?;
        if kind != 0 {
            continue;
        }
        let mut b = Reader(body);
        let n = b.u16()?;
        let mut list = Reader(b.take(n)?);
        while let Some(t) = list.u8() {
            let n = list.u16()?;
            let name = list.take(n)?;
            if t == 0 {
                return std::str::from_utf8(name).ok().map(|s| s.trim_end_matches('.').to_ascii_lowercase());
            }
        }
    }
    None
}

// ---- the relay ------------------------------------------------------------------------------

/// Frames between the relay and the engine: kind (1 byte), connection (4), length (4), bytes.
pub(crate) const T_READY: u8 = 0;
pub(crate) const T_OPEN: u8 = 1;
pub(crate) const T_DATA: u8 = 2;
pub(crate) const T_CLOSE: u8 = 3;
const MAX_FRAME: usize = 64 * 1024;
const MAX_CONNS: usize = 64;
/// The relay's port on the run's internal network.
pub(crate) const PORT: u16 = 3128;

/// The relay, in a locked-down container on the run's internal network (Python from the
/// workspace image): accepts TCP on port 3128 and passes each connection as frames over
/// stdin/stdout. It decides nothing and reaches nothing; the engine does both. It ends with its
/// stdin.
pub(crate) const RELAY_SCRIPT: &str = r#"import os, socket, struct, sys, threading
o = sys.stdout.buffer
i = sys.stdin.buffer
lk = threading.Lock()
cs = {}
def snd(t, n, d=b""):
    with lk:
        o.write(struct.pack(">BII", t, n, len(d)) + d)
        o.flush()
def done(n, side):
    with lk:
        e = cs.get(n)
        if e is None:
            return
        e[1].add(side)
        if len(e[1]) < 2:
            return
        del cs[n]
    try:
        e[0].close()
    except OSError:
        pass
def pump(n, s):
    try:
        while True:
            d = s.recv(16384)
            if not d:
                break
            snd(2, n, d)
    except OSError:
        pass
    snd(3, n)
    done(n, "r")
def serve(l):
    n = 0
    while True:
        s, _ = l.accept()
        n += 1
        with lk:
            full = len(cs) >= 64
            if not full:
                cs[n] = (s, set())
        if full:
            s.close()
            continue
        snd(1, n)
        threading.Thread(target=pump, args=(n, s), daemon=True).start()
def rd(k):
    b = b""
    while len(b) < k:
        c = i.read(k - len(b))
        if not c:
            os._exit(0)
        b += c
    return b
l = socket.socket()
l.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
l.bind(("0.0.0.0", 3128))
l.listen(64)
threading.Thread(target=serve, args=(l,), daemon=True).start()
snd(0, 0)
while True:
    t, n, k = struct.unpack(">BII", rd(9))
    d = rd(k) if k else b""
    with lk:
        e = cs.get(n)
    if e is None:
        continue
    try:
        if t == 2:
            e[0].sendall(d)
        elif t == 3:
            e[0].shutdown(socket.SHUT_WR)
            done(n, "w")
    except OSError:
        done(n, "w")
"#;

/// In the private computer as root, before a run with network: the rules that keep every
/// internal bridge of the workspace (`ai-…`) from reaching the computer itself or anything
/// beyond it (UDP, ICMP and TCP alike; traffic between the relay and the workload on the same
/// bridge stays), then the run's internal network. `$1` network, `$2` bridge name.
pub(crate) const NET_UP_SCRIPT: &str = r#"set -e
command -v iptables >/dev/null 2>&1 || { echo "iptables fehlt im eigenen Rechner; ohne Sperrregeln gibt es kein Netz." >&2; exit 4; }
iptables -C INPUT -i ai-+ -j DROP 2>/dev/null || iptables -I INPUT -i ai-+ -j DROP
iptables -C FORWARD -i ai-+ ! -o ai-+ -j DROP 2>/dev/null || iptables -I FORWARD -i ai-+ ! -o ai-+ -j DROP
iptables -C FORWARD -o ai-+ ! -i ai-+ -j DROP 2>/dev/null || iptables -I FORWARD -o ai-+ ! -i ai-+ -j DROP
if command -v ip6tables >/dev/null 2>&1; then
  ip6tables -C INPUT -i ai-+ -j DROP 2>/dev/null || ip6tables -I INPUT -i ai-+ -j DROP 2>/dev/null || true
  ip6tables -C FORWARD -i ai-+ ! -o ai-+ -j DROP 2>/dev/null || ip6tables -I FORWARD -i ai-+ ! -o ai-+ -j DROP 2>/dev/null || true
  ip6tables -C FORWARD -o ai-+ ! -i ai-+ -j DROP 2>/dev/null || ip6tables -I FORWARD -o ai-+ ! -i ai-+ -j DROP 2>/dev/null || true
fi
docker network create --internal --label aiscraft.net=1 -o com.docker.network.bridge.name="$2" "$1" >/dev/null"#;
/// After the run: the relay and the run's network go. `$1` network, `$2` relay.
pub(crate) const NET_DOWN_SCRIPT: &str = r#"docker rm -f "$2" >/dev/null 2>&1; docker network rm "$1" >/dev/null 2>&1; true"#;

pub(crate) fn frame(kind: u8, id: u32, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(9 + data.len());
    v.push(kind);
    v.extend_from_slice(&id.to_be_bytes());
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(data);
    v
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<(u8, u32, Vec<u8>)> {
    let mut h = [0u8; 9];
    r.read_exact(&mut h).await?;
    let [kind, i0, i1, i2, i3, l0, l1, l2, l3] = h;
    let len = u32::from_be_bytes([l0, l1, l2, l3]) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut data = vec![0u8; len];
    r.read_exact(&mut data).await?;
    Ok((kind, u32::from_be_bytes([i0, i1, i2, i3]), data))
}

/// Called once per connection when it ends (refused at once).
pub type OnConn = Arc<dyn Fn(Conn) + Send + Sync>;

/// Serve the relay's connections until it ends: each one through [`handle`].
pub async fn serve<R, W>(mut reader: R, writer: W, policy: Policy, on_conn: OnConn)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let mut w = writer;
        while let Some(f) = rx.recv().await {
            if w.write_all(&f).await.is_err() || w.flush().await.is_err() {
                break;
            }
        }
    });
    let mut conns: HashMap<u32, mpsc::Sender<Vec<u8>>> = HashMap::new();
    while let Ok((kind, id, data)) = read_frame(&mut reader).await {
        while tasks.try_join_next().is_some() {}
        match kind {
            T_OPEN => {
                if conns.len() >= MAX_CONNS || conns.contains_key(&id) {
                    let _ = tx.send(frame(T_CLOSE, id, &[])).await;
                    continue;
                }
                let (engine_side, relay_side) = tokio::io::duplex(256 * 1024);
                let (mut from_client, mut to_client) = tokio::io::split(relay_side);
                let (ctx, mut crx) = mpsc::channel::<Vec<u8>>(64);
                conns.insert(id, ctx);
                tasks.spawn(async move {
                    while let Some(d) = crx.recv().await {
                        if to_client.write_all(&d).await.is_err() {
                            break;
                        }
                    }
                    let _ = to_client.shutdown().await;
                });
                let out = tx.clone();
                tasks.spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match from_client.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if out.send(frame(T_DATA, id, buf.get(..n).unwrap_or(&[]))).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    let _ = out.send(frame(T_CLOSE, id, &[])).await;
                });
                let (p, cb) = (policy.clone(), on_conn.clone());
                tasks.spawn(async move {
                    if let Some(c) = handle(engine_side, &p).await {
                        cb(c);
                    }
                });
            }
            T_DATA => {
                if let Some(s) = conns.get(&id)
                    && s.send(data).await.is_err()
                {
                    conns.remove(&id);
                }
            }
            // The client is done sending: dropping its sender ends the stream towards `handle`.
            T_CLOSE => {
                conns.remove(&id);
            }
            _ => {}
        }
    }
    // The relay ended: every client stream ends with it; connections log what went through.
    drop(conns);
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(3), async { while tasks.join_next().await.is_some() {} }).await;
}

/// A relay with its network, for one run: the workload joins `network` and finds the proxy at
/// `http://<relay>:3128`. [`Online::close`] removes both and gives the connections.
pub struct Online {
    pub network: String,
    pub relay: String,
    task: tokio::task::JoinHandle<()>,
    conns: Arc<Mutex<Vec<Conn>>>,
}

/// Start the relay on a new internal network and serve it with `policy`. `on_conn` sees every
/// connection as it ends (for the action log).
pub(crate) async fn open(m: &dyn SandboxRuntime, image: &str, policy: Policy, on_conn: OnConn) -> Result<Online> {
    let id: String = crate::new_id().replace('-', "").chars().take(12).collect();
    let network = format!("aiscraft-int-{id}");
    let bridge = format!("ai-{}", id.chars().take(10).collect::<String>());
    let relay = format!("aiscraft-relay-{id}");
    let up = m.exec(k::machine_script(NET_UP_SCRIPT, &[&network, &bridge]), Vec::new(), Duration::from_secs(60), 16 * 1024).await?;
    if !up.ok() {
        return Err(Error::unsupported(format!("Das Netz des Arbeitsbereichs lässt sich nicht einrichten: {}", up.why())));
    }
    let started = async {
        let mut pipe = m.stream(k::docker_relay(&network, &relay, image)).await?;
        match tokio::time::timeout(Duration::from_secs(60), read_frame(&mut pipe.reader)).await {
            Ok(Ok((T_READY, _, _))) => Ok(pipe),
            _ => Err(Error::unsupported("Der Netz-Wächter im eigenen Rechner startet nicht. „Anhalten“ in den Einstellungen und noch einmal versuchen.")),
        }
    }
    .await;
    let pipe = match started {
        Ok(p) => p,
        Err(e) => {
            let _ = m.exec(k::machine_script(NET_DOWN_SCRIPT, &[&network, &relay]), Vec::new(), Duration::from_secs(60), 4096).await;
            return Err(e);
        }
    };
    let conns = Arc::new(Mutex::new(Vec::new()));
    let seen = conns.clone();
    let cb: OnConn = Arc::new(move |c: Conn| {
        on_conn(c.clone());
        let mut v = seen.lock().unwrap_or_else(PoisonError::into_inner);
        if v.len() < 1000 {
            v.push(c);
        }
    });
    let task = tokio::spawn(async move {
        let super::runtime::Pipe { reader, writer, guard } = pipe;
        serve(reader, writer, policy, cb).await;
        drop(guard);
    });
    Ok(Online { network, relay, task, conns })
}

impl Online {
    /// Remove the relay and the network; the connections of the run.
    pub(crate) async fn close(self, m: &dyn SandboxRuntime) -> Vec<Conn> {
        let Online { network, relay, mut task, conns } = self;
        let _ = m.exec(k::machine_script(NET_DOWN_SCRIPT, &[&network, &relay]), Vec::new(), Duration::from_secs(60), 4096).await;
        // Connections still open end with the relay; give their log lines a moment.
        if tokio::time::timeout(Duration::from_secs(5), &mut task).await.is_err() {
            task.abort();
        }
        conns.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// What the model reads about the network after a command: allowed sites with bytes, refusals
/// with how the person allows a site.
pub(crate) fn report(conns: &[Conn]) -> String {
    if conns.is_empty() {
        return String::new();
    }
    let mut ok: Vec<(String, usize, u64, u64)> = Vec::new();
    let mut no: Vec<String> = Vec::new();
    for c in conns {
        match c.outcome {
            Outcome::Allowed => match ok.iter_mut().find(|(h, ..)| *h == c.host) {
                Some(e) => {
                    e.1 += 1;
                    e.2 += c.sent;
                    e.3 += c.received;
                }
                None => ok.push((c.host.clone(), 1, c.sent, c.received)),
            },
            _ => {
                let line = format!("{}: {}", if c.outcome == Outcome::Refused { "abgelehnt" } else { "nicht erreicht" }, c.why);
                if !no.contains(&line) && no.len() < 20 {
                    no.push(line);
                }
            }
        }
    }
    let mut s = String::from("--- Netz ---\n");
    for (h, n, up, down) in ok.iter().take(20) {
        s.push_str(&format!("erlaubt: {h} ({n} Verbindung{}, {up} Bytes hinaus, {down} Bytes herein)\n", if *n == 1 { "" } else { "en" }));
    }
    for l in no {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// A ClientHello naming `sni`, as a TLS client sends it first through a tunnel.
#[cfg(test)]
pub(crate) fn client_hello(sni: &str) -> Vec<u8> {
    let name = sni.as_bytes();
    let mut ext = Vec::new();
    ext.extend_from_slice(&0u16.to_be_bytes());
    let list_len = 3 + name.len();
    ext.extend_from_slice(&((list_len + 2) as u16).to_be_bytes());
    ext.extend_from_slice(&(list_len as u16).to_be_bytes());
    ext.push(0);
    ext.extend_from_slice(&(name.len() as u16).to_be_bytes());
    ext.extend_from_slice(name);
    let mut body = vec![3, 3];
    body.extend_from_slice(&[7u8; 32]);
    body.push(0);
    body.extend_from_slice(&[0, 2, 0x13, 0x01]);
    body.extend_from_slice(&[1, 0]);
    body.extend_from_slice(&(ext.len() as u16).to_be_bytes());
    body.extend_from_slice(&ext);
    let mut hs = vec![1];
    hs.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    hs.extend_from_slice(&body);
    let mut rec = vec![0x16, 3, 1];
    rec.extend_from_slice(&(hs.len() as u16).to_be_bytes());
    rec.extend_from_slice(&hs);
    rec
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn private_local_and_metadata_addresses_are_blocked() {
        for ip in [
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.10",
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fd00::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:192.168.1.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:c0a8:0101::1",
        ] {
            assert!(blocked(ip.parse().unwrap()).is_some(), "{ip}");
        }
        for ip in ["8.8.8.8", "151.101.0.223", "172.32.0.1", "100.128.0.1", "2a04:4e42::223", "::ffff:8.8.8.8"] {
            assert!(blocked(ip.parse().unwrap()).is_none(), "{ip}");
        }
    }

    #[test]
    fn domains_and_wildcards() {
        assert_eq!(normalize_domain(" PyPI.org. ").unwrap(), "pypi.org");
        assert_eq!(normalize_domain("https://github.com/a/b").unwrap(), "github.com");
        assert_eq!(normalize_domain("*.Example.org").unwrap(), "*.example.org");
        for bad in ["", "*", "*.org", "localhost", "192.168.1.1", "pypi.org:443", "a b.org", "-x.org", "müller.de", "user@x.org", "*.*.x.org"] {
            assert!(normalize_domain(bad).is_err(), "{bad}");
        }
        let list = vec!["pypi.org".to_string(), "*.example.org".to_string()];
        assert!(on_list(&list, "pypi.org") && on_list(&list, "PYPI.ORG."));
        assert!(!on_list(&list, "evilpypi.org") && !on_list(&list, "pypi.org.evil.net"));
        assert!(on_list(&list, "a.example.org") && on_list(&list, "b.a.example.org"));
        assert!(!on_list(&list, "example.org") && !on_list(&list, "badexample.org"));
    }

    #[test]
    fn requests_and_server_names() {
        let r = parse_request("CONNECT pypi.org:443 HTTP/1.1\r\nHost: pypi.org:443\r\n\r\n").unwrap();
        assert!(r.connect && r.host == "pypi.org" && r.port == 443);
        let r = parse_request("GET http://example.org/a?b HTTP/1.1\r\nHost: example.org\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n").unwrap();
        assert!(!r.connect && r.port == 80);
        assert!(r.head.starts_with("GET /a?b HTTP/1.1\r\n") && r.head.contains("Accept: */*") && r.head.contains("Connection: close"));
        assert!(!r.head.to_lowercase().contains("proxy-connection"));
        assert!(parse_request("GET http://example.org/ HTTP/1.1\r\nHost: evil.net\r\n\r\n").is_err(), "Host must match");
        assert!(parse_request("GET https://example.org/ HTTP/1.1\r\n\r\n").is_err());
        assert!(parse_request("GET http://user@example.org/ HTTP/1.1\r\n\r\n").is_err());
        assert!(parse_request("CONNECT [::1]:443 HTTP/1.1\r\n\r\n").unwrap().host == "[::1]");
        assert_eq!(server_name(&client_hello("PyPI.org")).as_deref(), Some("pypi.org"));
        assert_eq!(server_name(b"\x16\x03\x01\x00\x02\x01\x00"), None);
        assert_eq!(server_name(&[]), None);
    }

    #[test]
    fn old_switch_and_modes_parse() {
        assert_eq!(serde_json::from_value::<NetMode>(serde_json::json!(true)).unwrap(), NetMode::List);
        assert_eq!(serde_json::from_value::<NetMode>(serde_json::json!(false)).unwrap(), NetMode::Off);
        assert_eq!(serde_json::from_value::<NetMode>(serde_json::json!("internet")).unwrap(), NetMode::Internet);
        assert_eq!(serde_json::from_value::<NetMode>(serde_json::json!("quatsch")).unwrap(), NetMode::Off);
        assert_eq!(serde_json::to_value(NetMode::List).unwrap(), "list");
    }
}
