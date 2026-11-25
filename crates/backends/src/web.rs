//! Reading one web page for the model: the built-in page reader for backends that have no web
//! tools of their own. Read-only (GET), http and https only, no addresses in the local network,
//! capped in time and size. What comes back is untrusted data; the caller marks it as such.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use aiscraft_core::{Error, Result};

use crate::http::{USER_AGENT, net_err, read_capped};

/// Bytes read from a page at most.
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Characters of text handed to the model at most.
pub const MAX_TEXT_CHARS: usize = 60_000;
const MAX_REDIRECTS: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPage {
    /// Where the page was read, after redirects.
    pub url: String,
    pub title: String,
    pub text: String,
    /// The text was cut at [`MAX_TEXT_CHARS`] or the page at its byte cap.
    pub truncated: bool,
}

/// Read a page as text. `allow_private` lets tests reach their local server; everything else
/// refuses loopback, private and link-local addresses, also after redirects.
pub async fn fetch(url: &str, allow_private: bool) -> Result<WebPage> {
    let mut url = parse(url)?;
    for _ in 0..=MAX_REDIRECTS {
        let (client, host) = client_for(&url, allow_private, TIMEOUT).await?;
        let resp = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "text/html, text/plain;q=0.9, application/json;q=0.8, */*;q=0.1")
            .send()
            .await
            .map_err(net_err)?;
        let status = resp.status();
        if status.is_redirection() {
            url = redirect_target(&url, &resp)?;
            continue;
        }
        if !status.is_success() {
            return Err(Error::network(format!("{host} antwortet mit HTTP {}", status.as_u16())));
        }
        let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let textual = ctype.is_empty() || ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("xml");
        if !textual {
            return Err(Error::unsupported(format!("Kein lesbarer Text ({})", ctype.split(';').next().unwrap_or("").trim())));
        }
        let body = read_capped(resp, MAX_BYTES).await?;
        let cut_bytes = body.len() >= MAX_BYTES;
        let html = ctype.contains("html") || (ctype.is_empty() && body.trim_start().get(..1) == Some("<"));
        let (title, text) = if html { html_to_text(&body) } else { (String::new(), body) };
        let truncated = cut_bytes || text.chars().count() > MAX_TEXT_CHARS;
        let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
        let title = if title.trim().is_empty() { host.clone() } else { title.chars().take(200).collect() };
        return Ok(WebPage { url: url.to_string(), title, text, truncated });
    }
    Err(Error::network("Zu viele Weiterleitungen"))
}

/// A client that connects to exactly the addresses of `url`'s host checked here, so the name
/// cannot change in between; refuses addresses in the local network unless `allow_private`.
async fn client_for(url: &reqwest::Url, allow_private: bool, timeout: Duration) -> Result<(reqwest::Client, String)> {
    let host = url.host_str().unwrap_or("").to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    if host.is_empty() {
        return Err(Error::invalid("Die Adresse hat keinen Rechnernamen"));
    }
    let literal: Option<IpAddr> = host.trim_start_matches('[').trim_end_matches(']').parse().ok();
    let addrs: Vec<SocketAddr> = match literal {
        Some(ip) => vec![SocketAddr::new(ip, port)],
        None => tokio::time::timeout(timeout.min(Duration::from_secs(10)), tokio::net::lookup_host((host.as_str(), port)))
            .await
            .map_err(|_| Error::network(format!("{host}: Namensauflösung dauert zu lange")))?
            .map_err(|_| Error::network(format!("{host} gibt es nicht oder ist nicht erreichbar")))?
            .take(16)
            .collect(),
    };
    if addrs.is_empty() {
        return Err(Error::network(format!("{host} gibt es nicht oder ist nicht erreichbar")));
    }
    if !allow_private && addrs.iter().any(|a| !is_public(a.ip())) {
        return Err(Error::refused(format!("{host} liegt im lokalen Netz; solche Adressen liest AIS.ChatCraft nicht")));
    }
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .timeout(timeout);
    if literal.is_none() {
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    // Building a client reads the system's certificate store, which can take a while (macOS).
    // Off the async task, so the other links checked at the same time keep being answered and
    // their time limits are not spent waiting for this one.
    let client = tokio::task::spawn_blocking(move || builder.build())
        .await
        .map_err(|e| Error::network(format!("HTTP-Client: {e}")))?
        .map_err(|e| Error::network(format!("HTTP-Client: {e}")))?;
    Ok((client, host))
}

/// Where a redirect points, checked like the first address.
fn redirect_target(url: &reqwest::Url, resp: &reqwest::Response) -> Result<reqwest::Url> {
    let next = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|l| url.join(l).ok())
        .ok_or_else(|| Error::protocol("Weiterleitung ohne gültiges Ziel"))?;
    parse(next.as_str())
}

