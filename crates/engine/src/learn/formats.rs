//! Output formats (#56): content a model delivers in one marked block, so backends without
//! tool calling (AIS.chat) can create artifacts, and the same formats as tools for backends
//! that have them. Learning apps (```` ```html ````) and worksheets (```` ```aiscraft-worksheet ````)
//! are the first two entries; the whiteboard (`aiscraft-board`), calendar and diagram formats
//! join this list later.
//!
//! Each entry has the fence languages it is recognised by, a compact description for the
//! prompt (sent only when the format is asked for), a lenient check, the artifact kind it
//! becomes and the tool that creates it on tool backends.

use aiscraft_core::{ArtifactKind, worksheet};

pub struct OutputFormat {
    /// Stable id (`learning_app`, `worksheet`).
    pub id: &'static str,
    pub label: &'static str,
    /// Fence languages that carry this format.
    pub fences: &'static [&'static str],
    pub kind: ArtifactKind,
    /// The tool that creates it on backends with tool calling.
    pub tool: &'static str,
    /// What the model is told, with an example, when the format is asked for.
    pub prompt_description: fn() -> String,
    /// The blocks of this format in an answer (whole, closed blocks only).
    pub find: fn(&str) -> Vec<&str>,
    /// Lenient check of one block: the problems found, or why it is unusable.
    pub check: fn(&str) -> Result<Vec<String>, String>,
}

fn app_description() -> String {
    super::APP_RULES.to_string()
}

fn app_check(src: &str) -> Result<Vec<String>, String> {
    if super::is_html_document(src) { Ok(Vec::new()) } else { Err("keine vollständige HTML-Datei (<!doctype html> bis </html>, höchstens 2 MB)".into()) }
}

fn worksheet_description() -> String {
    worksheet::prompt_schema().to_string()
}

fn worksheet_check(src: &str) -> Result<Vec<String>, String> {
    worksheet::parse_lenient(src).map(|(_, problems)| problems)
}

static FORMATS: &[OutputFormat] = &[
    OutputFormat {
        id: "learning_app",
        label: "Lern-App",
        fences: &["html", "htm", "xhtml"],
        kind: ArtifactKind::App,
        tool: "learning_app_create",
        prompt_description: app_description,
        find: super::html_documents,
        check: app_check,
    },
    OutputFormat {
        id: "worksheet",
        label: "Arbeitsblatt",
        fences: &[worksheet::FENCE, "aiscraft-arbeitsblatt"],
        kind: ArtifactKind::Worksheet,
        tool: "worksheet_create",
        prompt_description: worksheet_description,
        find: super::worksheet_blocks,
        check: worksheet_check,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn registry_finds_and_checks_both_formats() {
        let answer = "```html\n<!doctype html><html><body>x</body></html>\n```\n```aiscraft-worksheet\n{\"tasks\":[{\"type\":\"true_false\",\"prompt\":\"Ja?\",\"answer\":true}]}\n```";
        let found = blocks(answer);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0.id, "learning_app");
        assert_eq!(found[1].0.kind, ArtifactKind::Worksheet);
        for (f, b) in found {
            assert!((f.check)(b).unwrap().is_empty());
            assert!(!(f.prompt_description)().is_empty());
            assert!(super::super::tool_specs().iter().any(|t| t.name == f.tool), "every format has its tool");
        }
        assert!((by_id("worksheet").unwrap().check)("{kaputt").is_err());
        assert!(by_id("board").is_none());
    }
}
