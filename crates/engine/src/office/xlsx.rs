//! Spreadsheets (.xlsx) from a model's JSON, written by craft-xlsx.
//!
//! `{"sheets": [{"name", "columns": [..], "rows": [[..]], "widths": [..]}]}`. A cell is a
//! string, a number, a boolean or null; a string that starts with `=` is a formula (`'=` keeps it
//! as text). The `columns` row is the header: bold on the accent colour, frozen when scrolling.
//! Excel and LibreOffice compute the formulas when they open the file.

use std::collections::HashMap;

use aiscraft_core::{Error, Result};
use craft_xlsx::{
    Cell, CellFormula, CellRef, CellStyle, CellValue, Color, ColorSource, Column, Document, Fill, Font, Pane, Pattern, PatternFill, Row, SheetEntry, SheetKind,
    SheetView, StringItem, Styles, Visibility, Workbook, Worksheet, Xf,
};
use craft_xml::El;
use serde_json::Value;

pub const MAX_SHEETS: usize = 20;
pub const MAX_ROWS: usize = 10_000;
pub const MAX_COLS: usize = 200;
pub const MAX_CELLS: usize = 200_000;
/// Excel's own limit for one cell's text.
pub const MAX_CELL_CHARS: usize = 32_767;
pub const MAX_FORMULA_CHARS: usize = 8_000;

/// A sheet name Excel accepts: at most 31 characters, none of `[]:*?/\`, not empty, unique.
fn sheet_name(raw: &str, n: usize, taken: &[String]) -> String {
    let clean: String = raw.chars().filter(|c| !"[]:*?/\\".contains(*c) && !c.is_control()).take(31).collect();
    let clean = clean.trim().trim_matches('\'').to_string();
    let base = if clean.is_empty() { format!("Tabelle{n}") } else { clean };
    let mut name = base.clone();
    let mut k = 2;
    while taken.iter().any(|t| t.to_lowercase() == name.to_lowercase()) {
        let suffix = format!(" ({k})");
        name = format!("{}{suffix}", base.chars().take(31 - suffix.chars().count()).collect::<String>());
        k += 1;
    }
    name
}

struct Strings {
    items: Vec<StringItem>,
    index: HashMap<String, u32>,
}

impl Strings {
    fn add(&mut self, s: &str) -> u32 {
        if let Some(i) = self.index.get(s) {
            return *i;
        }
        let i = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        self.items.push(StringItem::plain(s));
        self.index.insert(s.to_string(), i);
        i
    }
}

/// (value, formula) of one cell from JSON.
fn cell_value(v: &Value, strings: &mut Strings) -> Result<Option<(CellValue, Option<CellFormula>)>> {
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Bool(b) => (CellValue::Bool(*b), None),
        Value::Number(n) => match n.as_f64().filter(|f| f.is_finite()) {
            Some(f) => (CellValue::Number(f), None),
            None => return Ok(None),
        },
        Value::String(s) => {
            if let Some(f) = s.strip_prefix('=').map(str::trim).filter(|f| !f.is_empty()) {
                if f.chars().count() > MAX_FORMULA_CHARS {
                    return Err(Error::invalid(format!("Eine Formel ist länger als {MAX_FORMULA_CHARS} Zeichen")));
                }
                (CellValue::Empty, Some(CellFormula { text: f.to_string(), ..CellFormula::default() }))
            } else {
                let text = s.strip_prefix("'=").map(|r| format!("={r}")).unwrap_or_else(|| s.clone());
                if text.is_empty() {
                    return Ok(None);
                }
                let text: String = text.chars().take(MAX_CELL_CHARS).collect();
                (CellValue::Shared(strings.add(&text)), None)
            }
        }
        other => {
            let text: String = other.to_string().chars().take(MAX_CELL_CHARS).collect();
            (CellValue::Shared(strings.add(&text)), None)
        }
    }))
}

fn styles() -> Styles {
    let font = |bold: bool, white: bool| Font {
        name: Some("Calibri".into()),
        size: Some(11.0),
        bold: bold.then_some(true),
        color: white.then_some(Color { source: ColorSource::Rgb(0xFFFF_FFFF), tint: 0.0 }),
        ..Font::default()
    };
    let solid = Fill::Pattern(PatternFill {
        pattern: Some(Pattern::Solid),
        fg: Some(Color { source: ColorSource::Rgb(0xFF10_7FAD), tint: 0.0 }),
        bg: Some(Color { source: ColorSource::Indexed(64), tint: 0.0 }),
    });
    let header = Xf { font_id: 1, fill_id: 2, xf_id: Some(0), el: El::new("xf").with_attr("applyFont", "1").with_attr("applyFill", "1"), ..Xf::default() };
    Styles {
        fonts: vec![font(false, false), font(true, true)],
        // Excel wants these two fills first.
        fills: vec![
            Fill::Pattern(PatternFill { pattern: Some(Pattern::None), ..PatternFill::default() }),
            Fill::Pattern(PatternFill { pattern: Some(Pattern::Gray125), ..PatternFill::default() }),
            solid,
        ],
        borders: vec![Default::default()],
        cell_style_xfs: vec![Xf::default()],
        cell_xfs: vec![Xf { xf_id: Some(0), ..Xf::default() }, header],
        cell_styles: vec![CellStyle { name: "Normal".into(), xf_id: 0, builtin_id: Some(0), ..CellStyle::default() }],
        ..Styles::default()
    }
}

