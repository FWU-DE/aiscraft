//! How AIS.ChatCraft makes itself known to model providers that list apps by name.
//!
//! OpenRouter and the Vercel AI Gateway attribute requests to an app by `HTTP-Referer` and
//! `X-Title`. Every other provider only gets the User-Agent the HTTP clients already send.

/// The address providers show for AIS.ChatCraft.
pub const ATTRIBUTION_REFERER: &str = "https://github.com/FWU-DE/aiscraft";
/// The name providers show for AIS.ChatCraft.
pub const ATTRIBUTION_TITLE: &str = "AIS.ChatCraft";

/// Hosts that read the attribution headers (and their subdomains).
pub const ATTRIBUTION_HOSTS: &[&str] = &["openrouter.ai", "ai-gateway.vercel.sh"];

const HEADERS: &[(&str, &str)] = &[("HTTP-Referer", ATTRIBUTION_REFERER), ("X-Title", ATTRIBUTION_TITLE)];

/// The host of an `http(s)://` address, lower case, without user info, port or trailing dot.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => host_port.split(':').next()?,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The extra headers a request to `url` carries so the provider can name AIS.ChatCraft; empty
/// for every provider that does not read them.
pub fn attribution_headers(url: &str) -> &'static [(&'static str, &'static str)] {
    let Some(host) = host_of(url) else { return &[] };
    let listed = ATTRIBUTION_HOSTS.iter().any(|h| host == *h || host.strip_suffix(h).is_some_and(|sub| sub.ends_with('.')));
    if listed { HEADERS } else { &[] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_and_vercel_get_the_headers_others_do_not() {
        for url in [
            "https://openrouter.ai/api/v1",
            "https://OpenRouter.ai./api/v1/chat/completions",
            "https://ai-gateway.vercel.sh/v1",
            "http://ai-gateway.vercel.sh:8080/v1/models",
            "https://eu.openrouter.ai/api/v1",
            "https://user:pw@openrouter.ai/api/v1",
        ] {
            assert_eq!(attribution_headers(url), HEADERS, "{url}");
        }
        for url in [
            "https://api.openai.com/v1",
            "https://api.anthropic.com",
            "http://127.0.0.1:11434/v1",
            "https://evil-openrouter.ai/api/v1",
            "https://openrouter.ai.example.org/v1",
            "https://example.org/openrouter.ai",
            "https://openrouter.ai@example.org/v1",
            "https://vercel.sh/v1",
            "openrouter.ai/api/v1",
            "",
        ] {
            assert!(attribution_headers(url).is_empty(), "{url}");
        }
    }

    #[test]
    fn host_of_handles_ports_ipv6_and_user_info() {
        assert_eq!(host_of("http://[::1]:8080/v1").as_deref(), Some("::1"));
        assert_eq!(host_of("https://a@b.example:1/x").as_deref(), Some("b.example"));
        assert_eq!(host_of("https://example.org?x=1").as_deref(), Some("example.org"));
        assert_eq!(host_of("nonsense"), None);
        assert_eq!(host_of("https:///path"), None);
    }

    #[test]
    fn header_values_name_the_app() {
        let h = attribution_headers("https://openrouter.ai/api/v1");
        assert!(h.contains(&("HTTP-Referer", "https://github.com/FWU-DE/aiscraft")));
        assert!(h.contains(&("X-Title", "AIS.ChatCraft")));
    }
}
