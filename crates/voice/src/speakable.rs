//! Markdown to text that sounds right when read aloud, and sentence chunks for streaming TTS.

/// Chunks for streaming TTS stay below this many characters.
pub const CHUNK_CHARS: usize = 400;

/// Markdown answer → plain text for speech, one block (paragraph, heading, list item, table row)
/// per line. Code blocks become "Codeblock.", links their text, bare URLs "Link".
pub fn speakable(markdown: &str) -> String {
    blocks(markdown).join("\n")
}

/// `speakable`, cut into sentence chunks of at most [`CHUNK_CHARS`] characters.
pub fn speakable_chunks(markdown: &str) -> Vec<String> {
    chunks(&speakable(markdown), CHUNK_CHARS)
}

fn blocks(md: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut table: Vec<&str> = Vec::new();
    let flush_para = |para: &mut Vec<String>, out: &mut Vec<String>| {
        let text = collapse(&inline(&para.join(" ")));
        if !text.is_empty() {
            out.push(text);
        }
        para.clear();
    };
    for raw in md.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if let Some((ch, len)) = fence {
            if fence_marker(trimmed).is_some_and(|(c, l, rest)| c == ch && l >= len && rest.trim().is_empty()) {
                fence = None;
            }
            continue;
        }
        if !table.is_empty() && !trimmed.starts_with('|') {
            out.extend(table_rows(&table));
            table.clear();
        }
        if let Some((c, l, _)) = fence_marker(trimmed) {
            flush_para(&mut para, &mut out);
            out.push("Codeblock.".into());
            fence = Some((c, l));
            continue;
        }
        if trimmed.starts_with('|') {
            flush_para(&mut para, &mut out);
            table.push(trimmed);
            continue;
        }
        // Block quotes read like normal text.
        let mut body = trimmed;
        while let Some(rest) = body.strip_prefix('>') {
            body = rest.trim_start();
        }
        if body.is_empty() || is_rule(body) {
            flush_para(&mut para, &mut out);
            continue;
        }
        if let Some(h) = heading(body) {
            flush_para(&mut para, &mut out);
            push_sentence(&mut out, h);
            continue;
        }
        if let Some(item) = list_item(body) {
            flush_para(&mut para, &mut out);
            push_sentence(&mut out, item);
            continue;
        }
        para.push(body.to_string());
    }
    flush_para(&mut para, &mut out);
    if !table.is_empty() {
        out.extend(table_rows(&table));
    }
    out
}

/// A block of its own: cleaned, with a full stop so the voice pauses.
fn push_sentence(out: &mut Vec<String>, text: &str) {
    let t = collapse(&inline(text));
    if !t.is_empty() {
        out.push(terminate(t));
    }
}

fn terminate(mut t: String) -> String {
    if !t.ends_with(['.', '!', '?', ':', ';', '…']) {
        t.push('.');
    }
    t
}

/// "```rust" → ('`', 3, "rust").
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let c = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = line.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, line.get(n..).unwrap_or("")))
}

fn is_rule(line: &str) -> bool {
    let compact: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3 && ['-', '*', '_', '='].iter().any(|r| compact.iter().all(|c| c == r))
}

fn heading(line: &str) -> Option<&str> {
    let n = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    let rest = line.get(n..)?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim())
}

fn list_item(line: &str) -> Option<&str> {
    let rest = if let Some(r) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")) {
        r
    } else {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = line.get(digits..)?;
        after.strip_prefix(". ").or_else(|| after.strip_prefix(") "))?
    };
    let rest = rest.trim_start();
    let rest = ["[ ] ", "[x] ", "[X] "].iter().find_map(|p| rest.strip_prefix(p)).unwrap_or(rest);
    Some(rest)
}

fn cells(row: &str) -> Vec<String> {
    let r = row.trim();
    let r = r.strip_prefix('|').unwrap_or(r);
    let r = r.strip_suffix('|').unwrap_or(r);
    r.split('|').map(|c| collapse(&inline(c.trim()))).collect()
}

fn is_separator(row: &str) -> bool {
    let cs = cells(row);
    !cs.is_empty() && cs.iter().all(|c| !c.is_empty() && c.chars().all(|x| x == '-' || x == ':' || x == ' ') && c.contains('-'))
}