fn rows_of(v: Option<&Value>) -> Vec<Vec<Value>> {
    v.and_then(Value::as_array).map(|rows| rows.iter().map(|r| r.as_array().cloned().unwrap_or_else(|| vec![r.clone()])).collect()).unwrap_or_default()
}

/// The workbook described by `spec`, as the bytes of an .xlsx file.
pub fn workbook_xlsx(spec: &Value) -> Result<Vec<u8>> {
    let sheets = spec.get("sheets").and_then(Value::as_array).ok_or_else(|| Error::invalid("sheets fehlt (eine Liste von Tabellenblättern)"))?;
    if sheets.is_empty() {
        return Err(Error::invalid("Die Tabelle braucht mindestens ein Tabellenblatt"));
    }
    if sheets.len() > MAX_SHEETS {
        return Err(Error::invalid(format!("Höchstens {MAX_SHEETS} Tabellenblätter")));
    }
    let mut strings = Strings { items: Vec::new(), index: HashMap::new() };
    let mut entries = Vec::new();
    let mut worksheets = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut cells_total = 0usize;
    for (i, sheet) in sheets.iter().enumerate() {
        let name = sheet_name(sheet.get("name").and_then(Value::as_str).unwrap_or(""), i + 1, &names);
        names.push(name.clone());
        let header: Vec<Value> = sheet.get("columns").and_then(Value::as_array).cloned().unwrap_or_default();
        let body = rows_of(sheet.get("rows"));
        let has_header = !header.is_empty();
        let all: Vec<(bool, Vec<Value>)> = has_header.then_some((true, header)).into_iter().chain(body.into_iter().map(|r| (false, r))).collect();
        if all.len() > MAX_ROWS {
            return Err(Error::invalid(format!("Tabellenblatt „{name}“ hat mehr als {MAX_ROWS} Zeilen")));
        }
        let mut rows = Vec::new();
        let mut widths: Vec<usize> = Vec::new();
        for (r, (is_header, values)) in all.iter().enumerate() {
            if values.len() > MAX_COLS {
                return Err(Error::invalid(format!("Tabellenblatt „{name}“ hat mehr als {MAX_COLS} Spalten")));
            }
            cells_total += values.len();
            if cells_total > MAX_CELLS {
                return Err(Error::invalid(format!("Die Tabelle hat mehr als {MAX_CELLS} Zellen")));
            }
            let mut cells = Vec::new();
            for (c, v) in values.iter().enumerate() {
                let Some((value, formula)) = cell_value(v, &mut strings)? else { continue };
                let shown = match v {
                    Value::String(s) => s.chars().count(),
                    other => other.to_string().len(),
                };
                if widths.len() <= c {
                    widths.resize(c + 1, 0);
                }
                if let Some(w) = widths.get_mut(c) {
                    *w = (*w).max(if formula.is_some() { 10 } else { shown });
                }
                let at = CellRef::new(u32::try_from(r).unwrap_or(0), u32::try_from(c).unwrap_or(0));
                cells.push(Cell { at, style: u32::from(*is_header), value, formula, ..Cell::default() });
            }
            rows.push(Row { index: u32::try_from(r).unwrap_or(0), cells, ..Row::default() });
        }
        let given: Vec<f64> = sheet.get("widths").and_then(Value::as_array).map(|w| w.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let columns: Vec<Column> = widths
            .iter()
            .enumerate()
            .map(|(c, w)| {
                let width = given.get(c).copied().filter(|g| g.is_finite() && *g > 0.0).unwrap_or((*w as f64 + 2.0).clamp(8.0, 60.0)).min(255.0);
                let c = u32::try_from(c).unwrap_or(0);
                Column { first: c, last: c, width: Some(width), custom_width: true, ..Column::default() }
            })
            .collect();
        let mut ws = Worksheet { rows, ..Worksheet::default() };
        ws.part.columns = columns;
        let mut view = SheetView { workbook_view_id: Some(0), tab_selected: (i == 0).then_some(true), ..SheetView::default() };
        if has_header {
            view.pane = Some(Pane {
                y_split: Some(1.0),
                top_left_cell: Some("A2".into()),
                active_pane: Some("bottomLeft".into()),
                state: Some("frozen".into()),
                ..Pane::default()
            });
        }
        ws.part.features.views = vec![view];
        worksheets.push(Some(ws));
        entries.push(SheetEntry {
            name,
            sheet_id: u32::try_from(i + 1).unwrap_or(1),
            visibility: Visibility::Visible,
            rel_id: String::new(),
            kind: SheetKind::Worksheet,
            part: None,
        });
    }
    let mut root = El::new("workbook");
    root.push(El::new("calcPr").with_attr("calcId", "191029").with_attr("fullCalcOnLoad", "1"));
    let doc = Document {
        workbook: Workbook { sheets: entries, root, ..Workbook::default() },
        shared_strings: strings.items,
        styles: Some(styles()),
        worksheets,
        ..Document::default()
    };
    doc.write().map_err(|e| Error::invalid(format!("Tabelle ließ sich nicht schreiben: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn part(bytes: &[u8], name: &str) -> String {
        let pkg = craft_opc::Package::open(bytes, &craft_opc::Limits::new(1000, 64 << 20, 256 << 20)).unwrap();
        String::from_utf8(pkg.get(name).unwrap_or_else(|| panic!("no part {name}")).to_vec()).unwrap()
    }

    #[test]
    fn sheets_rows_formulas_and_header() {
        let spec = json!({"sheets": [
            {"name": "Noten 7b", "columns": ["Name", "Note", "Punkte"], "rows": [["Anna", 2, 13], ["Ben", 1.5, true], ["Summe", null, "=SUM(C2:C3)"], ["'=kein", "x"]]},
            {"name": "Noten 7b", "rows": [[1, 2]]},
            {"name": "a/b:c*?[]", "rows": []}
        ]});
        let bytes = workbook_xlsx(&spec).unwrap();
        assert!(bytes.starts_with(b"PK"));
        let wb = part(&bytes, "xl/workbook.xml");
        assert!(wb.contains("name=\"Noten 7b\"") && wb.contains("name=\"Noten 7b (2)\"") && wb.contains("name=\"abc\""), "{wb}");
        assert!(wb.contains("fullCalcOnLoad=\"1\""));
        let s1 = part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(s1.contains("<f>SUM(C2:C3)</f>"), "{s1}");
        assert!(s1.contains("r=\"A1\" s=\"1\""), "header styled: {s1}");
        assert!(s1.contains("state=\"frozen\""));
        assert!(s1.contains("<v>1.5</v>") && s1.contains("t=\"b\""));
        let shared = part(&bytes, "xl/sharedStrings.xml");
        assert!(shared.contains("<t>Anna</t>") && shared.contains("<t>=kein</t>"));
        let styles = part(&bytes, "xl/styles.xml");
        assert!(styles.contains("FF107FAD") && styles.contains("<b/>"), "{styles}");
        // craft-xlsx reads its own file back.
        let back = Document::read(&bytes, &craft_xlsx::Limits::default()).unwrap();
        assert_eq!(back.workbook.sheets.len(), 3);
        assert_eq!(back.worksheets[0].as_ref().unwrap().rows.len(), 5);
    }

    #[test]
    fn hostile_specs_are_refused() {
        assert!(workbook_xlsx(&json!({})).is_err());
        assert!(workbook_xlsx(&json!({"sheets": []})).is_err());
        let many: Vec<Value> = (0..=MAX_SHEETS).map(|_| json!({"rows": []})).collect();
        assert!(workbook_xlsx(&json!({"sheets": many})).is_err());
        let rows: Vec<Value> = (0..=MAX_ROWS).map(|i| json!([i])).collect();
        assert!(workbook_xlsx(&json!({"sheets": [{"rows": rows}]})).is_err());
        let wide: Vec<Value> = (0..=MAX_COLS).map(|i| json!(i)).collect();
        assert!(workbook_xlsx(&json!({"sheets": [{"rows": [wide]}]})).is_err());
        assert!(workbook_xlsx(&json!({"sheets": [{"rows": [[format!("={}", "1+".repeat(MAX_FORMULA_CHARS))]]}]})).is_err());
        // Odd values do not break it: objects become text, a row that is no list is one cell.
        let ok = workbook_xlsx(&json!({"sheets": [{"rows": ["einzeln", [{"a": 1}, [1, 2], 1e308]]}]})).unwrap();
        assert!(part(&ok, "xl/sharedStrings.xml").contains("einzeln"));
    }
}
