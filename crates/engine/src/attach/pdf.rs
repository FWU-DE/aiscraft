//! A PDF with only the chosen pages, built on craft-pdf's object layer (`cos`): the page tree
//! is replaced by one flat list of the kept pages, then a full rewrite keeps only what the new
//! tree reaches. Nothing outside the kept pages may stay in the file, so links, outlines and
//! forms that could pull other pages back in are left out, and the result is checked.

use std::collections::HashSet;
use std::sync::Arc;

use aiscraft_core::{Error, Result};
use craft_pdf::cos::{CosError, Dict, Document, ObjRef, Object, SaveOptions, write_full};

/// Page trees deeper than this, or with more leaves, are refused.
const MAX_DEPTH: usize = 64;
pub const MAX_PAGES: usize = 20_000;
/// Attributes a page inherits from its parents (ISO 32000-2 §7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

fn cos_error(e: CosError) -> Error {
    match e {
        CosError::NeedsPassword | CosError::WrongPassword => {
            Error::unsupported("Die PDF-Datei ist mit einem Passwort geschützt. Entferne den Schutz zuerst (z. B. „Drucken als PDF“).")
        }
        CosError::NotPdf => Error::invalid("Das ist keine lesbare PDF-Datei"),
        other => Error::invalid(format!("Die PDF-Datei lässt sich nicht lesen: {other}")),
    }
}

fn open(bytes: &[u8]) -> Result<Document> {
    Document::open(Arc::new(bytes.to_vec())).map_err(cos_error)
}

