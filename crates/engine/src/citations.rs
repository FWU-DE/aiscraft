//! Sources and citations (#39): which passage of an attached document or a web page backs a
//! statement of an answer. A click shows the passage where it stands in the document.

use aiscraft_core::{Citation, Error, Result, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;

/// Text shown before and after the passage, in characters.
const AROUND: usize = 400;

/// A citation opened from an answer: the passage with the text around it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CitationView {
    pub message_id: String,
    pub index: usize,
    pub citation: Citation,
    /// The document's text before and after the passage, when it is at hand.
    pub before: String,
    pub passage: String,
    pub after: String,
    /// The attached file, for opening it (PDFs: at the page named in `citation.pages`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The passage was found in the attached document.
    pub found: bool,
}

/// `(before, passage, after)` around characters `start..end` of `text`.
fn around(text: &str, start: usize, end: usize) -> (String, String, String) {
    let chars: Vec<char> = text.chars().collect();
    let start = start.min(chars.len());
    let end = end.clamp(start, chars.len());
    let from = start.saturating_sub(AROUND);
    let to = (end + AROUND).min(chars.len());
    let s = |a: usize, b: usize| chars.get(a..b).map(|c| c.iter().collect::<String>()).unwrap_or_default();
    let mut before = s(from, start);
    if from > 0 {
        before.insert(0, '…');
    }
    let mut after = s(end, to);
    if to < chars.len() {
        after.push('…');
    }
    (before, s(start, end), after)
}

/// Where `needle` starts in `hay`, in characters.
fn char_find(hay: &str, needle: &str) -> Option<usize> {
    let byte = hay.find(needle)?;
    Some(hay.get(..byte)?.chars().count())
}

/// The passage in the document's text: at the characters the citation names when they hold the
/// cited text, else wherever the cited text stands.
pub(crate) fn locate(text: &str, c: &Citation) -> Option<(String, String, String)> {
    let cited = c.cited_text.trim();
    if let Some((s, e)) = c.chars {
        let (s, e) = (usize::try_from(s).ok()?, usize::try_from(e).ok()?);
        let (b, p, a) = around(text, s, e);
        if !p.trim().is_empty() && (cited.is_empty() || p.contains(cited) || cited.contains(p.trim())) {
            return Some((b, p, a));
        }
    }
    if cited.is_empty() {
        return None;
    }
    let start = char_find(text, cited)?;
    Some(around(text, start, start + cited.chars().count()))
}

/// `message.citation`: open a citation of an answer in the open chat, or close it.
pub(crate) fn open(c: &Ctx, p: &Value) -> Result<Value> {
    if p.get("close").and_then(Value::as_bool).unwrap_or(false) {
        c.update(|s| s.citation = None);
        return Ok(json!({}));
    }
    let message_id = p.get("message_id").and_then(Value::as_str).ok_or_else(|| Error::invalid("message_id fehlt"))?;
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| Error::invalid("index fehlt (eine Zahl ab 0)"))?;
    let state = c.state();
    let conv = state.current.clone().ok_or_else(|| Error::invalid("Kein Chat offen"))?;
    let m = state.messages.iter().find(|m| m.id == message_id).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    let mut citation = usize::try_from(index).ok().and_then(|i| m.citations.get(i)).cloned().ok_or_else(|| Error::not_found("Diesen Beleg gibt es nicht"))?;
    // What went out with placeholders of „Namen schützen“ is shown with the names (#55).
    citation.cited_text = crate::privacy::reveal(c, &conv, &citation.cited_text);
    citation.claim = crate::privacy::reveal(c, &conv, &citation.claim);
    // The attached file with this title, newest first.
    let file = if citation.url.is_none() {
        state.messages.iter().rev().filter(|m| m.role == Role::User).flat_map(|m| &m.attachments).find(|a| !a.is_link() && a.name == citation.title).cloned()
    } else {
        None
    };
    let text = file.as_ref().and_then(|a| {
        a.text.clone().or_else(|| {
            let path = a.local_path.as_deref()?;
            crate::files::is_text(&a.mime).then(|| std::fs::read_to_string(path).ok()).flatten()
        })
    });
    // Not attached: the chat's open document (sent as a document), else a file of the chat's
    // project knowledge (#32).
    let (text, file_path) = match (text, file.as_ref().and_then(|a| a.local_path.clone())) {
        (None, None) if citation.url.is_none() => match crate::artifacts::text_by_title(c, &conv, &citation.title) {
            Some(t) => (Some(t), None),
            None => match crate::knowledge::text_by_name(c, &conv, &citation.title) {
                Some((t, p)) => (Some(t), Some(p)),
                None => (None, None),
            },
        },
        other => other,
    };
    let located = text.as_deref().map(|t| crate::privacy::reveal(c, &conv, t)).and_then(|t| locate(&t, &citation));
    let found = located.is_some();
    let (before, passage, after) = located.unwrap_or_else(|| (String::new(), citation.cited_text.clone(), String::new()));
    let view = CitationView {
        message_id: message_id.to_string(),
        index: usize::try_from(index).unwrap_or(0),
        citation,
        before,
        passage,
        after,
        path: file_path,
        found,
    };
    let out = serde_json::to_value(&view)?;
    c.update(|s| s.citation = Some(view));
    Ok(out)
}

