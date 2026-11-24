//! Spreadsheets (.xls, .xlsx, .ods) as tables of displayed text: one sheet becomes CSV, a whole
//! workbook Markdown with one table per sheet.

use std::collections::BTreeMap;

use aiscraft_core::{Error, Result};

/// At most this many cells are taken from a workbook, and this many rows and columns per sheet.
const MAX_CELLS: usize = 500_000;
const MAX_ROWS: u32 = 100_000;
const MAX_COLS: u32 = 1_000;
const MAX_SHEETS: usize = 200;
const MAX_CELL_CHARS: usize = 32_767;

/// One sheet: its name and the text of its non-empty cells by (row, column), zero-based.
#[derive(Debug, Default)]
pub struct Grid {
    pub name: String,
    pub cells: BTreeMap<(u32, u32), String>,
}

fn number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// craft-xls and craft-xlsx share craft-sheet's `Value`.
fn sheet_value(v: &craft_xlsx::Value) -> String {
    match v {
        craft_xlsx::Value::Empty => String::new(),
        craft_xlsx::Value::Number(n) => number(*n),
        craft_xlsx::Value::Text(t) => t.to_string(),
        craft_xlsx::Value::Bool(b) => if *b { "WAHR" } else { "FALSCH" }.to_string(),
        craft_xlsx::Value::Error(e) => format!("{e:?}"),
    }
}

struct Budget(usize);

impl Budget {
    fn take(&mut self) -> Result<()> {
        self.0 = self.0.checked_sub(1).ok_or_else(|| Error::invalid(format!("Die Tabelle ist zu groß (mehr als {MAX_CELLS} Zellen)")))?;
        Ok(())
    }
}

fn put(grid: &mut Grid, row: u32, col: u32, text: String, budget: &mut Budget) -> Result<()> {
    if text.is_empty() || row >= MAX_ROWS || col >= MAX_COLS {
        return Ok(());
    }
    budget.take()?;
    grid.cells.insert((row, col), text.chars().take(MAX_CELL_CHARS).collect());
    Ok(())
}

fn encrypted() -> Error {
    Error::unsupported("Die Tabelle ist mit einem Passwort geschützt. Speichere sie ohne Passwort und hänge sie dann an.")
}

/// The sheets of a workbook; with `only`, the cells of that sheet alone (the others empty).
pub fn read(ext: &str, bytes: &[u8], only: Option<usize>) -> Result<Vec<Grid>> {
    let mut budget = Budget(MAX_CELLS);
    let mut out = Vec::new();
    match ext {
        "xls" => {
            let read = craft_xls::read(bytes).map_err(|e| match e {
                craft_xls::Error::Encrypted | craft_xls::Error::WrongPassword | craft_xls::Error::UnsupportedEncryption(_) => encrypted(),
                craft_xls::Error::Biff5 => Error::unsupported("Excel-5.0/95-Dateien (vor 1997) kann AIS.ChatCraft nicht lesen"),
                other => Error::invalid(format!("Die Excel-Datei lässt sich nicht lesen: {other}")),
            })?;
            for (i, sheet) in read.workbook.sheets.iter().enumerate().take(MAX_SHEETS) {
                let mut g = Grid { name: sheet.name.clone(), ..Grid::default() };
                if only.is_none_or(|o| o == i) {
                    for (at, cell) in &sheet.cells {
                        put(&mut g, at.row, at.col, sheet_value(&cell.value), &mut budget)?;
                    }
                }
                out.push(g);
            }
        }
        "xlsx" | "xlsm" => {
            let doc = craft_xlsx::Document::read(bytes, &craft_xlsx::Limits::default())
                .map_err(|e| Error::invalid(format!("Die Excel-Datei lässt sich nicht lesen: {e}")))?;
            let date_1904 = doc.workbook.date_1904;
            for (i, entry) in doc.workbook.sheets.iter().enumerate().take(MAX_SHEETS) {
                let mut g = Grid { name: entry.name.clone(), ..Grid::default() };
                if only.is_none_or(|o| o == i)
                    && let Some(Some(ws)) = doc.worksheets.get(i)
                {
                    for row in &ws.rows {
                        for cell in &row.cells {
                            put(&mut g, cell.at.row, cell.at.col, sheet_value(&cell.value.resolve(&doc.shared_strings, date_1904)), &mut budget)?;
                        }
                    }
                }
                out.push(g);
            }
        }
        "ods" => {
            let opts = craft_ods::Options { only_sheet: only, ..craft_ods::Options::default() };
            let wb = craft_ods::read(bytes, &opts).map_err(|e| match e {
                craft_ods::Error::Encrypted => encrypted(),
                other => Error::invalid(format!("Die Tabelle lässt sich nicht lesen: {other}")),
            })?;
            for sheet in wb.sheets.iter().take(MAX_SHEETS) {
                let mut g = Grid { name: sheet.name.clone(), ..Grid::default() };
                for (at, cell) in &sheet.cells {
                    put(&mut g, at.row, at.col, cell.text.clone(), &mut budget)?;
                }
                out.push(g);
            }
        }
        _ => return Err(Error::unsupported(format!(".{ext} ist keine Tabelle, die AIS.ChatCraft lesen kann"))),
    }
    if out.is_empty() {
        return Err(Error::invalid("Die Tabelle hat keine Blätter"));
    }
    Ok(out)
}

