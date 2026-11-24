//! Functional tests of the attachment commands: real small files built in the test, converted
//! as they would be for AIS.chat and for other backends. The engine's command guard reads this
//! file together with `tests.rs`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{Engine, Options};

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("aiscraft-attach-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(d: &Path, name: &str, bytes: &[u8]) -> String {
    let p = d.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.display().to_string()
}

fn prepare(engine: &Engine, path: &str) -> Value {
    engine.call("attachment.prepare", json!({"path": path, "backend": "ais-chat"})).unwrap_or_else(|e| panic!("{path}: {}", e.message))
}

fn refused(engine: &Engine, path: &str) -> String {
    engine.call("attachment.prepare", json!({"path": path, "backend": "ais-chat"})).expect_err(path).message
}

fn eml() -> Vec<u8> {
    b"From: Ada <ada@schule.example>\r\nTo: Ben <ben@schule.example>\r\nSubject: =?utf-8?q?Elternabend_n=C3=A4chste_Woche?=\r\nDate: Mon, 23 Nov 2026 10:00:00 +0100\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=XX\r\n\r\n--XX\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBitte bringt die Listen mit.\r\n--XX\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\r\n%PDF-1.4\r\n--XX--\r\n".to_vec()
}

const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nSUMMARY:Elternabend 7b\r\nDTSTART:20261203T190000\r\nDTEND:20261203T203000\r\nLOCATION:Raum 12\\, Altbau\r\nDESCRIPTION:Themen: Klassenfahrt\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Erinnerung\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const VCF: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lehrerin\r\nN:Lehrerin;Ada;;;\r\nORG:Gesamtschule\r\nEMAIL;TYPE=work:ada@schule.example\r\nTEL;TYPE=cell:+49 30 123\r\nEND:VCARD\r\n";

fn epub() -> Vec<u8> {
    let chapter = craft_epub::xhtml("de", "Novelle", "", "", "<h1>Erstes Kapitel</h1><p>Es war einmal ein Schimmelreiter.</p>\n");
    let book = craft_epub::Book {
        metadata: craft_epub::Metadata {
            identifier: craft_epub::urn_uuid("Novelle"),
            title: "Novelle".into(),
            language: "de".into(),
            modified: "2026-10-13T09:30:00Z".into(),
            ..craft_epub::Metadata::default()
        },
        resources: vec![craft_epub::Resource::new("c1", "c1.xhtml", "application/xhtml+xml", chapter.into_bytes())],
        spine: vec!["c1".into()],
        ..craft_epub::Book::default()
    };
    craft_epub::write(&book).unwrap()
}