/// One hit of a web search.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search with a SearXNG server the person set up (`<base>/search?q=…&format=json`), for the
/// research (#37). The address is the person's own choice, like a local model server, so it may
/// be in the local network. Results are capped and only http(s) links kept.
pub async fn search_searx(base: &str, query: &str, max: usize) -> Result<Vec<SearchHit>> {
    let base = base.trim().trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(Error::invalid(format!("Suchadresse muss mit https:// beginnen: {base}")));
    }
    let mut url = reqwest::Url::parse(&format!("{base}/search")).map_err(|_| Error::invalid(format!("Keine gültige Suchadresse: {base}")))?;
    url.query_pairs_mut().append_pair("q", query).append_pair("format", "json").append_pair("language", "de");
    let client = crate::http::client(true)?;
    let resp = client.get(url).header(reqwest::header::ACCEPT, "application/json").timeout(TIMEOUT).send().await.map_err(crate::http::net_err)?;
    if !resp.status().is_success() {
        return Err(crate::http::status_error(resp, "Websuche").await);
    }
    let v = crate::http::json(resp, "Websuche").await?;
    let mut out = Vec::new();
    for r in v.get("results").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let s = |k: &str, cap: usize| r.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().chars().take(cap).collect::<String>();
        let link = s("url", 2000);
        if !(link.starts_with("https://") || link.starts_with("http://")) || out.iter().any(|h: &SearchHit| h.url == link) {
            continue;
        }
        out.push(SearchHit { title: s("title", 200), url: link, snippet: s("content", 600) });
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

/// What a link check found out about one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// The page answers (also behind a login or a bot check: it exists).
    Alive,
    /// Gone or unreachable, with the reason people read.
    Dead(String),
    /// Not asked: an address in the local network or not http(s).
    Skipped(String),
}

/// Check whether a link leads somewhere: HEAD first, GET when the server does not take HEAD;
/// redirects followed (at most [`MAX_REDIRECTS`], each target checked again), the body never read.
/// Same rules as [`fetch`]: http and https only, no addresses in the local network unless
/// `allow_private` (tests).
pub async fn check(url: &str, allow_private: bool, timeout: Duration) -> LinkState {
    let mut url = match parse(url) {
        Ok(u) => u,
        Err(e) => return LinkState::Skipped(e.message),
    };
    for _ in 0..=MAX_REDIRECTS {
        let client = match client_for(&url, allow_private, timeout).await {
            Ok((c, _)) => c,
            Err(e) if matches!(e.kind, aiscraft_core::ErrorKind::Refused | aiscraft_core::ErrorKind::Invalid | aiscraft_core::ErrorKind::Busy) => {
                return LinkState::Skipped(e.message);
            }
            Err(e) => return LinkState::Dead(e.message),
        };
        let mut resp = client.head(url.clone()).send().await;
        // Some servers refuse or botch HEAD; ask once more the normal way (only the head is read).
        let retry = match &resp {
            Ok(r) => matches!(r.status().as_u16(), 400 | 403 | 404 | 405 | 406 | 429 | 500 | 501 | 503),
            Err(e) => !e.is_timeout(),
        };
        if retry {
            resp = client.get(url.clone()).send().await;
        }
        let resp = match resp {
            Ok(r) => r,
            Err(e) => return LinkState::Dead(net_err(e).message),
        };
        let status = resp.status();
        if status.is_redirection() {
            match redirect_target(&url, &resp) {
                Ok(next) => {
                    url = next;
                    continue;
                }
                Err(e) if e.kind == aiscraft_core::ErrorKind::Invalid => return LinkState::Skipped(e.message),
                Err(e) => return LinkState::Dead(e.message),
            }
        }
        return match status.as_u16() {
            200..=299 => LinkState::Alive,
            // Behind a login, a bot check or a rate limit: the page is there.
            401 | 403 | 429 => LinkState::Alive,
            404 | 410 => LinkState::Dead(format!("Seite nicht gefunden (HTTP {})", status.as_u16())),
            code => LinkState::Dead(format!("HTTP {code}")),
        };
    }
    LinkState::Dead("Zu viele Weiterleitungen".into())
}

