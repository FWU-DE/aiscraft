#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use yrs::Any;

use crate::{Board, Element, Kind, arrow_endpoints, parse_outline};

fn note(x: f64, y: f64, text: &str) -> Element {
    Element::new(Kind::Note, x, y).with_text(text)
}

fn sync(a: &mut Board, b: &mut Board) {
    let to_b = a.encode_diff(&b.state_vector()).unwrap();
    let to_a = b.encode_diff(&a.state_vector()).unwrap();
    b.apply_update(&to_b).unwrap();
    a.apply_update(&to_a).unwrap();
}

#[test]
fn round_trip_state_and_json() {
    let mut b = Board::new();
    b.set_title("Biologie");
    let n = b.add(note(10.0, 20.0, "Zelle"));
    let r = b.add(Element::new(Kind::Rect, 300.0, 40.0).with_text("Kern").with_color("blue"));
    b.add(Element::connector(&b.element(&n).unwrap(), &b.element(&r).unwrap()));
    b.add(Element::path(Kind::Freehand, &[(0.0, 0.0), (5.0, 8.0), (12.0, 3.0)]));
    let state = b.encode_state();
    let c = Board::from_state(&state).unwrap();
    assert_eq!(c.elements(), b.elements());
    assert_eq!(c.title(), "Biologie");
    let json: serde_json::Value = serde_json::from_str(&c.to_json()).unwrap();
    assert_eq!(json["elements"].as_array().unwrap().len(), 4);
    assert_eq!(json["title"], "Biologie");
    let parsed: crate::BoardJson = serde_json::from_str(&c.to_json()).unwrap();
    assert_eq!(parsed.elements, c.elements());
    assert!(c.to_raw_json().contains("Zelle"));
}

#[test]
fn freehand_points_are_relative() {
    let e = Element::path(Kind::Freehand, &[(100.0, 50.0), (110.0, 70.0), (90.0, 60.0)]);
    assert_eq!((e.x, e.y, e.w, e.h), (90.0, 50.0, 20.0, 20.0));
    let pts: Vec<_> = e.world_points().collect();
    assert_eq!(pts, vec![(100.0, 50.0), (110.0, 70.0), (90.0, 60.0)]);
}

#[test]
fn concurrent_edits_converge() {
    let mut a = Board::new();
    let e1 = a.add(note(0.0, 0.0, "eins"));
    let e3 = a.add(note(0.0, 200.0, "drei"));
    let mut b = Board::from_state(&a.encode_state()).unwrap();
    assert_eq!(a.elements(), b.elements());

    // Concurrent, without exchanging anything in between.
    a.move_to(&e1, 50.0, 60.0);
    b.set_text(&e1, "eins (geändert)");
    let e2 = a.add(note(400.0, 0.0, "zwei"));
    b.delete(std::slice::from_ref(&e3));
    b.set_color(std::slice::from_ref(&e1), "green");
    a.set_color(std::slice::from_ref(&e1), "blue");
    a.set_title("A");
    b.set_title("B");

    sync(&mut a, &mut b);
    assert_eq!(a.elements(), b.elements());
    assert_eq!(a.title(), b.title());
    let one = a.element(&e1).unwrap();
    assert_eq!((one.x, one.y), (50.0, 60.0));
    assert_eq!(one.text, "eins (geändert)");
    assert!(one.color == "blue" || one.color == "green");
    assert!(a.element(&e2).is_some());
    assert!(a.element(&e3).is_none());

    // Updates applied in a different order and twice still converge.
    let mut c = Board::new();
    let u = a.encode_state();
    c.apply_update(&u).unwrap();
    c.apply_update(&u).unwrap();
    assert_eq!(c.elements(), a.elements());
}

#[test]
fn ids_do_not_collide_between_peers() {
    let mut a = Board::new();
    let mut b = Board::from_state(&a.encode_state()).unwrap();
    let ia: Vec<String> = (0..20).map(|i| a.add(note(i as f64, 0.0, "a"))).collect();
    let ib: Vec<String> = (0..20).map(|i| b.add(note(i as f64, 0.0, "b"))).collect();
    sync(&mut a, &mut b);
    assert_eq!(a.len(), 40);
    assert!(ia.iter().all(|i| !ib.contains(i)));
}

