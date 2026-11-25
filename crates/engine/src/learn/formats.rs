//! Output formats (#56): content a model delivers in one marked block, so backends without
//! tool calling (AIS.chat) can create artifacts, and the same formats as tools for backends
//! that have them. Learning apps (```` ```html ````) and worksheets (```` ```aiscraft-worksheet ````)
//! are the first two entries, H5P (```` ```aiscraft-h5p ````, #59) the third, the whiteboard
//! (```` ```aiscraft-board ````, content only, laid out by `aiscraft_board::spec`) the fourth,
//! dates (```` ```ics ````, #51) the fifth: they become a calendar file, not an artifact.
//!
//! Each entry has the fence languages it is recognised by, a compact description for the
//! prompt (sent only when the format is asked for), a lenient check, the artifact kind it
//! becomes and the tool that creates it on tool backends.

use aiscraft_core::{ArtifactKind, h5p, worksheet};

pub struct OutputFormat {
    /// Stable id (`learning_app`, `worksheet`).
    pub id: &'static str,
    pub label: &'static str,
    /// Fence languages that carry this format.
    pub fences: &'static [&'static str],
    /// The artifact it becomes; `None`: a file to save (calendar).
    pub kind: Option<ArtifactKind>,
    /// The tool that creates it on backends with tool calling.
    pub tool: &'static str,
    /// First words of the request sent for this format, followed by the topic. The chat shows
    /// only the topic and folds the rest away as „Formatvorgabe“ (see [`split_request`]).
    pub request_intro: &'static str,
    /// What the model is told, with an example, when the format is asked for.
    pub prompt_description: fn(&PromptOptions) -> String,
    /// The blocks of this format in an answer (whole, closed blocks only).
    pub find: fn(&str) -> Vec<&str>,
    /// Lenient check of one block: the problems found, or why it is unusable.
    pub check: fn(&str) -> Result<Vec<String>, String>,
}

/// Settings that change what a model is told about a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptOptions {
    /// New worksheets count points (setting „Arbeitsblätter mit Punkten“).
    pub worksheet_points: bool,
}

impl Default for PromptOptions {
    fn default() -> Self {
        PromptOptions { worksheet_points: true }
    }
}

fn app_description(_: &PromptOptions) -> String {
    super::APP_RULES.to_string()
}

fn app_check(src: &str) -> Result<Vec<String>, String> {
    if super::is_html_document(src) { Ok(Vec::new()) } else { Err("keine vollständige HTML-Datei (<!doctype html> bis </html>, höchstens 2 MB)".into()) }
}

pub(crate) fn worksheet_description(o: &PromptOptions) -> String {
    worksheet::prompt_schema_for(o.worksheet_points)
}

fn worksheet_check(src: &str) -> Result<Vec<String>, String> {
    worksheet::parse_lenient(src).map(|(_, problems)| problems)
}

fn h5p_description(_: &PromptOptions) -> String {
    h5p::prompt_schema().to_string()
}

fn h5p_check(src: &str) -> Result<Vec<String>, String> {
    h5p::parse_lenient(src).map(|(h, mut problems)| {
        problems.extend(h5p::validate(&h));
        problems
    })
}

fn board_description(_: &PromptOptions) -> String {
    super::board::description()
}

fn calendar_description(_: &PromptOptions) -> String {
    super::calendar::description()
}

pub const APP_INTRO: &str = "Erstelle eine interaktive Lern-App zum Thema: ";
pub const WORKSHEET_INTRO: &str = "Erstelle ein interaktives Arbeitsblatt zum Thema: ";

static FORMATS: &[OutputFormat] = &[
    OutputFormat {
        id: "learning_app",
        label: "Lern-App",
        fences: &["html", "htm", "xhtml"],
        kind: Some(ArtifactKind::App),
        tool: "learning_app_create",
        request_intro: APP_INTRO,
        prompt_description: app_description,
        find: super::html_documents,
        check: app_check,
    },
    OutputFormat {
        id: "worksheet",
        label: "Arbeitsblatt",
        fences: &[worksheet::FENCE, "aiscraft-arbeitsblatt"],
        kind: Some(ArtifactKind::Worksheet),
        tool: "worksheet_create",
        request_intro: WORKSHEET_INTRO,
        prompt_description: worksheet_description,
        find: super::worksheet_blocks,
        check: worksheet_check,
    },
    OutputFormat {
        id: "h5p",
        label: "H5P",
        fences: &[h5p::FENCE],
        kind: Some(ArtifactKind::H5p),
        tool: "h5p_create",
        request_intro: super::h5p::H5P_INTRO,
        prompt_description: h5p_description,
        find: super::h5p::h5p_blocks,
        check: h5p_check,
    },
    OutputFormat {
        id: "board",
        label: "Whiteboard",
        fences: &[aiscraft_board::spec::FENCE, "aiscraft-whiteboard"],
        kind: Some(ArtifactKind::Board),
        tool: "board_create",
        request_intro: super::board::BOARD_INTRO,
        prompt_description: board_description,
        find: super::board::board_blocks,
        check: super::board::check,
    },
    OutputFormat {
        id: "calendar",
        label: "Termine",
        fences: &["ics", "ical", "icalendar"],
        kind: None,
        tool: "calendar_file_create",
        request_intro: super::calendar::CAL_INTRO,
        prompt_description: calendar_description,
        find: super::calendar::ics_blocks,
        check: super::calendar::check,
    },
];