/// The http(s) addresses in a text (Markdown links, `<…>` and bare), in order, without
/// duplicates, at most `max`.
pub fn urls_in(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while out.len() < max {
        let at = match (rest.find("https://"), rest.find("http://")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => break,
        };
        let Some(tail) = rest.get(at..) else { break };
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '|' | '[' | ']' | '{' | '}')).unwrap_or(tail.len());
        let mut url = tail.get(..end).unwrap_or("");
        // Trailing punctuation and unbalanced closing brackets belong to the sentence.
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '~']);
            let trimmed = if trimmed.ends_with(')') && trimmed.matches('(').count() < trimmed.matches(')').count() {
                trimmed.get(..trimmed.len().saturating_sub(1)).unwrap_or("")
            } else {
                trimmed
            };
            if trimmed.len() == url.len() {
                break;
            }
            url = trimmed;
        }
        let scheme_only = url == "https://" || url == "http://";
        if !scheme_only && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
        rest = tail.get(end.max(1)..).unwrap_or("");
    }
    out
}

fn parse(raw: &str) -> Result<reqwest::Url> {
    let raw = raw.trim();
    let url = reqwest::Url::parse(raw).map_err(|_| Error::invalid(format!("„{}“ ist keine gültige Adresse", raw.chars().take(200).collect::<String>())))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(Error::invalid("Nur Adressen mit https:// oder http:// werden gelesen"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::invalid("Adressen mit Zugangsdaten werden nicht gelesen"));
    }
    Ok(url)
}

/// Addresses on the public internet; everything else (loopback, private, link-local, shared,
/// documentation, multicast, …) is refused.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            let first = s.first().copied().unwrap_or(0);
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first == 0x2001 && s.get(1) == Some(&0x0db8))
                // IPv4-compatible and NAT64 forms carry an IPv4 address that is not checked here.
                || (first == 0 && s.get(1..6).is_some_and(|x| x.iter().all(|y| *y == 0)))
                || (first == 0x64 && s.get(1) == Some(&0xff9b)))
        }
    }
}

