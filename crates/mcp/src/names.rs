/// Longest tool name the model APIs accept (OpenAI, Anthropic: `^[a-zA-Z0-9_-]{1,64}$`).
const MAX_LEN: usize = 64;

/// The name a model sees for `tool_name` of the connector `server_id`: `<server>__<tool>`,
/// restricted to `[a-zA-Z0-9_-]` and at most 64 characters.
///
/// When characters had to be replaced or the name was too long, an 8-digit hash of the
/// original pair is appended, so two different tools never end up with the same name.
pub fn llm_tool_name(server_id: &str, tool_name: &str) -> String {
    let raw = format!("{server_id}__{tool_name}");
    let mut lossy = false;
    let mut clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                lossy = true;
                '_'
            }
        })
        .collect();
    // `<server>__<tool>` splits back unambiguously only if the server id has no `__` of its own
    // and does not end in `_`.
    let ambiguous = server_id.is_empty() || tool_name.is_empty() || server_id.contains("__") || server_id.ends_with('_');
    if clean.len() <= MAX_LEN && !lossy && !ambiguous {
        return clean;
    }
    let hash = fnv1a(server_id.as_bytes(), tool_name.as_bytes());
    let suffix = format!("_{:08x}", hash as u32 ^ (hash >> 32) as u32);
    // All characters are ASCII now, so byte truncation is char-safe.
    clean.truncate(MAX_LEN - suffix.len());
    clean.push_str(&suffix);
    clean
}

fn fnv1a(a: &[u8], b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in a.iter().chain(&[0u8]).chain(b) {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(n: &str) -> bool {
        !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    #[test]
    fn plain_names_stay_readable() {
        assert_eq!(llm_tool_name("github", "search_issues"), "github__search_issues");
        assert_eq!(llm_tool_name("my-fs", "read-file"), "my-fs__read-file");
    }

    #[test]
    fn invalid_characters_are_replaced_and_stay_unique() {
        let a = llm_tool_name("fs", "read.file");
        let b = llm_tool_name("fs", "read_file");
        let c = llm_tool_name("fs", "read file");
        assert!(valid(&a) && valid(&b) && valid(&c));
        assert!(a.starts_with("fs__read_file_"));
        assert_ne!(a, b);
        assert_ne!(a, c);
        let umlaut = llm_tool_name("büro", "größe/ändern");
        assert!(valid(&umlaut), "{umlaut}");
    }

    #[test]
    fn long_names_are_truncated_with_a_hash() {
        let long = "x".repeat(100);
        let a = llm_tool_name("550e8400-e29b-41d4-a716-446655440000", &format!("{long}a"));
        let b = llm_tool_name("550e8400-e29b-41d4-a716-446655440000", &format!("{long}b"));
        assert!(valid(&a) && valid(&b));
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(a, llm_tool_name("550e8400-e29b-41d4-a716-446655440000", &format!("{long}a")), "stable");
    }

    #[test]
    fn empty_parts_still_give_a_valid_name() {
        assert!(valid(&llm_tool_name("", "")));
        assert!(valid(&llm_tool_name("s", "")));
        assert_ne!(llm_tool_name("", "ab"), llm_tool_name("a", "b"));
        assert_ne!(llm_tool_name("a_", "b"), llm_tool_name("a", "_b"));
        assert_ne!(llm_tool_name("a__x", "b"), llm_tool_name("a", "x__b"));
    }
}