/// A `file://` address for a local path, with `#page=N` for a PDF page (browsers' PDF viewers
/// open there).
pub(crate) fn file_url(path: &str, page: Option<u64>) -> String {
    let unified = path.replace('\\', "/");
    let mut out = String::from("file://");
    if !unified.starts_with('/') {
        out.push('/');
    }
    for b in unified.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(char::from(b)),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    if let Some(p) = page {
        out.push_str(&format!("#page={p}"));
    }
    out
}

/// `message.citation_file`: open the file of the opened citation (or of `message_id`/`index`)
/// with the system: a PDF in the browser's PDF viewer at the page, other files with their
/// program. `dry_run` only says what would open.
pub(crate) fn open_file(c: &Ctx, p: &Value) -> Result<Value> {
    let view = if p.get("message_id").is_some() || p.get("index").is_some() {
        let v = open(c, p)?;
        serde_json::from_value::<CitationView>(v)?
    } else {
        c.state().citation.clone().ok_or_else(|| Error::invalid("Kein Beleg offen (message_id und index angeben)"))?
    };
    let path = view.path.clone().ok_or_else(|| Error::not_found("Zu diesem Beleg liegt keine Datei auf diesem Gerät"))?;
    if !std::path::Path::new(&path).is_file() {
        return Err(Error::not_found(format!("Die Datei {} gibt es nicht mehr", view.citation.title)));
    }
    let pdf = path.to_lowercase().ends_with(".pdf");
    let page = view.citation.pages.map(|(a, _)| a).filter(|_| pdf);
    let url = page.map(|n| file_url(&path, Some(n)));
    let out = json!({"path": path, "page": page, "url": url});
    if p.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(out);
    }
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keine Dateien (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    // A PDF at its page in the browser; when that fails, the file with its program and the page
    // named in a note.
    let at_page = url.as_deref().is_some_and(|u| webbrowser::open(u).is_ok());
    if !at_page {
        crate::learn::system_open(std::path::Path::new(&path))?;
        if let Some(n) = view.citation.pages.map(|(a, _)| a) {
            c.notice(crate::NoticeLevel::Info, format!("{} geöffnet – die Stelle steht auf Seite {n}.", view.citation.title));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_point_at_the_page() {
        assert_eq!(file_url("/home/a/Lehrplan Bio.pdf", Some(3)), "file:///home/a/Lehrplan%20Bio.pdf#page=3");
        assert_eq!(file_url(r"C:\Daten\plan#1.pdf", None), "file:///C:/Daten/plan%231.pdf");
        assert_eq!(file_url("/ä.pdf", Some(1)), "file:///%C3%A4.pdf#page=1");
    }

    #[test]
    fn the_passage_is_found_by_position_or_by_text() {
        let text = "Igel fressen Insekten. Im Winter halten sie Winterschlaf. Sie sind nachtaktiv.";
        let c = Citation { cited_text: "Im Winter halten sie Winterschlaf.".into(), chars: Some((23, 57)), ..Default::default() };
        let (b, p, a) = locate(text, &c).unwrap();
        assert_eq!((b.as_str(), p.as_str(), a.as_str()), ("Igel fressen Insekten. ", "Im Winter halten sie Winterschlaf.", " Sie sind nachtaktiv."));
        // Wrong numbers (another version of the file): found by its text.
        let moved = Citation { chars: Some((0, 4)), ..c.clone() };
        assert_eq!(locate(text, &moved).unwrap().1, "Im Winter halten sie Winterschlaf.");
        assert!(locate(text, &Citation { cited_text: "Kängurus".into(), ..Default::default() }).is_none());
        // Numbers past the end never panic.
        let far = Citation { cited_text: String::new(), chars: Some((10_000, 20_000)), ..Default::default() };
        assert!(locate(text, &far).is_none());
        let long = "ä".repeat(2000);
        let (b, p, _) = around(&long, 1000, 1001);
        assert!(b.starts_with('…') && p == "ä");
    }
}
