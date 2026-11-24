//! Documents AIS.chat does not take, read locally into Markdown: Word 97-2003 (.doc), RTF, EPUB,
//! e-mails (.eml, .mbox), calendars (.ics) and contacts (.vcf). Word (.docx) too, for backends
//! that cannot read it.

use aiscraft_core::{Error, Result};

/// Converted Markdown is cut here (AIS.chat reads files in 2,500-character sections anyway).
pub const MAX_CHARS: usize = 3_000_000;
const MAX_CHAPTERS: usize = 2_000;
const MAX_MESSAGES: usize = 1_000;
const MAX_EVENTS: usize = 5_000;
const MAX_CONTACTS: usize = 10_000;

fn locked(what: &str) -> Error {
    Error::unsupported(format!("{what} ist mit einem Passwort geschützt. Speichere sie ohne Passwort und hänge sie dann an."))
}

fn cut(mut s: String) -> String {
    if s.len() > MAX_CHARS {
        let mut end = MAX_CHARS;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n\n(gekürzt)\n");
    }
    s
}

fn html_markdown(html: &str) -> String {
    craft_markdown::export(&craft_html::doc::import(html))
}

/// Markdown for `ext`; `title` is the file name without extension.
pub fn markdown(ext: &str, title: &str, bytes: &[u8]) -> Result<String> {
    let md = match ext {
        "doc" | "dot" => {
            if craft_doc_binary::is_encrypted(bytes) {
                return Err(locked("Das Word-Dokument"));
            }
            let doc = craft_doc_binary::read(bytes).map_err(|e| Error::invalid(format!("Das Word-Dokument (.doc) lässt sich nicht lesen: {e}")))?;
            craft_markdown::export(&doc)
        }
        "docx" => {
            if super::is_cfb(bytes) {
                return Err(locked("Das Word-Dokument"));
            }
            let doc = craft_docx::read(bytes).map_err(|e| Error::invalid(format!("Das Word-Dokument lässt sich nicht lesen: {e}")))?;
            craft_markdown::export(&doc)
        }
        "rtf" => {
            let doc = craft_rtf::doc::import(bytes).map_err(|e| Error::invalid(format!("Die RTF-Datei lässt sich nicht lesen: {e}")))?;
            craft_markdown::export(&doc)
        }
        "epub" => epub(title, bytes)?,
        "eml" => mail(bytes, 1),
        "mbox" => mbox(title, bytes),
        "ics" => calendar(title, bytes),
        "vcf" => contacts(title, bytes),
        "html" | "htm" => html_markdown(&craft_html::doc::decode(bytes)),
        _ => return Err(Error::unsupported(format!(".{ext}-Dateien kann AIS.ChatCraft nicht lesen"))),
    };
    if md.trim().is_empty() {
        return Err(Error::invalid(format!("In {title}.{ext} steht kein Text, den AIS.ChatCraft lesen kann")));
    }
    Ok(cut(md))
}

fn epub(title: &str, bytes: &[u8]) -> Result<String> {
    let mut book = craft_epub::Epub::open(bytes).map_err(|e| Error::invalid(format!("Das E-Book lässt sich nicht lesen: {e}")))?;
    let encrypted = book.encryption().map(|e| e.iter().any(|x| !x.is_font_obfuscation())).unwrap_or(false);
    if encrypted {
        return Err(Error::unsupported("Das E-Book ist kopiergeschützt (DRM). AIS.ChatCraft kann es nicht lesen."));
    }
    let name = if book.package.title.trim().is_empty() { title.to_string() } else { book.package.title.trim().to_string() };
    let spine: Vec<String> = book.package.spine_paths().take(MAX_CHAPTERS).map(str::to_string).collect();
    let mut out = format!("# {name}\n\n");
    for path in spine {
        let Ok(Some(xhtml)) = book.read(&path) else { continue };
        out.push_str(&html_markdown(&craft_html::doc::decode(&xhtml)));
        out.push_str("\n\n");
        if out.len() > MAX_CHARS {
            break;
        }
    }
    Ok(out)
}