#[test]
fn undo_redo_local_only() {
    let mut a = Board::new();
    let id = a.add(note(0.0, 0.0, "x"));
    a.checkpoint();
    a.move_to(&id, 10.0, 0.0);
    a.move_to(&id, 20.0, 0.0);
    a.move_to(&id, 30.0, 0.0);
    a.checkpoint();
    a.set_text(&id, "neu");
    assert!(a.can_undo());

    assert!(a.undo());
    assert_eq!(a.element(&id).unwrap().text, "x");
    assert!(a.undo());
    // The whole drag is one step.
    assert_eq!(a.element(&id).unwrap().x, 0.0);
    assert!(a.undo());
    assert!(a.element(&id).is_none());
    assert!(a.can_redo());
    assert!(a.redo());
    assert!(a.element(&id).is_some());
    assert!(a.redo());
    assert_eq!(a.element(&id).unwrap().x, 30.0);

    // Remote changes are not undone locally.
    let mut b = Board::new();
    b.apply_update(&a.encode_state()).unwrap();
    assert!(!b.can_undo());
    assert!(!b.undo());
    assert_eq!(b.len(), 1);
    let mine = b.add(note(5.0, 5.0, "b"));
    assert!(b.undo());
    assert!(b.element(&mine).is_none());
    assert!(b.element(&id).is_some());
}

#[test]
fn delete_keeps_bound_arrows_in_place() {
    let mut b = Board::new();
    let n1 = b.add(note(0.0, 0.0, "a"));
    let n2 = b.add(note(500.0, 0.0, "b"));
    let arrow = b.add(Element::connector(&b.element(&n1).unwrap(), &b.element(&n2).unwrap()));
    let before = {
        let els = b.elements();
        let map: HashMap<&str, &Element> = els.iter().map(|e| (e.id.as_str(), e)).collect();
        arrow_endpoints(map[arrow.as_str()], |id| map.get(id).copied())
    };
    b.delete(std::slice::from_ref(&n2));
    let a = b.element(&arrow).unwrap();
    assert_eq!(a.to, None);
    assert_eq!(a.from.as_deref(), Some(n1.as_str()));
    let els = b.elements();
    let after = arrow_endpoints(&a, |id| els.iter().find(|e| e.id == id));
    assert!((after.1.0 - before.1.0).abs() < 1e-6 && (after.1.1 - before.1.1).abs() < 1e-6);
    b.undo();
    assert_eq!(b.element(&arrow).unwrap().to.as_deref(), Some(n2.as_str()));
}

#[test]
fn arrows_follow_bound_elements() {
    let mut b = Board::new();
    let n1 = b.add(note(0.0, 0.0, "a").with_size(100.0, 100.0));
    let n2 = b.add(note(400.0, 0.0, "b").with_size(100.0, 100.0));
    let arrow = b.add(Element::connector(&b.element(&n1).unwrap(), &b.element(&n2).unwrap()));
    b.move_to(&n2, 400.0, 300.0);
    let els = b.elements();
    let a = els.iter().find(|e| e.id == arrow).unwrap();
    let (_, end) = arrow_endpoints(a, |id| els.iter().find(|e| e.id == id));
    // End sits on the moved note's outline (a few units outside).
    assert!(end.1 > 290.0 && end.1 < 400.0, "{end:?}");
}