/// Visible text of an HTML page and its title. Scripts, styles and comments are dropped; block
/// elements become line breaks.
pub fn html_to_text(html: &str) -> (String, String) {
    const SKIP: [&str; 6] = ["script", "style", "noscript", "svg", "template", "head"];
    const BLOCK: [&str; 22] = [
        "p",
        "div",
        "br",
        "li",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "tr",
        "section",
        "article",
        "header",
        "footer",
        "ul",
        "ol",
        "table",
        "pre",
        "blockquote",
        "main",
        "nav",
    ];
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut title = String::new();
    let mut i = 0;
    while i < html.len() {
        let rest = lower.get(i..).unwrap_or("");
        if rest.starts_with("<!--") {
            i = rest.find("-->").map(|e| i + e + 3).unwrap_or(html.len());
            continue;
        }
        if rest.starts_with('<') {
            let end = rest.find('>').map(|e| i + e + 1).unwrap_or(html.len());
            let inner = lower.get(i + 1..end.saturating_sub(1).max(i + 1)).unwrap_or("");
            let closing = inner.starts_with('/');
            let name: String = inner.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !closing && name == "title" {
                let close = lower.get(end..).and_then(|r| r.find("</title")).map(|e| end + e).unwrap_or(html.len());
                if title.is_empty() {
                    title = decode_entities(html.get(end..close).unwrap_or("")).split_whitespace().collect::<Vec<_>>().join(" ");
                }
                i = close;
                continue;
            }
            // <head> is skipped as a whole except for its title, which comes first in practice.
            if !closing && SKIP.contains(&name.as_str()) && !(name == "head" && lower.get(end..).is_some_and(|r| r.contains("<title"))) {
                let close = lower.get(end..).and_then(|r| r.find(&format!("</{name}"))).map(|e| end + e).unwrap_or(html.len());
                i = lower.get(close..).and_then(|r| r.find('>')).map(|e| close + e + 1).unwrap_or(html.len());
                continue;
            }
            if BLOCK.contains(&name.as_str()) {
                out.push('\n');
            } else if name == "td" || name == "th" {
                out.push(' ');
            }
            i = end;
            continue;
        }
        let next = rest.find('<').map(|e| i + e).unwrap_or(html.len());
        out.push_str(&decode_entities(html.get(i..next).unwrap_or("")));
        i = next;
    }
    let mut text = String::new();
    let mut blank = 0;
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank == 1 && !text.is_empty() {
                text.push('\n');
            }
            continue;
        }
        blank = 0;
        text.push_str(&line);
        text.push('\n');
    }
    (title, text.trim().to_string())
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or(""));
        let after = rest.get(p + 1..).unwrap_or("");
        let semi = after.find(';').filter(|n| *n <= 10);
        let decoded = semi.and_then(|n| {
            let name = after.get(..n)?;
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" | "#39" => '\'',
                "nbsp" => ' ',
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, n))
        });
        match decoded {
            Some((c, n)) => {
                out.push(c);
                rest = after.get(n + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_text_without_scripts() {
        let (title, text) = html_to_text(
            "<html><head><title> Die  Seite </title><style>p{}</style></head><body><!-- x --><script>alert(1)</script><h1>Kopf</h1><p>Erster &amp; zweiter&nbsp;Satz &#x263A;</p><ul><li>a</li><li>b</li></ul></body></html>",
        );
        assert_eq!(title, "Die Seite");
        assert!(text.contains("Kopf") && text.contains("Erster & zweiter Satz ☺"));
        assert!(!text.contains("alert") && !text.contains("p{}"));
        assert!(text.lines().count() >= 4, "block elements break lines: {text:?}");
    }

    #[test]
    fn hostile_html_does_not_panic() {
        for s in ["<", "<<<>", "&", "&#xFFFFFFFF;", "&#;", "<script>", "<!--", "<title>", "ä<ü>ö&auml", "<HEAD><TITLE>x"] {
            let _ = html_to_text(s);
        }
    }

    #[test]
    fn local_addresses_are_not_public() {
        for ip in
            ["127.0.0.1", "10.1.2.3", "192.168.0.1", "172.16.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"]
        {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn urls_in_text() {
        let t = "Siehe [Plan](https://schule.example/plan). Auch https://a.example/x_(y), <http://b.example/?q=1&r=2> und https://schule.example/plan!\n\
                 `https://` allein, ftp://c.example, (https://d.example/pfad) und https://e.example/ä?x=1.";
        assert_eq!(
            urls_in(t, 20),
            vec!["https://schule.example/plan", "https://a.example/x_(y)", "http://b.example/?q=1&r=2", "https://d.example/pfad", "https://e.example/ä?x=1"]
        );
        assert_eq!(urls_in(t, 2).len(), 2);
        for s in ["", "https://", "http://)", "ähttps://ö", "https://)))...", "[x](https://"] {
            let _ = urls_in(s, 20);
        }
    }

    #[tokio::test]
    async fn link_check_never_asks_local_addresses() {
        for u in [
            "http://127.0.0.1:9/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest",
            "http://[::1]/",
            "http://localhost:9/",
            "ftp://example.org/",
        ] {
            let state = check(u, false, Duration::from_millis(500)).await;
            assert!(matches!(state, LinkState::Skipped(_)), "{u}: {state:?}");
        }
    }

    #[tokio::test]
    async fn refuses_local_and_odd_addresses() {
        for u in ["http://127.0.0.1:9/", "http://[::1]/", "file:///etc/passwd", "ftp://example.org/", "http://user:pw@example.org/", "kein Link"] {
            assert!(fetch(u, false).await.is_err(), "{u}");
        }
    }
}
