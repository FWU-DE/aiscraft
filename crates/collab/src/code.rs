//! Join codes: ten symbols of Crockford's base 32 (50 bits), shown as `XXXXX-XXXXX`. Typing is
//! forgiving: case, spaces and dashes do not matter, `O` reads as `0`, `I` and `L` as `1`.

/// The symbols, without I, L, O and U.
pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Symbols in a code.
pub const LEN: usize = 10;

/// A code from random bytes (the server takes them from the operating system).
pub fn from_random(bytes: [u8; 8]) -> String {
    let mut n = u64::from_le_bytes(bytes);
    let mut out = String::with_capacity(LEN);
    for _ in 0..LEN {
        let i = (n & 31) as usize;
        n >>= 5;
        out.push(ALPHABET.get(i).map(|&b| b as char).unwrap_or('0'));
    }
    out
}

/// The code's ten symbols, or `None` when `input` is not a code.
pub fn normalize(input: &str) -> Option<String> {
    let mut out = String::with_capacity(LEN);
    for c in input.chars() {
        if c == '-' || c == ' ' || c == '\u{a0}' {
            continue;
        }
        let c = match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
        if out.len() > LEN {
            return None;
        }
    }
    (out.len() == LEN).then_some(out)
}

/// `XXXXX-XXXXX` for people.
pub fn display(code: &str) -> String {
    match (code.get(..5), code.get(5..)) {
        (Some(a), Some(b)) if !b.is_empty() => format!("{a}-{b}"),
        _ => code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_forgive_typos() {
        let c = from_random([7, 1, 2, 3, 4, 5, 6, 255]);
        assert_eq!(c.len(), LEN);
        let shown = display(&c);
        assert_eq!(shown.len(), LEN + 1);
        assert_eq!(normalize(&shown).as_deref(), Some(c.as_str()));
        assert_eq!(normalize(&shown.to_lowercase()).as_deref(), Some(c.as_str()));
        assert_eq!(normalize("oooo0-iiiil").as_deref(), Some("0000011111"));
        assert_eq!(normalize("ABCDE-FGHJ"), None, "too short");
        assert_eq!(normalize("ABCDE-FGHJKM"), None, "too long");
        assert_eq!(normalize("ABCDE-FGHJU"), None, "U is not a symbol");
        assert_eq!(normalize("ÄBCDE-FGHJK"), None);
        assert_eq!(display("abc"), "abc");
    }
}