#[test]
fn z_order_and_resize() {
    let mut b = Board::new();
    let a = b.add(note(0.0, 0.0, "a"));
    let c = b.add(note(0.0, 0.0, "c"));
    assert_eq!(b.elements().last().unwrap().id, c);
    b.bring_to_front(std::slice::from_ref(&a));
    assert_eq!(b.elements().last().unwrap().id, a);
    b.send_to_back(std::slice::from_ref(&a));
    assert_eq!(b.elements().first().unwrap().id, a);

    let f = b.add(Element::path(Kind::Freehand, &[(0.0, 0.0), (10.0, 20.0)]));
    b.resize(&f, 0.0, 0.0, 20.0, 10.0);
    let e = b.element(&f).unwrap();
    assert_eq!(e.points, vec![0.0, 0.0, 20.0, 10.0]);
    // A flat line keeps h = 0 instead of dividing by zero.
    let l = b.add(Element::segment(Kind::Line, (0.0, 0.0), (10.0, 0.0)));
    b.resize(&l, 0.0, 0.0, 30.0, 50.0);
    let e = b.element(&l).unwrap();
    assert_eq!(e.points, vec![0.0, 0.0, 30.0, 0.0]);
    assert!(e.points.iter().all(|p| p.is_finite()));
    assert!(b.update(&Element::new(Kind::Note, 0.0, 0.0)).is_err());
}

#[test]
fn malformed_crdt_data_is_skipped_or_clamped() {
    let mut b = Board::new();
    b.raw_insert("nan", vec![("kind", Any::from("note")), ("x", Any::from(f64::NAN)), ("y", Any::from(f64::INFINITY)), ("w", Any::from(-5.0))]);
    b.raw_insert(
        "huge",
        vec![
            ("kind", Any::from("rect")),
            ("x", Any::from(1e300)),
            ("y", Any::from(-1e300)),
            ("w", Any::from(1e300)),
            ("h", Any::from(f64::MAX)),
            ("z", Any::from(f64::NAN)),
            ("text", Any::from("x".repeat(100_000))),
            ("color", Any::from("c".repeat(10_000))),
        ],
    );
    b.raw_insert("badkind", vec![("kind", Any::from(5.0)), ("x", Any::from(1.0))]);
    b.raw_insert("unknown", vec![("kind", Any::from("hexagon"))]);
    b.raw_insert("nokind", vec![("x", Any::from(1.0))]);
    b.raw_insert(
        "pts",
        vec![
            ("kind", Any::from("freehand")),
            ("points", Any::Array(Arc::from(vec![Any::from(1.0), Any::from("a"), Any::from(f64::NAN), Any::Null, Any::from(1e300)]))),
            ("from", Any::from(3.0)),
        ],
    );
    b.raw_insert("strpts", vec![("kind", Any::from("line")), ("points", Any::from("1,2,3")), ("text", Any::Bool(true))]);
    b.raw_insert("bigpts", vec![("kind", Any::from("freehand")), ("points", Any::Array((0..100_000).map(|i| Any::from(i as f64)).collect::<Vec<_>>().into()))]);
    b.raw_insert("arrow", vec![("kind", Any::from("arrow")), ("from", Any::from("does-not-exist")), ("to", Any::from("arrow"))]);
    b.raw_insert_any("plainmap", Any::Map(Arc::new(HashMap::from([("kind".to_string(), Any::from("note")), ("text".to_string(), Any::from("ok"))]))));
    b.raw_insert_any("string", Any::from("kein Element"));
    b.raw_insert_any("number", Any::from(1.0));
    b.raw_insert(&"i".repeat(10_000), vec![("kind", Any::from("note"))]);

    let els = b.elements();
    let ids: Vec<&str> = els.iter().map(|e| e.id.as_str()).collect();
    for skipped in ["badkind", "unknown", "nokind", "string", "number"] {
        assert!(!ids.contains(&skipped), "{skipped}");
    }
    for e in &els {
        for v in [e.x, e.y, e.w, e.h, e.z] {
            assert!(v.is_finite() && v.abs() <= 1e12, "{} {v}", e.id);
        }
        assert!(e.w >= 0.0 && e.h >= 0.0);
        assert!(e.text.chars().count() <= crate::MAX_TEXT);
        assert!(e.points.len() <= crate::MAX_POINTS && e.points.len() % 2 == 0);
        assert!(e.points.iter().all(|p| p.is_finite()));
        assert!(e.color.len() <= crate::MAX_ID);
    }
    let pts = els.iter().find(|e| e.id == "pts").unwrap();
    assert_eq!(pts.from, None);
    assert_eq!(pts.points.len(), 4);
    assert!(els.iter().any(|e| e.id == "plainmap" && e.text == "ok"));

    // Every API keeps working on top of the junk.
    let arrow = els.iter().find(|e| e.id == "arrow").unwrap();
    let _ = arrow_endpoints(arrow, |id| els.iter().find(|e| e.id == id));
    b.move_by(&ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(), 1e300, f64::NAN);
    b.set_text("plainmap", "neu");
    assert_eq!(b.element("plainmap").unwrap().text, "neu");
    b.resize("huge", f64::NAN, 1.0, f64::INFINITY, -1.0);
    let _ = b.summary_text();
    let _ = b.to_json();
    b.delete(&["nan".to_string(), "missing".to_string()]);
    let copy = Board::from_state(&b.encode_state()).unwrap();
    assert_eq!(copy.elements(), b.elements());
}

