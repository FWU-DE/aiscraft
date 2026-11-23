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
    // Root at the origin, every other note beside it.
    let root = notes.iter().find(|n| n.text == "Klassenfahrt").unwrap();
    assert!((root.center().0).abs() < 1.0 && (root.center().1).abs() < 1.0);
    assert!(notes.iter().filter(|n| n.id != root.id).all(|n| n.x > root.x + root.w || n.x + n.w < root.x));

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
    rect: egui::Rect,
}

type H = egui_kittest::Harness<'static, App>;

fn harness(board: Board) -> H {
    harness_sized(board, crate::ViewState::default(), egui::vec2(1000.0, 700.0))
}

fn harness_sized(board: Board, view: crate::ViewState, size: egui::Vec2) -> H {
    let app = App { board, view, style: crate::BoardStyle::light(), rect: egui::Rect::NOTHING };
    egui_kittest::Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            app.rect = crate::BoardView::new(&mut app.board, &mut app.view, &app.style).show(ui).rect;
        },
        app,
    )
}

/// Screen position of a world point.
fn screen(h: &H, x: f64, y: f64) -> egui::Pos2 {
    let st = h.state();
    st.view.camera.to_screen(st.rect, egui::pos2(x as f32, y as f32))
}

fn click(h: &mut H, at: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
}