/// #45: what AIS.chat does not take is converted to Markdown, CSV or JPEG, with a note.
#[test]
fn attachment_prepare_converts_what_ais_chat_refuses() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let d = dir();
    let doc = craft_doc_binary::write(&craft_wordmodel::Document::from_text("Hallo Klasse 7b")).unwrap();
    let p = prepare(&engine, &write(&d, "Brief.doc", &doc));
    assert_eq!(p["name"], "Brief.md");
    assert_eq!(p["mime"], "text/markdown");
    assert_eq!(p["note"], "Wird als Markdown hochgeladen: Brief.md");
    assert!(p["preview"].as_str().unwrap().contains("Hallo Klasse 7b"));

    let rtf = craft_rtf::write(
        &craft_rtf::Document {
            blocks: vec![craft_rtf::Block::Para(craft_rtf::Para {
                runs: vec![craft_rtf::Run { text: "Arbeitsblatt Brüche".into(), fmt: craft_rtf::CharFmt::default() }],
                ..craft_rtf::Para::default()
            })],
            pictures_left_out: 0,
        },
        "Liberation Sans",
    );
    assert!(prepare(&engine, &write(&d, "Blatt.rtf", rtf.as_bytes()))["preview"].as_str().unwrap().contains("Arbeitsblatt Brüche"));

    let p = prepare(&engine, &write(&d, "Novelle.epub", &epub()));
    assert!(p["preview"].as_str().unwrap().contains("Schimmelreiter"), "{p}");

    let p = prepare(&engine, &write(&d, "Mail.eml", &eml()));
    let text = p["preview"].as_str().unwrap();
    assert!(text.contains("Elternabend nächste Woche") && text.contains("Bitte bringt die Listen mit.") && text.contains("Von: Ada"), "{text}");

    let p = prepare(&engine, &write(&d, "Termine.ics", ICS.as_bytes()));
    let text = p["preview"].as_str().unwrap();
    assert!(text.contains("## Elternabend 7b") && text.contains("03.12.2026 19:00") && text.contains("Raum 12, Altbau"), "{text}");
    assert!(!text.contains("Erinnerung"), "alarm is not an event: {text}");

    let p = prepare(&engine, &write(&d, "Kontakte.vcf", VCF.as_bytes()));
    let text = p["preview"].as_str().unwrap();
    assert!(text.contains("## Ada Lehrerin") && text.contains("ada@schule.example") && text.contains("Gesamtschule"), "{text}");

    let xls = craft_xls::testing::workbook(&[("Noten", vec![vec!["Name".into(), "Note".into()], vec!["Anna".into(), "2".into()]])]).unwrap();
    let p = prepare(&engine, &write(&d, "Noten.xls", &xls));
    assert_eq!(p["name"], "Noten.csv");
    assert_eq!(p["preview"], "Name,Note\nAnna,2\n");
    assert_eq!(p["range_kind"], "sheet");

    let p = prepare(&engine, &write(&d, "skript.py", b"print('hallo')\n"));
    assert_eq!(p["name"], "skript.py.txt");

    // Accepted types go as they are.
    let p = prepare(&engine, &write(&d, "Notizen.md", b"# Hallo"));
    assert_eq!(p["name"], "Notizen.md");
    assert!(p.get("note").is_none());

    // Clear German errors instead of a server failure; the chip shows them too.
    let zip = write(&d, "Archiv.zip", b"PK\x03\x04xx");
    assert!(refused(&engine, &zip).contains("AIS.chat nimmt .zip-Dateien nicht an"));
    assert!(engine.state().attachments[&zip].error.as_deref().unwrap().contains(".zip"));
    // The composer asks quietly: the error is in the preview, not a notice.
    let q = engine.call("attachment.prepare", json!({"path": zip, "backend": "ais-chat", "quiet": true})).unwrap();
    assert!(q["error"].as_str().unwrap().contains("Möglich sind"));
    assert!(refused(&engine, &write(&d, "Folien.ppt", b"kaputt")).contains("PowerPoint-Datei"));
    assert!(refused(&engine, &write(&d, "Alt.doc", b"\xd0\xcf\x11\xe0 kaputt")).contains("Word-Dokument"));
    assert!(refused(&engine, &write(&d, "Geheim.docx", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1 verschluesselt")).contains("Passwort"));
    assert!(refused(&engine, &write(&d, "Leer.ics", b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")).contains("kein Text"));
    assert!(refused(&engine, &write(&d, "Foto.heic", b"\0\0\0\x18ftypheic kaputt")).len() > 10);
    assert!(engine.call("attachment.prepare", json!({"path": d.join("fehlt.doc").display().to_string()})).is_err());

    // Other backends read the converted text inline; their pictures stay as they are.
    let p = engine.call("attachment.prepare", json!({"path": d.join("Brief.doc").display().to_string(), "backend": "nope"})).unwrap();
    assert_eq!(p["note"], "Als Text gelesen: Brief.md");

    assert_eq!(engine.call("attachment.remove", json!({"path": zip})).unwrap()["removed"], 1);
    assert!(engine.call("attachment.remove", json!({})).unwrap()["removed"].as_u64().unwrap() > 3);
    assert!(engine.state().attachments.is_empty());
    let _ = std::fs::remove_dir_all(d);
}

/// #50: photos lose location and camera data and are scaled to 1000 px; HEIC becomes JPEG.
#[test]
fn attachment_prepare_strips_photo_location() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let d = dir();
    let path = write(&d, "Klassenfahrt.jpg", &super::image::tests::jpeg_with_gps(1200, 900));
    let p = prepare(&engine, &path);
    assert_eq!(p["name"], "Klassenfahrt.jpg");
    assert_eq!(p["location_removed"], true);
    assert_eq!(p["note"], "Standortdaten entfernt · auf 1000 × 750 px verkleinert");
    assert!(engine.state().attachments[&path].location_removed);

    #[cfg(feature = "heif")]
    {
        let sample = |x: u32, y: u32| [u16::try_from(x * 7 % 256).unwrap(), u16::try_from(y * 5 % 256).unwrap(), 128];
        let heic = craft_heif::testdata::build(&craft_heif::testdata::Spec::new(32, 32, &sample));
        let p = prepare(&engine, &write(&d, "IMG_0001.HEIC", &heic));
        assert_eq!(p["name"], "IMG_0001.jpg", "{p}");
        assert_eq!(p["mime"], "image/jpeg");
        assert!(p["note"].as_str().unwrap().contains("als JPEG"), "{p}");
    }
    let _ = std::fs::remove_dir_all(d);
}

/// #47: only some pages of a PDF, one sheet as CSV, some slides as Markdown; strict ranges.
#[test]
fn attachment_set_range_pages_sheets_and_slides() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let d = dir();
    let md: String = (1..=5).map(|i| format!("# Kapitel {i}\n\n{}\n\n", format!("Zeile in Kapitel {i}.\n\n").repeat(45))).collect();
    let pdf = write(&d, "Skript.pdf", &crate::office::pdf::markdown_pdf("Skript", &md).unwrap());
    let whole = prepare(&engine, &pdf);
    assert_eq!(whole["range_kind"], "pages");
    let p = engine.call("attachment.set_range", json!({"path": pdf, "range": "2-3", "backend": "ais-chat"})).unwrap();
    assert_eq!(p["name"], "Skript (Seiten 2-3).pdf");
    let total = p["total"].as_u64().unwrap();
    assert!(total >= 4);
    assert!(p["note"].as_str().unwrap().starts_with(&format!("Nur Seiten 2-3 von {total}")));
    assert!(p["size"].as_u64().unwrap() < whole["size"].as_u64().unwrap());
    assert_eq!(engine.state().attachments[&pdf].range, "2-3");
    for (bad, msg) in [("1-999999", "nur"), ("abc", "verstehe"), ("0", "ab 1"), ("4-2", "kleineren")] {
        let e = engine.call("attachment.set_range", json!({"path": pdf, "range": bad, "backend": "ais-chat"})).unwrap_err();
        assert!(e.message.contains(msg), "{bad}: {}", e.message);
    }
    assert!(engine.call("attachment.set_range", json!({"path": pdf})).is_err(), "range is required");
    let back = engine.call("attachment.set_range", json!({"path": pdf, "range": "", "backend": "ais-chat"})).unwrap();
    assert_eq!(back["name"], "Skript.pdf");

    let xlsx =
        crate::office::xlsx::workbook_xlsx(&json!({"sheets": [{"name": "Noten", "rows": [["Anna", 2]]}, {"name": "Liste", "rows": [["Ben"], ["Cem"]]}]}))
            .unwrap();
    let book = write(&d, "Klasse.xlsx", &xlsx);
    let p = engine.call("attachment.set_range", json!({"path": book, "range": "liste", "backend": "ais-chat"})).unwrap();
    assert_eq!(p["name"], "Klasse - Liste.csv");
    assert_eq!(p["preview"], "Ben\nCem\n");
    assert_eq!(p["sheets"], json!(["Noten", "Liste"]));
    assert!(engine.call("attachment.set_range", json!({"path": book, "range": "Fehlt", "backend": "ais-chat"})).unwrap_err().message.contains("Noten, Liste"));

    let pptx = crate::office::pptx::slides_pptx(
        &json!({"slides": [{"title": "Eins", "bullets": ["A"]}, {"title": "Zwei", "bullets": ["B"]}, {"title": "Drei", "bullets": ["C"]}]}),
    )
    .unwrap();
    let deck = write(&d, "Folien.pptx", &pptx);
    let p = engine.call("attachment.set_range", json!({"path": deck, "range": "2-3", "backend": "ais-chat"})).unwrap();
    assert_eq!(p["name"], "Folien (Folien 2-3).md");
    let text = p["preview"].as_str().unwrap();
    assert!(text.contains("Zwei") && text.contains("Drei") && !text.contains("Eins"), "{text}");
    assert!(engine.call("attachment.set_range", json!({"path": deck, "range": "4", "backend": "ais-chat"})).is_err());

    let txt = write(&d, "a.txt", b"x");
    assert!(engine.call("attachment.set_range", json!({"path": txt, "range": "1"})).unwrap_err().message.contains("keine Seiten"));
    // A corrupt PDF with a range: a German error, no panic.
    let broken = write(&d, "kaputt.pdf", b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 9 0 R >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF");
    assert!(engine.call("attachment.set_range", json!({"path": broken, "range": "1"})).is_err());
    let _ = std::fs::remove_dir_all(d);
}

/// Sending to AIS.chat uploads the prepared files: Markdown for .doc, the photo without
/// location, only the chosen page; a sixth picture in one chat is refused before upload.
#[test]
fn chat_send_uploads_prepared_files_to_ais_chat() {
    let (engine, posted) = crate::tests::ais_engine();
    let d = dir();
    let doc = write(&d, "Brief.doc", &craft_doc_binary::write(&craft_wordmodel::Document::from_text("Liebe Eltern")).unwrap());
    let photo = write(&d, "Tafel.jpg", &super::image::tests::jpeg_with_gps(1400, 700));
    let md: String = (1..=4).map(|i| format!("# Teil {i}\n\n{}\n\n", "Text.\n\n".repeat(45))).collect();
    let pdf = write(&d, "Skript.pdf", &crate::office::pdf::markdown_pdf("Skript", &md).unwrap());
    engine.call("chat.send", json!({"text": "Bitte zusammenfassen", "files": [doc, photo, {"path": pdf, "range": "1"}]})).unwrap();
    let uploads: Vec<String> = posted.lock().unwrap().iter().filter(|r| r.starts_with("POST /api/v1/files")).cloned().collect();
    assert_eq!(uploads.len(), 3, "{uploads:?}");
    assert!(uploads[0].contains("filename=\"Brief.md\"") && uploads[0].contains("Liebe Eltern"));
    assert!(uploads[1].contains("filename=\"Tafel.jpg\"") && uploads[1].contains("image/jpeg"));
    assert!(uploads[2].contains("filename=\"Skript (Seiten 1).pdf\""));
    let s = engine.state();
    let sent = &s.messages[0].attachments;
    assert_eq!(sent.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Brief.md", "Tafel.jpg", "Skript (Seiten 1).pdf"]);
    let kept = std::fs::read(sent[1].local_path.as_deref().unwrap()).unwrap();
    assert!(craft_metadata::embedded(&kept).exif.is_none(), "the photo left without EXIF");
    assert_eq!(craft_image_codecs::probe_size(&kept).unwrap(), (1000, 500));
    assert!(s.attachments.is_empty(), "previews are forgotten after sending");

    // Five pictures per chat at most (one is in the chat already).
    let many: Vec<String> = (0..5).map(|i| write(&d, &format!("b{i}.png"), &super::image::tests::jpeg_with_gps(8, 8))).collect();
    let before = posted.lock().unwrap().len();
    let err = engine.call("chat.send", json!({"text": "Noch mehr", "files": many})).unwrap_err();
    assert!(err.message.contains("höchstens 5 Bilder"), "{}", err.message);
    assert_eq!(posted.lock().unwrap().len(), before, "nothing uploaded");
    let _ = std::fs::remove_dir_all(d);
}