#[test]
fn hostile_updates_do_not_panic() {
    let mut b = Board::new();
    for bad in [&b""[..], b"\x00", b"\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff", b"garbage that is not yjs"] {
        let _ = b.apply_update(bad);
        let _ = b.encode_diff(bad);
        let _ = Board::from_state(bad);
    }
    let mut src = Board::from_outline("- Wurzel\n  - Ast eins\n  - Ast zwei\n    - Blatt");
    src.add(Element::path(Kind::Freehand, &[(0.0, 0.0), (3.0, 4.0)]));
    let good = src.encode_state();
    // Truncations and byte flips of a valid update (deterministic pseudo-random).
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for cut in (0..good.len()).step_by(3) {
        let mut t = Board::new();
        let _ = t.apply_update(&good[..cut]);
        let _ = t.elements();
    }
    for _ in 0..300 {
        let mut data = good.clone();
        for _ in 0..(1 + next() % 4) {
            let i = (next() as usize) % data.len();
            data[i] = next() as u8;
        }
        let mut t = Board::new();
        let _ = t.apply_update(&data);
        let _ = t.elements();
        let _ = t.summary_text();
    }
    let too_big = vec![0u8; crate::MAX_UPDATE_BYTES + 1];
    assert!(b.apply_update(&too_big).is_err());
}

#[test]
fn outline_parsing() {
    let md = "# Photosynthese\n\n## Was passiert?\n- Blätter nehmen **CO₂** auf\n- Wurzeln liefern Wasser\n  - über die [Leitbündel](https://example.org)\n## Produkte\n1. Traubenzucker\n2. Sauerstoff\n\n```\n- kein Punkt\n```\n";
    let root = parse_outline(md);
    assert_eq!(root.text, "Photosynthese");
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].text, "Was passiert?");
    assert_eq!(root.children[0].children.len(), 2);
    assert_eq!(root.children[0].children[0].text, "Blätter nehmen CO₂ auf");
    assert_eq!(root.children[0].children[1].children[0].text, "über die Leitbündel");
    assert_eq!(root.children[1].children.len(), 2);
    assert_eq!(root.count(), 8);

    let list = parse_outline("* A\n* B\n    * B1\n    * B2\n* C");
    assert_eq!(list.text, "Übersicht");
    assert_eq!(list.children.len(), 3);
    assert_eq!(list.children[1].children.len(), 2);

    let plain = parse_outline("Erste Zeile\nZweite Zeile\nDritte");
    assert_eq!(plain.text, "Erste Zeile");
    assert_eq!(plain.children.len(), 2);

    assert_eq!(parse_outline("").text, "Übersicht");
    let deep: String = (0..50).map(|i| format!("{}- x{i}\n", "  ".repeat(i))).collect();
    let _ = parse_outline(&deep);
    let wide: String = (0..5000).map(|i| format!("- {i}\n")).collect();
    assert!(parse_outline(&wide).count() <= 301);
}

fn overlaps(a: &Element, b: &Element) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