fn header(m: &craft_mime::Message, name: &str) -> Option<String> {
    m.root.headers.get(name).map(craft_mime::decode_words).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// One message: its headers as a list, the text body, the names of attached files.
fn mail(bytes: &[u8], level: usize) -> String {
    let m = craft_mime::parse(bytes, &craft_mime::Limits::default());
    let subject = header(&m, "subject").unwrap_or_else(|| "(ohne Betreff)".into());
    let mut out = format!("{} {}\n\n", "#".repeat(level), subject.replace('\n', " "));
    for (key, label) in [("from", "Von"), ("to", "An"), ("cc", "Cc"), ("date", "Datum")] {
        if let Some(v) = header(&m, key) {
            out.push_str(&format!("- {label}: {}\n", v.replace('\n', " ")));
        }
    }
    let leaves = m.root.leaves();
    let is_attachment = |p: &craft_mime::Part| p.file_name().is_some() || p.disposition().value == "attachment";
    let plain = leaves.iter().find(|p| p.media_type() == "text/plain" && !is_attachment(p));
    let html = leaves.iter().find(|p| p.media_type() == "text/html" && !is_attachment(p));
    let body = match (plain, html) {
        (Some(p), _) => p.text(),
        (None, Some(h)) => html_markdown(&h.text()),
        _ if leaves.len() <= 1 && !m.root.is_multipart() => m.root.text(),
        _ => String::new(),
    };
    out.push('\n');
    out.push_str(body.trim());
    out.push('\n');
    let files: Vec<String> = leaves.iter().filter(|p| is_attachment(p)).filter_map(|p| p.file_name()).collect();
    if !files.is_empty() {
        out.push_str(&format!("\nAnhänge (nicht mitgeschickt): {}\n", files.join(", ")));
    }
    out
}

fn mbox(title: &str, bytes: &[u8]) -> String {
    let mut out = format!("# {title}\n\n");
    for (i, msg) in craft_mime::mbox_messages(bytes).enumerate() {
        if i >= MAX_MESSAGES || out.len() > MAX_CHARS {
            out.push_str("\n(weitere Nachrichten ausgelassen)\n");
            break;
        }
        out.push_str(&mail(&msg, 2));
        out.push('\n');
    }
    out
}

fn when(v: &str) -> String {
    match craft_ical::When::parse(v) {
        Some(craft_ical::When::Date(d)) => d.format("%d.%m.%Y").to_string(),
        Some(craft_ical::When::Local(t)) => t.format("%d.%m.%Y %H:%M").to_string(),
        Some(craft_ical::When::Utc(t)) => t.format("%d.%m.%Y %H:%M UTC").to_string(),
        None => v.to_string(),
    }
}

/// Events and to-dos as a readable list, in file order.
fn calendar(title: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = format!("# {title}\n");
    let mut item: Option<Vec<(String, String)>> = None;
    let mut depth = 0usize;
    let mut count = 0;
    for raw in craft_ical::Lines::new(&text) {
        let Some(line) = craft_ical::ContentLine::parse(&raw) else { continue };
        match (line.name.as_str(), line.value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT" | "VTODO") if item.is_none() => {
                item = Some(Vec::new());
                depth = 0;
            }
            ("BEGIN", _) if item.is_some() => depth += 1,
            ("END", "VEVENT" | "VTODO") if depth == 0 => {
                if let Some(fields) = item.take() {
                    count += 1;
                    if count > MAX_EVENTS {
                        out.push_str("\n(weitere Termine ausgelassen)\n");
                        break;
                    }
                    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                    let summary = get("SUMMARY").map(|s| craft_ical::values::unescape_text(&s)).unwrap_or_else(|| "(ohne Titel)".into());
                    out.push_str(&format!("\n## {}\n\n", summary.replace('\n', " ")));
                    if let Some(s) = get("DTSTART") {
                        let end = get("DTEND").or_else(|| get("DUE")).map(|e| format!(" bis {}", when(&e))).unwrap_or_default();
                        out.push_str(&format!("- Wann: {}{end}\n", when(&s)));
                    }
                    if let Some(r) = get("RRULE") {
                        out.push_str(&format!("- Wiederholung: {r}\n"));
                    }
                    if let Some(l) = get("LOCATION") {
                        out.push_str(&format!("- Ort: {}\n", craft_ical::values::unescape_text(&l).replace('\n', ", ")));
                    }
                    if let Some(d) = get("DESCRIPTION") {
                        out.push_str(&format!("\n{}\n", craft_ical::values::unescape_text(&d).trim()));
                    }
                }
            }
            ("END", _) if item.is_some() => depth = depth.saturating_sub(1),
            (name, _) => {
                if depth == 0
                    && let Some(fields) = item.as_mut()
                    && fields.len() < 64
                {
                    fields.push((name.to_string(), line.value.chars().take(20_000).collect()));
                }
            }
        }
    }
    if count == 0 {
        return String::new();
    }
    out
}

fn contacts(title: &str, bytes: &[u8]) -> String {
    let cards = craft_vcard::parse_bytes(bytes);
    if cards.is_empty() {
        return String::new();
    }
    let mut out = format!("# {title}\n");
    for c in cards.iter().take(MAX_CONTACTS) {
        let name = c.name();
        out.push_str(&format!("\n## {}\n\n", if name.trim().is_empty() { "(ohne Namen)".to_string() } else { name.replace('\n', " ") }));
        let org: Vec<&str> = [c.title.as_str(), c.department.as_str(), c.company.as_str()].into_iter().filter(|s| !s.trim().is_empty()).collect();
        if !org.is_empty() {
            out.push_str(&format!("- {}\n", org.join(", ")));
        }
        for e in &c.emails {
            out.push_str(&format!("- E-Mail{}: {}\n", label(&e.label), e.value));
        }
        for p in &c.phones {
            out.push_str(&format!("- Telefon{}: {}\n", label(&p.label), p.value));
        }
        for a in &c.addresses {
            let parts: Vec<String> = [a.street.clone(), format!("{} {}", a.postal_code, a.city), a.country.clone()]
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !parts.is_empty() {
                out.push_str(&format!("- Adresse{}: {}\n", label(&a.label), parts.join(", ")));
            }
        }
        if let Some(b) = c.birthday {
            out.push_str(&format!("- Geburtstag: {}\n", b.format("%d.%m.%Y")));
        }
        if !c.notes.trim().is_empty() {
            out.push_str(&format!("- Notiz: {}\n", c.notes.trim().replace('\n', " ")));
        }
    }
    out
}

fn label(l: &str) -> String {
    if l.trim().is_empty() { String::new() } else { format!(" ({})", l.trim()) }
}
