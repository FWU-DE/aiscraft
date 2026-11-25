//! Network grants of connector views (MCP Apps, #74): which domains a view may reach. Shared by
//! the engine, which checks a view before it opens, and `aiscraft-preview`, which checks again.

/// Last labels that never name a public host: local networks, reserved names. A view on the
/// person's device must not reach the device itself or the school's network through them.
const PRIVATE_TLDS: &[&str] = &["localhost", "local", "internal", "lan", "home", "corp", "intranet", "private", "arpa", "test", "invalid", "localdomain"];

/// Public suffixes under which anyone can have a name. `*.github.io` would let a view reach every
/// GitHub Pages site, so a wildcard right over one of these is refused. A small list of the common
/// ones; second levels like `co.uk` are caught by [`GENERIC_SECOND_LEVELS`] as well.
const PUBLIC_SUFFIXES: &[&str] = &[
    "github.io",
    "githubusercontent.com",
    "gitlab.io",
    "pages.dev",
    "workers.dev",
    "vercel.app",
    "netlify.app",
    "herokuapp.com",
    "web.app",
    "firebaseapp.com",
    "appspot.com",
    "blogspot.com",
    "azurewebsites.net",
    "azurestaticapps.net",
    "cloudapp.net",
    "windows.net",
    "core.windows.net",
    "blob.core.windows.net",
    "cloudfront.net",
    "amazonaws.com",
    "s3.amazonaws.com",
    "onrender.com",
    "fly.dev",
    "glitch.me",
    "replit.app",
    "repl.co",
    "ngrok.io",
    "ngrok-free.app",
    "trycloudflare.com",
    "r2.dev",
    "surge.sh",
    "neocities.org",
    "wordpress.com",
    "wixsite.com",
    "myshopify.com",
    "duckdns.org",
    "dyndns.org",
    "no-ip.org",
    "eu.org",
];

/// Second levels that country registries hand out to everyone (`co.uk`, `com.au`, `ac.jp` …).
const GENERIC_SECOND_LEVELS: &[&str] =
    &["co", "com", "org", "net", "ac", "gov", "edu", "or", "ne", "go", "gv", "sch", "nic", "ltd", "plc", "me", "info", "biz"];

/// Whether `label` is a number as a URL parser reads it in a host (decimal, `0x` hex, octal),
/// i.e. the host would be an IPv4 address.
fn numeric_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    if let Some(hex) = l.strip_prefix("0x") {
        return hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    !l.is_empty() && l.chars().all(|c| c.is_ascii_digit())
}

/// Whether `host` (lower case, without a wildcard) is a public suffix in our list or a generic
/// second level of a country (`co.uk`).
fn public_suffix(host: &str) -> bool {
    if PUBLIC_SUFFIXES.contains(&host) {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    matches!(labels.as_slice(), [second, tld] if tld.len() == 2 && GENERIC_SECOND_LEVELS.contains(second))
}

/// A declared domain of a view: `https://host[:port]`, `https://*.host` (and `wss://` for
/// connections). Never `*`, `http:`, other schemes, paths, quotes or keywords; never an IP
/// address, `localhost`, a local name (`.local`, `.internal` …) or a wildcard over a public
/// suffix (`*.co.uk`, `*.github.io`).
pub fn view_domain_ok(d: &str, connect: bool) -> bool {
    if d.len() > 253 {
        return false;
    }
    let rest = match d.split_once("://") {
        Some(("https", r)) => r,
        Some(("wss", r)) if connect => r,
        _ => return false,
    };
    let (wildcard, host) = match rest.strip_prefix("*.") {
        Some(h) => (true, h),
        None => (false, rest),
    };
    let (host, port) = match host.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host, None),
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()));
    let labels: Vec<&str> = host.split('.').collect();
    let shape_ok = port_ok
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-'));
    if !shape_ok {
        return false;
    }
    let host = host.to_ascii_lowercase();
    let tld = labels.last().map(|l| l.to_ascii_lowercase()).unwrap_or_default();
    // IPv4 in any spelling (IPv6 needs brackets, which the label check refuses).
    if numeric_label(&tld) || labels.iter().all(|l| numeric_label(l)) {
        return false;
    }
    if PRIVATE_TLDS.contains(&tld.as_str()) {
        return false;
    }
    !(wildcard && public_suffix(&host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_domains() {
        for ok in ["https://api.example.org", "https://*.example.org", "https://cdn.example.org:8443", "https://user.github.io", "https://*.bbc.co.uk"] {
            assert!(view_domain_ok(ok, false), "{ok}");
        }
        assert!(view_domain_ok("wss://live.example.org", true) && !view_domain_ok("wss://live.example.org", false));
        for bad in [
            "*",
            "https://*",
            "http://example.org",
            "data:",
            "https://example.org/path",
            "'unsafe-eval'",
            "https://exa mple.org",
            "https://example.org;",
            "example.org",
            "https://localhost",
            "https://localhost:8080",
            "https://app.localhost",
            "https://*.localhost",
            "https://drucker.local",
            "https://intranet.schule.internal",
            "https://fritz.box.lan",
            "https://127.0.0.1",
            "https://127.0.0.1:8080",
            "https://10.0.0.1",
            "https://192.168.0.1",
            "https://0x7f.1",
            "https://0x7f000001.0x1",
            "https://1.0x7f",
            "https://[::1]",
            "https://[::1]:443",
            "https://*.co.uk",
            "https://*.com.au",
            "https://*.github.io",
            "https://*.GitHub.IO",
            "https://*.vercel.app",
            "https://*.s3.amazonaws.com",
            "https://*.com",
        ] {
            assert!(!view_domain_ok(bad, true), "{bad}");
        }
    }
}