/// Each data row read as "Spalte: Wert, Spalte: Wert."
fn table_rows(lines: &[&str]) -> Vec<String> {
    let has_header = lines.get(1).is_some_and(|l| is_separator(l));
    let header: Vec<String> = if has_header { lines.first().map(|l| cells(l)).unwrap_or_default() } else { Vec::new() };
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if (has_header && i <= 1) || is_separator(line) {
            continue;
        }
        let parts: Vec<String> = cells(line)
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.is_empty())
            .map(|(j, c)| match header.get(j).filter(|h| !h.is_empty()) {
                Some(h) => format!("{h}: {c}"),
                None => c,
            })
            .collect();
        if !parts.is_empty() {
            out.push(terminate(parts.join(", ")));
        }
    }
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Find the index of the `close` that matches the `open` just before `from`.
fn matching(chars: &[char], from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in chars.iter().enumerate().skip(from) {
        if *c == open {
            depth += 1;
        } else if *c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        } else if *c == '\n' {
            return None;
        }
    }
    None
}

fn starts_with_at(chars: &[char], i: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(i + k) == Some(&p))
}

/// Inline Markdown → text.
fn inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i + 1).copied();
        match c {
            '\\' if next.is_some_and(|n| n.is_ascii_punctuation()) => {
                if let Some(n) = next {
                    out.push(n);
                }
                i += 2;
                continue;
            }
            '!' if next == Some('[') => {
                // Image: keep the alt text.
                if let Some(end) = matching(&chars, i + 2, '[', ']')
                    && chars.get(end + 1) == Some(&'(')
                    && let Some(close) = matching(&chars, end + 2, '(', ')')
                {
                    out.push_str(&inline(&chars.get(i + 2..end).unwrap_or_default().iter().collect::<String>()));
                    i = close + 1;
                    continue;
                }
            }
            '[' => {
                if next == Some('^')
                    && let Some(end) = matching(&chars, i + 1, '[', ']')
                {
                    i = end + 1;
                    continue;
                }
                if let Some(end) = matching(&chars, i + 1, '[', ']') {
                    let text: String = chars.get(i + 1..end).unwrap_or_default().iter().collect();
                    let after = chars.get(end + 1).copied();
                    let skip_to = match after {
                        Some('(') => matching(&chars, end + 2, '(', ')'),
                        Some('[') => matching(&chars, end + 2, '[', ']'),
                        _ => None,
                    };
                    if let Some(close) = skip_to {
                        out.push_str(&inline(&text));
                        i = close + 1;
                        continue;
                    }
                }
            }
            '<' => {
                let rest: String = chars.get(i + 1..).unwrap_or_default().iter().take(300).collect();
                if let Some(end) = rest.find('>') {
                    let inner = rest.get(..end).unwrap_or("");
                    let is_link = inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:");
                    let is_tag = inner.chars().next().is_some_and(|f| f.is_ascii_alphabetic() || f == '/' || f == '!') && !inner.contains('<');
                    if is_link || is_tag {
                        if is_link {
                            out.push_str("Link");
                        } else if inner.eq_ignore_ascii_case("br") || inner.eq_ignore_ascii_case("br/") || inner.eq_ignore_ascii_case("br /") {
                            out.push(' ');
                        }
                        i += 1 + inner.chars().count() + 1;
                        continue;
                    }
                }
            }
            'h' | 'w'
                if !prev.is_some_and(char::is_alphanumeric)
                    && (starts_with_at(&chars, i, "http://") || starts_with_at(&chars, i, "https://") || starts_with_at(&chars, i, "www.")) =>
            {
                let mut j = i;
                while chars.get(j).is_some_and(|x| !x.is_whitespace()) {
                    j += 1;
                }
                // Keep sentence punctuation that ends the URL.
                while j > i && chars.get(j - 1).is_some_and(|x| matches!(x, '.' | ',' | ';' | ':' | '!' | '?' | ')')) {
                    j -= 1;
                }
                out.push_str("Link");
                i = j;
                continue;
            }
            '`' => {
                i += 1;
                continue;
            }
            '*' | '~' => {
                i += 1;
                continue;
            }
            '_' => {
                let intra = prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric);
                if !intra {
                    i += 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    out
}

const ABBREVIATIONS: &[&str] = &[
    "abs", "abb", "art", "bspw", "bzw", "ca", "dr", "etc", "evtl", "ggf", "hr", "fr", "inkl", "jh", "kap", "max", "min", "mio", "mrd", "nr", "prof", "sog",
    "st", "str", "tel", "usw", "vgl", "zb", "zzgl", "mr", "mrs", "ms", "vs", "eg", "ie", "bzgl", "insb", "lt", "mind",
];

/// Split one line into sentences, keeping German abbreviations ("z. B.", "Nr.") and ordinals
/// ("3. Oktober") together.
fn sentences(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        if matches!(c, '.' | '!' | '?' | '…') {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|x| matches!(x, '.' | '!' | '?' | '…' | '"' | '\'' | ')' | '»' | '«' | '“' | '”')) {
                j += 1;
            }
            let at_space = chars.get(j).is_none_or(|x| x.is_whitespace());
            let next_word = chars.get(j..).unwrap_or_default().iter().find(|x| !x.is_whitespace()).copied();
            let mut boundary = at_space && !next_word.is_some_and(char::is_lowercase);
            if boundary && c == '.' && j == i + 1 {
                let word: String =
                    chars.get(start..i).unwrap_or_default().iter().rev().take_while(|x| x.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
                let lower = word.to_lowercase();
                if word.chars().count() == 1 || (!word.is_empty() && word.chars().all(|x| x.is_ascii_digit())) || ABBREVIATIONS.contains(&lower.as_str()) {
                    boundary = false;
                }
            }
            if boundary {
                let s: String = chars.get(start..j).unwrap_or_default().iter().collect();
                if !s.trim().is_empty() {
                    out.push(s.trim().to_string());
                }
                start = j;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    let rest: String = chars.get(start..).unwrap_or_default().iter().collect();
    if !rest.trim().is_empty() {
        out.push(rest.trim().to_string());
    }
    out
}

/// Cut an over-long sentence at the best break before `max` characters.
fn hard_split(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = s.chars().collect();
    while rest.len() > max {
        let window = rest.get(..max).unwrap_or_default();
        let at = [", ", "; ", ": ", " – ", " - ", " "]
            .iter()
            .find_map(|sep| {
                let w: String = window.iter().collect();
                w.rfind(sep).map(|b| w.get(..b).map(|p| p.chars().count() + sep.trim_end().chars().count()).unwrap_or(0))
            })
            .filter(|n| *n > max / 4)
            .unwrap_or(max);
        let head: String = rest.get(..at).unwrap_or_default().iter().collect();
        if !head.trim().is_empty() {
            out.push(head.trim().to_string());
        }
        rest = rest.get(at..).unwrap_or_default().to_vec();
    }
    let tail: String = rest.iter().collect();
    if !tail.trim().is_empty() {
        out.push(tail.trim().to_string());
    }
    out
}

/// Pack sentences into chunks of at most `max` characters (`max` ≥ 20).
pub fn chunks(text: &str, max: usize) -> Vec<String> {
    let max = max.max(20);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for line in text.lines() {
        for s in sentences(line) {
            for piece in hard_split(&s, max) {
                let n = piece.chars().count();
                if cur_len > 0 && cur_len + 1 + n > max {
                    out.push(std::mem::take(&mut cur));
                    cur_len = 0;
                }
                if cur_len > 0 {
                    cur.push(' ');
                    cur_len += 1;
                }
                cur.push_str(&piece);
                cur_len += n;
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_emphasis_and_headings() {
        assert_eq!(speakable("# Titel\n\nDas ist **fett** und *kursiv* und ~~weg~~ und `code`."), "Titel.\nDas ist fett und kursiv und weg und code.");
        assert_eq!(speakable("## Frage?"), "Frage?");
        assert_eq!(speakable("snake_case bleibt, _betont_ nicht"), "snake_case bleibt, betont nicht");
        assert_eq!(speakable("#hashtag"), "#hashtag");
    }

    #[test]
    fn code_blocks_become_a_word() {
        let md = "Hier:\n\n```rust\nfn main() { println!(\"x\"); }\n```\n\nFertig.";
        assert_eq!(speakable(md), "Hier:\nCodeblock.\nFertig.");
        // An unclosed fence swallows the rest instead of reading code aloud.
        assert_eq!(speakable("A\n~~~~\ncode\n```\nmehr"), "A\nCodeblock.");
    }

    #[test]
    fn links_images_urls_html() {
        assert_eq!(speakable("Siehe [die Doku](https://x.de/a_(b)) und ![ein Bild](i.png)."), "Siehe die Doku und ein Bild.");
        assert_eq!(speakable("Mehr unter https://example.org/x?y=1. Danke"), "Mehr unter Link. Danke");
        assert_eq!(speakable("Quelle <https://a.b> und<br>neu<b>fett</b>"), "Quelle Link und neufett");
        assert_eq!(speakable("Fußnote[^1] und [Ref][1]"), "Fußnote und Ref");
        assert_eq!(speakable("3 < 4 und \\*kein\\* Stern"), "3 < 4 und *kein* Stern");
    }

    #[test]
    fn lists_and_quotes() {
        assert_eq!(speakable("- eins\n- zwei!\n1. drei\n2) vier\n- [x] erledigt"), "eins.\nzwei!\ndrei.\nvier.\nerledigt.");
        assert_eq!(speakable("> zitiert\n> weiter"), "zitiert weiter");
        assert_eq!(speakable("oben\n\n---\n\nunten"), "oben\nunten");
    }

    #[test]
    fn tables_read_by_row() {
        let md = "| Land | Hauptstadt |\n|---|:---:|\n| Bayern | München |\n| Berlin | **Berlin** |\nDanach";
        assert_eq!(speakable(md), "Land: Bayern, Hauptstadt: München.\nLand: Berlin, Hauptstadt: Berlin.\nDanach");
        assert_eq!(speakable("| a | b |\n| c | |"), "a, b.\nc.");
    }

    #[test]
    fn sentence_splitting() {
        assert_eq!(
            sentences("Das ist z. B. gut. Am 3. Oktober ist frei! Wirklich? ja."),
            vec!["Das ist z. B. gut.", "Am 3. Oktober ist frei!", "Wirklich? ja."]
        );
        assert_eq!(sentences("Vgl. Nr. 5 bzw. Abs. 2. Ende"), vec!["Vgl. Nr. 5 bzw. Abs. 2. Ende"]);
        assert_eq!(sentences("Er sagte: „Hallo.“ Dann ging er…"), vec!["Er sagte: „Hallo.“", "Dann ging er…"]);
        assert_eq!(sentences("Version 1.2.3 ist da."), vec!["Version 1.2.3 ist da."]);
    }

    #[test]
    fn chunks_respect_limit() {
        let text = "Erster Satz ist hier. ".repeat(50);
        let cs = chunks(&text, 100);
        assert!(cs.len() > 5);
        assert!(cs.iter().all(|c| c.chars().count() <= 100), "{cs:?}");
        assert_eq!(cs.join(" "), text.trim());
        // A sentence without breaks is cut at spaces/commas.
        let long = format!("{}.", ["Wort"; 200].join(", "));
        let cs = chunks(&long, 120);
        assert!(cs.iter().all(|c| c.chars().count() <= 120 && !c.is_empty()), "{cs:?}");
        assert!(cs.first().is_some_and(|c| c.ends_with(',')));
        // Umlauts count as one character, and a word longer than the limit is cut.
        let cs = chunks(&"ä".repeat(250), 100);
        assert_eq!(cs.iter().map(|c| c.chars().count()).collect::<Vec<_>>(), vec![100, 100, 50]);
        assert!(chunks("", 100).is_empty());
    }

    #[test]
    fn speakable_chunks_end_to_end() {
        let md = "# Überblick\n\nKurz gesagt: **ja**. Hier ist [mehr](http://x).\n\n```\ncode\n```";
        assert_eq!(speakable_chunks(md), vec!["Überblick. Kurz gesagt: ja. Hier ist mehr. Codeblock."]);
    }
}