#[test]
fn outline_mind_map_has_no_overlaps() {
    let md = "- Klassenfahrt\n  - Planung\n    - Termin\n    - Kosten\n    - Elternbrief\n  - Programm\n    - Wanderung\n    - Museum\n  - Packliste\n    - Kleidung\n    - Proviant mit einem längeren Text, der umbricht und mehr Höhe braucht\n  - Notfall\n";
    let b = Board::from_outline(md);
    assert_eq!(b.title(), "Klassenfahrt");
    assert!(!b.can_undo());
    let els = b.elements();
    let notes: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Note).collect();
    let arrows: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Arrow).collect();
    assert_eq!(notes.len(), 12);
    assert_eq!(arrows.len(), 11);
    for (i, a) in notes.iter().enumerate() {
        for c in notes.iter().skip(i + 1) {
            assert!(!overlaps(a, c), "{} overlaps {}", a.text, c.text);
        }
    }
    // Every arrow is bound at both ends to existing notes.
    for a in &arrows {
        assert!(notes.iter().any(|n| Some(&n.id) == a.from.as_ref()));
        assert!(notes.iter().any(|n| Some(&n.id) == a.to.as_ref()));
    }
    // Root in the centre, branches on both sides.
    let root = notes.iter().find(|n| n.text == "Klassenfahrt").unwrap();
    assert!((root.center().0).abs() < 1.0 && (root.center().1).abs() < 1.0);
    assert!(notes.iter().any(|n| n.x > root.x + root.w));
    assert!(notes.iter().any(|n| n.x + n.w < root.x));

    // Adding a second map to an existing board keeps ids unique.
    let mut b2 = Board::from_outline(md);
    let ids = b2.add_outline(md, 2000.0, 0.0);
    assert_eq!(ids.len(), 23);
    assert_eq!(b2.len(), 46);
    assert!(b2.can_undo());
}

#[test]
fn summary_groups_by_frame() {
    let mut b = Board::new();
    b.set_title("Elternabend");
    b.add(Element::new(Kind::Frame, 0.0, 0.0).with_size(600.0, 400.0).with_text("Themen"));
    let a = b.add(note(20.0, 20.0, "Klassenfahrt"));
    let c = b.add(note(300.0, 20.0, "Handyregeln"));
    b.add(note(1000.0, 0.0, "Kuchenbuffet"));
    b.add(Element::path(Kind::Freehand, &[(0.0, 0.0), (1.0, 1.0)]));
    b.add(Element::connector(&b.element(&a).unwrap(), &b.element(&c).unwrap()));
    let s = b.summary_text();
    assert!(s.starts_with("Whiteboard: Elternabend"), "{s}");
    let themen = s.find("## Rahmen „Themen“").unwrap();
    let outside = s.find("## Außerhalb von Rahmen").unwrap();
    let k = s.find("Notiz: Klassenfahrt").unwrap();
    let kb = s.find("Notiz: Kuchenbuffet").unwrap();
    assert!(themen < k && k < outside && outside < kb, "{s}");
    assert!(s.contains("- Klassenfahrt → Handyregeln"), "{s}");
    assert!(s.contains("1× Zeichnung"), "{s}");
    assert!(Board::new().summary_text().contains("ohne Titel"));
}

// ----- widget (headless, no rendering) --------------------------------------------------------

struct App {
    board: Board,
    view: crate::ViewState,
    style: crate::BoardStyle,
}

fn harness(board: Board) -> egui_kittest::Harness<'static, App> {
    let app = App { board, view: crate::ViewState::default(), style: crate::BoardStyle::light() };
    egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut App| {
            crate::BoardView::new(&mut app.board, &mut app.view, &app.style).show(ui);
        },
        app,
    )
}

fn drag(h: &mut egui_kittest::Harness<'static, App>, from: egui::Pos2, to: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
}

fn key(h: &mut egui_kittest::Harness<'static, App>, key: egui::Key, modifiers: egui::Modifiers) {
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
}