pub fn formats() -> &'static [OutputFormat] {
    FORMATS
}

pub fn by_id(id: &str) -> Option<&'static OutputFormat> {
    FORMATS.iter().find(|f| f.id == id)
}

/// Every block of a known format in an answer, in format order.
pub fn blocks(answer: &str) -> Vec<(&'static OutputFormat, &str)> {
    FORMATS.iter().flat_map(|f| (f.find)(answer).into_iter().map(move |b| (f, b))).collect()
}

/// A format request as the chat shows it: the teacher's own words and the format spec that
/// went along with them. The message keeps the whole text (the backend needs it, also when the
/// chat continues); only the view and the exports split it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatRequest<'a> {
    pub format: &'static OutputFormat,
    /// What the teacher asked for (topic, subject, grade), one line.
    pub topic: &'a str,
    /// Everything after the topic line: task rules, format description, delivery.
    pub spec: &'a str,
}

impl std::fmt::Debug for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

impl PartialEq for OutputFormat {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for OutputFormat {}

/// Recognises a request made by `learning_app.request`, `worksheet.request` or any other entry
/// of the registry, also in messages saved before the split existed (the intro never changed).
/// Plain messages that only start like a request, without a spec after the topic, stay whole.
pub fn split_request(text: &str) -> Option<FormatRequest<'_>> {
    let text = text.trim_start();
    FORMATS.iter().find_map(|f| {
        let rest = text.strip_prefix(f.request_intro)?;
        let (topic, spec) = rest.split_once("\n\n")?;
        let (topic, spec) = (topic.trim(), spec.trim());
        (!topic.is_empty() && !spec.is_empty()).then_some(FormatRequest { format: f, topic, spec })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn registry_finds_and_checks_both_formats() {
        let answer = "```html\n<!doctype html><html><body>x</body></html>\n```\n```aiscraft-worksheet\n{\"tasks\":[{\"type\":\"true_false\",\"prompt\":\"Ja?\",\"answer\":true}]}\n```";
        let found = blocks(answer);
        assert_eq!(found.len(), 2);
        assert_eq!(blocks("```aiscraft-h5p\n{\"tasks\":[{\"type\":\"true_false\",\"prompt\":\"Ja?\",\"answer\":true}]}\n```")[0].0.id, "h5p");
        assert_eq!(found[0].0.id, "learning_app");
        assert_eq!(found[1].0.kind, Some(ArtifactKind::Worksheet));
        for (f, b) in found {
            assert!((f.check)(b).unwrap().is_empty());
            assert!(!(f.prompt_description)(&PromptOptions::default()).is_empty());
            assert!(super::super::tool_specs(true).iter().any(|t| t.name == f.tool), "every format has its tool");
        }
        assert!((by_id("worksheet").unwrap().check)("{kaputt").is_err());
        assert!(by_id("diagramm").is_none());
        let board = "```aiscraft-board\n{\"nodes\":[\"A\"]}\n```\n```aiscraft-board\n- nur eine Liste\n```";
        let found = blocks(board);
        assert_eq!(found.len(), 1, "an indented list is a change block, not the format");
        assert_eq!(found[0].0.id, "board");
        assert!((found[0].0.check)(found[0].1).unwrap().is_empty());
        let cal = blocks("```ics\nBEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Elternabend\nDTSTART:20261203T190000\nEND:VEVENT\nEND:VCALENDAR\n```");
        assert_eq!((cal.len(), cal[0].0.id, cal[0].0.kind), (1, "calendar", None));
        assert!((cal[0].0.check)(cal[0].1).unwrap().is_empty());
    }

    #[test]
    fn split_request_recognises_every_format_and_old_messages() {
        for f in formats() {
            let text = format!("{}Brüche (Klasse: 6)\n\nRegeln\n\nmehr", f.request_intro);
            let r = split_request(&text).unwrap();
            assert_eq!((r.format.id, r.topic, r.spec), (f.id, "Brüche (Klasse: 6)", "Regeln\n\nmehr"));
            // Only the intro, no spec: the message stays whole.
            assert!(split_request(&format!("{}Brüche", f.request_intro)).is_none());
            assert!(split_request(&format!("{}\n\nRegeln", f.request_intro)).is_none());
        }
        // Saved before the split existed (older schema wording): still recognised.
        let old = "Erstelle ein interaktives Arbeitsblatt zum Thema: Photosynthese\n\nEtwa 8 Aufgaben, verschiedene Aufgabentypen.\n\nFormat aiscraft-worksheet (JSON): {\"tasks\": []}\n\nAntworte mit genau einem ```aiscraft-worksheet-Codeblock.";
        let r = split_request(old).unwrap();
        assert_eq!((r.format.id, r.topic), ("worksheet", "Photosynthese"));
        assert!(r.spec.starts_with("Etwa 8 Aufgaben") && r.spec.ends_with("Codeblock."));
        assert!(split_request("Wie erstelle ich ein Arbeitsblatt?").is_none());
    }
}