struct Leaf {
    page: ObjRef,
    inherited: Vec<(&'static [u8], Object)>,
}

/// The leaves of the page tree in order, with what each inherits.
fn leaves(doc: &Document) -> Result<Vec<Leaf>> {
    let root = doc.root().ok_or_else(|| Error::invalid("Die PDF-Datei hat keinen Katalog"))?;
    let catalog = doc.get(root);
    let pages = catalog.as_dict().and_then(|d| d.get(b"Pages")).and_then(Object::as_ref).ok_or_else(|| Error::invalid("Die PDF-Datei hat keine Seiten"))?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(doc, pages, Vec::new(), 0, &mut seen, &mut out)?;
    Ok(out)
}

fn walk(doc: &Document, node: ObjRef, inherited: Vec<(&'static [u8], Object)>, depth: usize, seen: &mut HashSet<u32>, out: &mut Vec<Leaf>) -> Result<()> {
    if depth > MAX_DEPTH || !seen.insert(node.num) {
        return Err(Error::invalid("Der Seitenbaum der PDF-Datei ist beschädigt"));
    }
    let obj = doc.get(node);
    let Some(dict) = obj.as_dict() else { return Ok(()) };
    let mut mine = inherited;
    for key in INHERITED {
        if let Some(v) = dict.get(key) {
            mine.retain(|(k, _)| *k != key);
            mine.push((key, v.clone()));
        }
    }
    let is_pages = dict.get(b"Type").and_then(Object::as_name) == Some(b"Pages".as_slice()) || dict.get(b"Kids").is_some();
    if !is_pages {
        if out.len() >= MAX_PAGES {
            return Err(Error::invalid(format!("Die PDF-Datei hat mehr als {MAX_PAGES} Seiten")));
        }
        out.push(Leaf { page: node, inherited: mine });
        return Ok(());
    }
    let kids = dict.get(b"Kids").map(|k| doc.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap_or_default();
    for kid in kids {
        if let Some(r) = kid.as_ref() {
            walk(doc, r, mine.clone(), depth + 1, seen, out)?;
        }
    }
    Ok(())
}

/// How many pages the PDF has.
pub fn page_count(bytes: &[u8]) -> Result<usize> {
    Ok(leaves(&open(bytes)?)?.len())
}

/// Count objects of type /Page in a file (after a rewrite: the pages that are really in it).
fn pages_in(bytes: &[u8]) -> Result<usize> {
    let doc = open(bytes)?;
    Ok(doc.scan_objects().filter(|(_, o)| o.as_dict().and_then(|d| d.get(b"Type")).and_then(Object::as_name) == Some(b"Page".as_slice())).count())
}

/// A new PDF with only `numbers` (1-based, ascending) of `bytes`.
pub fn subset(bytes: &[u8], numbers: &[usize]) -> Result<Vec<u8>> {
    let mut doc = open(bytes)?;
    let all = leaves(&doc)?;
    let keep: Vec<&Leaf> = numbers.iter().filter_map(|n| n.checked_sub(1).and_then(|i| all.get(i))).collect();
    if keep.is_empty() || keep.len() != numbers.len() {
        return Err(Error::invalid("Diese Seiten gibt es in der PDF-Datei nicht"));
    }
    let root = doc.root().ok_or_else(|| Error::invalid("Die PDF-Datei hat keinen Katalog"))?;
    let tree = doc.add(Object::Dict(Dict::new()));
    for leaf in &keep {
        let mut page = doc.dict(&Object::Ref(leaf.page)).ok_or_else(|| Error::invalid("Eine Seite der PDF-Datei ist beschädigt"))?;
        for (k, v) in &leaf.inherited {
            if !page.contains(k) {
                page.set(k.to_vec(), v.clone());
            }
        }
        page.set(b"Parent".to_vec(), Object::Ref(tree));
        // Links and form fields can point at pages that are left out.
        if let Some(annots) = page.get(b"Annots").map(|a| doc.resolve(a)).and_then(|a| a.as_array().cloned()) {
            let kept: Vec<Object> = annots
                .into_iter()
                .filter(|a| {
                    let d = doc.dict(a);
                    let subtype = d.as_ref().and_then(|d| d.get(b"Subtype")).and_then(Object::as_name).map(<[u8]>::to_vec);
                    !matches!(subtype.as_deref(), Some(b"Link" | b"Widget" | b"Popup"))
                })
                .collect();
            if kept.is_empty() {
                page.remove(b"Annots");
            } else {
                page.set(b"Annots".to_vec(), Object::Array(kept));
            }
        }
        page.remove(b"B"); // article beads
        doc.set(leaf.page, Object::Dict(page));
    }
    let mut pages = Dict::new();
    pages.set(b"Type".to_vec(), Object::name("Pages"));
    pages.set(b"Kids".to_vec(), Object::Array(keep.iter().map(|l| Object::Ref(l.page)).collect()));
    pages.set(b"Count".to_vec(), Object::Int(i64::try_from(keep.len()).unwrap_or(0)));
    doc.set(tree, Object::Dict(pages));
    let mut catalog = doc.dict(&Object::Ref(root)).ok_or_else(|| Error::invalid("Die PDF-Datei hat keinen Katalog"))?;
    catalog.set(b"Pages".to_vec(), Object::Ref(tree));
    for key in [b"Outlines".as_slice(), b"Dests", b"Names", b"AcroForm", b"OpenAction", b"StructTreeRoot", b"PageLabels", b"Threads", b"PieceInfo", b"MarkInfo"]
    {
        catalog.remove(key);
    }
    doc.set(root, Object::Dict(catalog));
    let opts = SaveOptions { object_streams: false, id_entropy: Some([7; 16]), ..SaveOptions::default() };
    let out = write_full(&doc, &opts).map_err(cos_error)?;
    // Check: exactly the kept pages are in the file, nothing else came along.
    if pages_in(&out)? != keep.len() {
        return Err(Error::invalid("Die Teil-PDF ließ sich nicht sauber erstellen. Sende die ganze Datei oder speichere die Seiten selbst als PDF."));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_chosen_pages() {
        // Our own writer breaks pages by length: many short paragraphs make several pages.
        let md: String = (1..=6).map(|i| format!("# Kapitel {i}\n\n{}\n\n", format!("Zeile auf Seite {i}.\n\n").repeat(45))).collect();
        let pdf = crate::office::pdf::markdown_pdf("Test", &md).unwrap();
        let total = page_count(&pdf).unwrap();
        assert!(total >= 4, "{total}");
        let out = subset(&pdf, &[2, 3]).unwrap();
        assert_eq!(page_count(&out).unwrap(), 2);
        assert_eq!(pages_in(&out).unwrap(), 2);
        assert!(out.len() < pdf.len());
        assert!(subset(&pdf, &[total + 1]).is_err());
        assert!(subset(b"%PDF-1.4 kaputt", &[1]).is_err());
        assert!(page_count(b"keine pdf").is_err());
    }
}