/// One sheet as CSV (comma, UTF-8, quoted where needed).
pub fn csv(grid: &Grid) -> String {
    let rows = grid.cells.keys().map(|(r, _)| *r).max().map_or(0, |r| r + 1);
    let cols = grid.cells.keys().map(|(_, c)| *c).max().map_or(0, |c| c + 1);
    let mut out = String::new();
    for r in 0..rows {
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            if let Some(t) = grid.cells.get(&(r, c)) {
                craft_csv::push_field(&mut out, t, ',');
            }
        }
        out.push('\n');
    }
    out
}

fn md_cell(t: &str) -> String {
    t.replace('\\', "\\\\").replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// A whole workbook as Markdown: a heading and a table per sheet.
pub fn markdown(title: &str, grids: &[Grid]) -> String {
    let mut out = format!("# {title}\n");
    for g in grids {
        out.push_str(&format!("\n## {}\n\n", md_cell(&g.name)));
        let rows = g.cells.keys().map(|(r, _)| *r).max().map(|r| r + 1);
        let cols = g.cells.keys().map(|(_, c)| *c).max().map_or(0, |c| c + 1);
        let Some(rows) = rows else {
            out.push_str("(leer)\n");
            continue;
        };
        for r in 0..rows {
            let line: Vec<String> = (0..cols).map(|c| g.cells.get(&(r, c)).map(|t| md_cell(t)).unwrap_or_default()).collect();
            out.push_str(&format!("| {} |\n", line.join(" | ")));
            if r == 0 {
                out.push_str(&format!("|{}\n", " --- |".repeat(cols as usize)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xlsx_sheets_to_csv_and_markdown() {
        let bytes = crate::office::xlsx::workbook_xlsx(&serde_json::json!({"sheets": [
            {"name": "Noten", "columns": ["Name", "Note"], "rows": [["Anna", 2], ["Ben, Jr.", 1.5]]},
            {"name": "Liste", "rows": [["x"]]}
        ]}))
        .unwrap();
        let all = read("xlsx", &bytes, None).unwrap();
        assert_eq!(all.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Noten", "Liste"]);
        assert_eq!(csv(&all[0]), "Name,Note\nAnna,2\n\"Ben, Jr.\",1.5\n");
        let md = markdown("Klasse", &all);
        assert!(md.contains("## Liste") && md.contains("| Anna | 2 |"), "{md}");
        let one = read("xlsx", &bytes, Some(1)).unwrap();
        assert!(one[0].cells.is_empty() && one[1].cells.len() == 1);
        assert!(read("xlsx", b"kaputt", None).is_err());
        assert!(read("ods", b"kaputt", None).is_err());
        assert!(read("xls", b"kaputt", None).is_err());
    }

    #[test]
    fn xls_reads_through_the_fixture_writer() {
        let bytes = craft_xls::testing::workbook(&[("Noten", vec![vec!["Name".into()], vec!["3".into()]])]).unwrap();
        let all = read("xls", &bytes, None).unwrap();
        assert_eq!(csv(&all[0]), "Name\n3\n");
    }
}