#[test]
fn widget_draws_moves_and_undoes() {
    let mut h = harness(Board::new());
    h.run_steps(2);
    // Hover the canvas so shortcuts reach it, then pick the rectangle tool with R.
    h.event(egui::Event::PointerMoved(egui::pos2(500.0, 400.0)));
    h.step();
    key(&mut h, egui::Key::R, egui::Modifiers::NONE);
    assert_eq!(h.state().view.tool, crate::Tool::Rect);
    drag(&mut h, egui::pos2(300.0, 300.0), egui::pos2(500.0, 420.0));
    let els = h.state().board.elements();
    assert_eq!(els.len(), 1, "{els:?}");
    assert_eq!(els[0].kind, Kind::Rect);
    assert!((els[0].w - 200.0).abs() < 1.0 && (els[0].h - 120.0).abs() < 1.0, "{:?}", els[0]);
    assert_eq!(h.state().view.tool, crate::Tool::Select);
    let (x0, y0) = (els[0].x, els[0].y);

    // Drag it with the select tool.
    drag(&mut h, egui::pos2(400.0, 360.0), egui::pos2(460.0, 400.0));
    let e = &h.state().board.elements()[0];
    assert!((e.x - x0 - 60.0).abs() < 1.0 && (e.y - y0 - 40.0).abs() < 1.0, "{e:?}");

    // Duplicate, then undo twice: back to one rectangle at the original place.
    key(&mut h, egui::Key::D, egui::Modifiers::COMMAND);
    assert_eq!(h.state().board.len(), 2);
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(h.state().board.len(), 1);
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    let e = &h.state().board.elements()[0];
    assert!((e.x - x0).abs() < 1.0, "{e:?}");
    key(&mut h, egui::Key::Z, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
    let e = &h.state().board.elements()[0];
    assert!((e.x - x0 - 60.0).abs() < 1.0, "{e:?}");

    // Undo dropped the copy from the selection; click the rectangle, then delete it.
    assert!(h.state().view.selection.is_empty());
    let at = egui::pos2(460.0, 400.0);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.step();
    assert_eq!(h.state().view.selection.len(), 1);
    key(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(h.state().board.is_empty());
}

#[test]
fn widget_edits_note_text_in_place() {
    let mut h = harness(Board::new());
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(egui::pos2(500.0, 400.0)));
    h.step();
    key(&mut h, egui::Key::N, egui::Modifiers::NONE);
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(500.0, 400.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(500.0, 400.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert_eq!(h.state().board.len(), 1);
    assert!(h.state().view.editing().is_some());
    h.event(egui::Event::Text("Hallo Welt".into()));
    h.run_steps(2);
    key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().view.editing().is_none());
    assert_eq!(h.state().board.elements()[0].text, "Hallo Welt");
}

#[test]
fn widget_handles_many_elements_and_junk() {
    let mut b = Board::new();
    let els: Vec<Element> = (0..1000)
        .map(|i| {
            let (x, y) = ((i % 40) as f64 * 220.0, (i / 40) as f64 * 160.0);
            match i % 5 {
                0 => Element::new(Kind::Rect, x, y),
                1 => Element::path(Kind::Freehand, &[(x, y), (x + 30.0, y + 50.0), (x + 90.0, y + 10.0)]),
                _ => note(x, y, &format!("Notiz {i} mit etwas Text, der umbricht")),
            }
        })
        .collect();
    b.add_many(els);
    b.raw_insert("junk", vec![("kind", Any::from("ellipse")), ("x", Any::from(f64::NAN)), ("w", Any::from(1e300))]);
    let mut h = harness(b);
    let t = std::time::Instant::now();
    h.run_steps(5);
    // Zoom in and out around the pointer, pan with the wheel.
    h.event(egui::Event::PointerMoved(egui::pos2(500.0, 350.0)));
    h.event(egui::Event::Zoom(4.0));
    h.step();
    h.event(egui::Event::Zoom(0.01));
    h.step();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -300.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    h.run_steps(5);
    key(&mut h, egui::Key::A, egui::Modifiers::COMMAND);
    assert_eq!(h.state().view.selection.len(), 1001);
    assert!(t.elapsed().as_secs() < 30);
}