fn drag(h: &mut H, from: egui::Pos2, to: egui::Pos2) {
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

fn key(h: &mut H, key: egui::Key, modifiers: egui::Modifiers) {
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
}

#[test]
fn widget_draws_moves_and_undoes() {
    let mut h = harness(Board::new());
    h.state_mut().view.snap = false;
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

// ----- UX behaviour --------------------------------------------------------------------------

const OUTLINE_3X3: &str = "# Photosynthese\n## Was passiert?\n- Blätter nehmen Kohlenstoffdioxid auf\n- Wurzeln liefern Wasser\n- Sonnenlicht liefert Energie\n## Produkte\n- Traubenzucker\n- Sauerstoff\n- Stärke als Speicher\n## Im Unterricht\n- Versuch mit Wasserpest\n- Arbeitsblatt Klasse 7\n- Kurzer Test\n";

fn notes_of(b: &Board) -> Vec<Element> {
    b.elements().into_iter().filter(|e| e.kind == Kind::Note).collect()
}

#[test]
fn mind_map_branches_are_balanced_across_sides() {
    // Many short branches: both sides, split evenly (not 3 + 1).
    let md = "# Thema\n- A\n- B\n- C\n- D\n- E\n- F\n- G\n- H\n";
    let b = Board::from_outline(md);
    let notes = notes_of(&b);
    let root = notes.iter().find(|n| n.text == "Thema").unwrap();
    let right = notes.iter().filter(|n| n.x > root.x + root.w).count();
    let left = notes.iter().filter(|n| n.x + n.w < root.x).count();
    assert_eq!((right, left), (4, 4));
    let four = Board::from_outline("# T\n- A\n- B\n- C\n- D\n");
    let notes = notes_of(&four);
    let root = notes.iter().find(|n| n.text == "T").unwrap();
    let right = notes.iter().filter(|n| n.x > root.x + root.w).count();
    let left = notes.iter().filter(|n| n.x + n.w < root.x).count();
    assert_eq!(right + left, 4);
    assert!(right.abs_diff(left) <= 2, "{right} / {left}");
    for (i, a) in notes.iter().enumerate() {
        for c in notes.iter().skip(i + 1) {
            assert!(!overlaps(a, c), "{} overlaps {}", a.text, c.text);
        }
    }
}

#[test]
fn mind_map_fits_readably_in_a_side_panel() {
    let b = Board::from_outline(OUTLINE_3X3);
    let notes = notes_of(&b);
    assert_eq!(notes.len(), 13);
    for (i, a) in notes.iter().enumerate() {
        for c in notes.iter().skip(i + 1) {
            assert!(!overlaps(a, c), "{} overlaps {}", a.text, c.text);
        }
    }
    let mut view = crate::ViewState::default();
    view.zoom_to_fit();
    let mut h = harness_sized(b, view, egui::vec2(650.0, 800.0));
    h.run_steps(3);
    let st = h.state();
    assert!(st.view.camera.zoom >= 0.65, "zoom {}", st.view.camera.zoom);
    // Everything is visible at that zoom.
    let vis = st.view.camera.visible(st.rect);
    for n in &notes {
        assert!(vis.contains(egui::pos2(n.x as f32, n.y as f32)) && vis.contains(egui::pos2((n.x + n.w) as f32, (n.y + n.h) as f32)), "{} not visible", n.text);
    }
}

#[test]
fn fit_never_goes_below_readable_zoom() {
    let mut b = Board::new();
    b.add(note(-5000.0, 0.0, "links"));
    b.add(note(5000.0, 0.0, "rechts"));
    let mut view = crate::ViewState::default();
    view.zoom_to_fit();
    let mut h = harness_sized(b, view, egui::vec2(650.0, 800.0));
    h.run_steps(2);
    assert!((h.state().view.camera.zoom - crate::view::READABLE_ZOOM).abs() < 1e-3);
}

#[test]
fn first_fit_follows_the_settled_canvas_size() {
    let b = Board::from_outline(OUTLINE_3X3);
    let mut view = crate::ViewState::default();
    view.zoom_to_fit();
    let mut h = harness_sized(b, view, egui::vec2(300.0, 300.0));
    h.run_steps(2);
    let small = h.state().view.camera.zoom;
    h.set_size(egui::vec2(1400.0, 1000.0));
    h.run_steps(3);
    let big = h.state().view.camera.zoom;
    assert!(big > small + 0.05, "{small} -> {big}");
    // Once the user zooms, a resize no longer refits.
    h.state_mut().view.camera.zoom = 0.5;
    h.run_steps(1);
    h.set_size(egui::vec2(900.0, 700.0));
    h.run_steps(2);
    assert!((h.state().view.camera.zoom - 0.5).abs() < 1e-3);
}

#[test]
fn arrow_end_can_be_rebound_by_dragging() {
    let mut b = Board::new();
    let a = b.add(note(0.0, 0.0, "A"));
    let c = b.add(note(400.0, 0.0, "B"));
    let d = b.add(note(400.0, 300.0, "C"));
    let arrow = b.add(Element::connector(&b.element(&a).unwrap(), &b.element(&c).unwrap()));
    let mut h = harness(b);
    h.run_steps(2);
    h.state_mut().view.selection = vec![arrow.clone()];
    h.run_steps(1);
    let end = {
        let els = h.state().board.elements();
        let el = els.iter().find(|e| e.id == arrow).unwrap();
        arrow_endpoints(el, |id| els.iter().find(|e| e.id == id)).1
    };
    let from = screen(&h, end.0, end.1);
    let to = screen(&h, 500.0, 370.0);
    drag(&mut h, from, to);
    let el = h.state().board.element(&arrow).unwrap();
    assert_eq!(el.to.as_deref(), Some(d.as_str()), "{el:?}");
    assert_eq!(el.from.as_deref(), Some(a.as_str()));
    // One undo step brings the old binding back.
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(h.state().board.element(&arrow).unwrap().to.as_deref(), Some(c.as_str()));
    // Dropping on empty canvas unbinds that end and leaves it where it was dropped.
    key(&mut h, egui::Key::Z, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
    h.state_mut().view.selection = vec![arrow.clone()];
    h.run_steps(1);
    let start = {
        let els = h.state().board.elements();
        let el = els.iter().find(|e| e.id == arrow).unwrap();
        arrow_endpoints(el, |id| els.iter().find(|e| e.id == id)).1
    };
    let (from, to) = (screen(&h, start.0, start.1), screen(&h, 800.0, 600.0));
    drag(&mut h, from, to);
    let el = h.state().board.element(&arrow).unwrap();
    assert_eq!(el.to, None);
    let end = el.world_points().last().unwrap();
    assert!((end.0 - 800.0).abs() < 2.0 && (end.1 - 600.0).abs() < 2.0, "{end:?}");
}

#[test]
fn notes_grow_with_their_text_while_typing() {
    let mut b = Board::new();
    let id = b.add(note(0.0, 0.0, ""));
    let mut h = harness(b);
    h.run_steps(2);
    let h0 = h.state().board.element(&id).unwrap().h;
    h.state_mut().view.edit(&id);
    h.run_steps(2);
    let long = "Eine längere Notiz mit sehr viel Text, die beim Schreiben mehrere Zeilen braucht und deshalb wachsen muss. ".repeat(3);
    h.event(egui::Event::Text(long.clone()));
    h.run_steps(2);
    // Enter finishes editing; the grown height and the text are stored together.
    key(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().view.editing().is_none());
    let el = h.state().board.element(&id).unwrap();
    assert_eq!(el.text, long);
    assert!(el.h > h0 + 40.0, "{} -> {}", h0, el.h);
    h.event(egui::Event::PointerMoved(egui::pos2(900.0, 650.0)));
    h.step();
    // Capped for very long text.
    h.state_mut().view.edit(&id);
    h.run_steps(2);
    h.event(egui::Event::Text("noch mehr Text ".repeat(200)));
    h.run_steps(2);
    key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run_steps(2);
    let el = h.state().board.element(&id).unwrap();
    assert!(el.h <= 520.0 + 0.5, "{}", el.h);
    // Text and height came in one undo step.
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    let el = h.state().board.element(&id).unwrap();
    assert_eq!(el.text, long);
}

#[test]
fn shift_enter_is_a_newline_enter_finishes() {
    let mut b = Board::new();
    let id = b.add(note(0.0, 0.0, ""));
    let mut h = harness(b);
    h.run_steps(2);
    h.state_mut().view.edit(&id);
    h.run_steps(2);
    h.event(egui::Event::Text("eins".into()));
    h.step();
    key(&mut h, egui::Key::Enter, egui::Modifiers::SHIFT);
    h.event(egui::Event::Text("zwei".into()));
    h.step();
    assert!(h.state().view.editing().is_some());
    key(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().view.editing().is_none());
    assert_eq!(h.state().board.element(&id).unwrap().text, "eins\nzwei");
    assert_eq!(h.state().view.selection, vec![id]);
}

#[test]
fn double_click_on_empty_canvas_creates_a_note_and_edits_it() {
    let mut h = harness(Board::new());
    h.run_steps(2);
    let at = egui::pos2(500.0, 400.0);
    click(&mut h, at);
    click(&mut h, at);
    h.run_steps(2);
    let els = h.state().board.elements();
    assert_eq!(els.len(), 1, "{els:?}");
    assert_eq!(els[0].kind, Kind::Note);
    assert_eq!(h.state().view.editing(), Some(els[0].id.as_str()));
    let c = screen(&h, els[0].center().0, els[0].center().1);
    assert!(c.distance(at) < 2.0, "{c:?}");
}

#[test]
fn plus_handle_creates_a_connected_note() {
    let mut b = Board::new();
    let id = b.add(note(0.0, 0.0, "Start").with_color("blue"));
    let mut h = harness(b);
    h.run_steps(2);
    h.state_mut().view.selection = vec![id.clone()];
    h.run_steps(1);
    // The right „+“ sits 26 px right of the note's edge.
    let src = h.state().board.element(&id).unwrap();
    let right = screen(&h, src.x + src.w, src.y + src.h / 2.0) + egui::vec2(26.0, 0.0);
    click(&mut h, right);
    h.run_steps(2);
    let els = h.state().board.elements();
    assert_eq!(els.len(), 3, "{els:?}");
    let new = els.iter().find(|e| e.kind == Kind::Note && e.id != id).unwrap();
    assert!(new.x > src.x + src.w + 20.0 && (new.y - src.y).abs() < 1.0, "{new:?}");
    assert_eq!(new.color, "blue");
    let arrow = els.iter().find(|e| e.kind == Kind::Arrow).unwrap();
    assert_eq!((arrow.from.as_deref(), arrow.to.as_deref()), (Some(id.as_str()), Some(new.id.as_str())));
    assert_eq!(h.state().view.editing(), Some(new.id.as_str()));
    // Typing goes into the new note; one undo removes note and arrow together.
    h.event(egui::Event::Text("Nächster Schritt".into()));
    h.step();
    key(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(h.state().board.element(&new.id).unwrap().text, "Nächster Schritt");
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(h.state().board.len(), 1);
    // A second click on the same side stacks below instead of overlapping.
    key(&mut h, egui::Key::Z, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
    h.state_mut().view.selection = vec![id.clone()];
    h.run_steps(1);
    click(&mut h, right);
    h.run_steps(2);
    let notes = notes_of(&h.state().board);
    assert_eq!(notes.len(), 3);
    for (i, a) in notes.iter().enumerate() {
        for c in notes.iter().skip(i + 1) {
            assert!(!overlaps(a, c), "{} overlaps {}", a.text, c.text);
        }
    }
}

#[test]
fn tab_while_editing_a_mind_map_node_adds_a_child() {
    let b = Board::from_outline("# Wurzel\n- Ast\n");
    let mut h = harness(b);
    h.run_steps(2);
    let ast = notes_of(&h.state().board).into_iter().find(|n| n.text == "Ast").unwrap();
    let root = notes_of(&h.state().board).into_iter().find(|n| n.text == "Wurzel").unwrap();
    h.state_mut().view.edit(&ast.id);
    h.run_steps(2);
    key(&mut h, egui::Key::Tab, egui::Modifiers::NONE);
    h.run_steps(2);
    let notes = notes_of(&h.state().board);
    assert_eq!(notes.len(), 3);
    let child = notes.iter().find(|n| n.id != ast.id && n.id != root.id).unwrap();
    // Away from the parent, connected, and being edited.
    let away = (ast.center().0 - root.center().0).signum();
    assert_eq!((child.center().0 - ast.center().0).signum(), away);
    assert_eq!(h.state().view.editing(), Some(child.id.as_str()));
    assert!(
        h.state()
            .board
            .elements()
            .iter()
            .any(|e| e.kind == Kind::Arrow && e.from.as_deref() == Some(ast.id.as_str()) && e.to.as_deref() == Some(child.id.as_str()))
    );
    // The text so far contains no tab character.
    assert!(!h.state().board.element(&ast.id).unwrap().text.contains('\t'));
}

#[test]
fn moves_snap_to_grid_and_to_other_elements() {
    let mut b = Board::new();
    let a = b.add(note(0.0, 0.0, "fest"));
    let m = b.add(note(400.0, 203.0, "beweglich"));
    let mut h = harness(b);
    h.run_steps(2);
    // Grid: a small move lands on a multiple of 24.
    let from = screen(&h, 500.0, 270.0);
    drag(&mut h, from, from + egui::vec2(37.0, 11.0));
    let e = h.state().board.element(&m).unwrap();
    assert!((e.x / 24.0 - (e.x / 24.0).round()).abs() < 1e-3 && (e.y / 24.0 - (e.y / 24.0).round()).abs() < 1e-3, "{e:?}");
    // Edges: moved near the fixed note's top edge, it lines up exactly (even off-grid).
    h.state_mut().board.move_to(&a, 7.0, 5.0);
    h.run_steps(1);
    let e = h.state().board.element(&m).unwrap();
    let grab = screen(&h, e.x + 50.0, e.y + 50.0);
    let target = screen(&h, e.x + 50.0, 5.0 + 3.0 + 50.0);
    drag(&mut h, grab, target);
    let e = h.state().board.element(&m).unwrap();
    assert!((e.y - 5.0).abs() < 1e-3, "{e:?}");
    // Alt moves freely.
    let e0 = e.clone();
    let grab = screen(&h, e.x + 50.0, e.y + 50.0);
    alt_drag(&mut h, grab, grab + egui::vec2(13.0, 7.0));
    let e = h.state().board.element(&m).unwrap();
    let z = h.state().view.camera.zoom as f64;
    assert!((e.x - e0.x - 13.0 / z).abs() < 0.5 && (e.y - e0.y - 7.0 / z).abs() < 0.5, "{e0:?} -> {e:?}");
}

fn alt_drag(h: &mut H, from: egui::Pos2, to: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    let alt = Modifiers::ALT;
    h.event(Event::ModifiersChanged(alt));
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: alt });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: alt });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
}

#[test]
fn help_overlay_opens_with_f1_and_closes_with_escape() {
    let mut h = harness(Board::new());
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(egui::pos2(500.0, 400.0)));
    h.step();
    key(&mut h, egui::Key::F1, egui::Modifiers::NONE);
    assert!(h.state().view.help);
    // While it is open, clicks do not create anything.
    click(&mut h, egui::pos2(500.0, 400.0));
    assert!(!h.state().view.help);
    assert!(h.state().board.is_empty());
    key(&mut h, egui::Key::Questionmark, egui::Modifiers::SHIFT);
    assert!(h.state().view.help);
    key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!h.state().view.help);
}
