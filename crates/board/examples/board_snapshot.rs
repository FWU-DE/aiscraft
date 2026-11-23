//! Render a demo whiteboard offscreen (no window, no focus stealing) and save a PNG.
//!
//! ```sh
//! cargo run -p aiscraft-board --example board_snapshot -- --out board.png --size 1400x900 --theme dark
//! cargo run -p aiscraft-board --example board_snapshot -- --scene mindmap --size 650x800 --scale 1.25
//! ```
//!
//! Scenes: `demo` (default), `mindmap` (fresh mind map), `selected` (note with „+“ handles and
//! context bar), `rebind` (an arrow end being dragged to another note), `empty`, `help`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_board::{Board, BoardStyle, BoardView, Element, Kind, Tool, ViewState, arrow_endpoints};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

const OUTLINE: &str = "# Photosynthese\n\
## Was passiert?\n\
- Blätter nehmen Kohlenstoffdioxid auf\n\
- Wurzeln liefern Wasser\n\
- Sonnenlicht liefert Energie\n\
## Produkte\n\
- Traubenzucker\n\
- Sauerstoff\n\
## Wo?\n\
- In den Chloroplasten\n\
## Im Unterricht\n\
- Versuch mit Wasserpest\n\
- Arbeitsblatt Klasse 7\n";

fn demo() -> (Board, Vec<String>) {
    let mut b = Board::new();
    b.set_title("Biologie 7b");
    b.add_outline(OUTLINE, 0.0, 0.0);
    // A frame with notes below the mind map.
    b.add(Element::new(Kind::Frame, -560.0, 420.0).with_size(700.0, 300.0).with_text("Hausaufgaben"));
    b.add(Element::new(Kind::Note, -520.0, 470.0).with_size(200.0, 130.0).with_text("Seite 42, Aufgabe 3 bis Freitag").with_color("pink"));
    b.add(Element::new(Kind::Note, -290.0, 470.0).with_size(200.0, 130.0).with_text("Blatt mitbringen").with_color("green"));
    // Shapes, an arrow between them, a text label and a drawing.
    let r = b.add(Element::new(Kind::Rect, 260.0, 470.0).with_size(180.0, 100.0).with_text("Licht").with_color("orange"));
    let e = b.add(Element::new(Kind::Ellipse, 560.0, 480.0).with_size(170.0, 90.0).with_text("Zucker").with_color("blue"));
    let ra = b.element(&r).unwrap();
    let ea = b.element(&e).unwrap();
    b.add(Element::connector(&ra, &ea).with_color("ink"));
    b.add(Element::new(Kind::Text, 260.0, 610.0).with_size(320.0, 40.0).with_text("Energie wird gespeichert").with_color("ink"));
    let wave: Vec<(f64, f64)> = (0..60).map(|i| (820.0 + i as f64 * 5.0, 620.0 + (i as f64 * 0.35).sin() * 22.0)).collect();
    b.add(Element::path(Kind::Freehand, &wave).with_color("red"));
    b.add(Element::segment(Kind::Line, (260.0, 680.0), (700.0, 680.0)).with_color("gray"));
    b.clear_history();
    (b, vec![r])
}

const OUTLINE_3X3: &str = "# Photosynthese\n## Was passiert?\n- Blätter nehmen Kohlenstoffdioxid auf\n- Wurzeln liefern Wasser\n- Sonnenlicht liefert Energie\n## Produkte\n- Traubenzucker\n- Sauerstoff\n- Stärke als Speicher\n## Im Unterricht\n- Versuch mit Wasserpest\n- Arbeitsblatt Klasse 7\n- Kurzer Test\n";

/// Two notes and an arrow from the first to the second, plus a third note to re-bind to.
fn small() -> (Board, String, String, String) {
    let mut b = Board::new();
    let a = b.add(Element::new(Kind::Note, -320.0, -60.0).with_text("Idee sammeln\nmit der Klasse").with_color("yellow"));
    let c = b.add(Element::new(Kind::Note, 80.0, -200.0).with_text("Recherche").with_color("blue"));
    let d = b.add(Element::new(Kind::Note, 80.0, 120.0).with_text("Präsentation vorbereiten").with_color("green"));
    let arrow = b.add(Element::connector(&b.element(&a).unwrap(), &b.element(&c).unwrap()).with_color("gray"));
    b.clear_history();
    (b, a, arrow, d)
}

struct App {
    board: Board,
    view: ViewState,
    style: BoardStyle,
    rect: egui::Rect,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").unwrap_or_else(|| "board.png".into());
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
        .unwrap_or((1400.0, 900.0));
    let scale: f32 = arg(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let dark = arg(&args, "--theme").is_some_and(|t| t == "dark");

    let scene = arg(&args, "--scene").unwrap_or_else(|| "demo".into());
    let mut view = ViewState::default();
    view.tool = Tool::Select;
    let mut rebind: Option<(String, String)> = None;
    let board = match scene.as_str() {
        "mindmap" => Board::from_outline(OUTLINE_3X3),
        "selected" => {
            let (b, a, _, _) = small();
            view.selection = vec![a];
            b
        }
        "rebind" => {
            let (b, _, arrow, d) = small();
            view.selection = vec![arrow.clone()];
            rebind = Some((arrow, d));
            b
        }
        "empty" => Board::new(),
        "help" => {
            view.help = true;
            demo().0
        }
        _ => {
            let (b, selected) = demo();
            view.selection = selected;
            b
        }
    };
    view.zoom_to_fit();
    let style = if dark { BoardStyle::dark() } else { BoardStyle::light() };
    let app = App { board, view, style, rect: egui::Rect::NOTHING };
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(scale)
        .with_max_steps(64)
        .allow_missing_glyphs()
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                ui.style_mut().visuals.panel_fill = app.style.bg;
                app.rect = BoardView::new(&mut app.board, &mut app.view, &app.style).show(ui).rect;
            },
            app,
        );
    harness.run_steps(6);
    if let Some((arrow, target)) = rebind {
        // Grab the arrow's head and drag it onto the third note, without letting go.
        let st = harness.state();
        let els = st.board.elements();
        let el = els.iter().find(|e| e.id == arrow).unwrap();
        let (_, end) = arrow_endpoints(el, |id| els.iter().find(|e| e.id == id));
        let t = els.iter().find(|e| e.id == target).unwrap();
        let to_screen = |x: f64, y: f64| st.view.camera.to_screen(st.rect, egui::pos2(x as f32, y as f32));
        let (from, to) = (to_screen(end.0, end.1), to_screen(t.x + t.w * 0.4, t.y + t.h * 0.6));
        use egui::{Event, Modifiers, PointerButton};
        harness.event(Event::PointerMoved(from));
        harness.step();
        harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.step();
        for i in 1..=10 {
            harness.event(Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
            harness.step();
        }
    }
    if scene == "selected" {
        // Hover the right „+“ handle so its highlight shows.
        let st = harness.state();
        let el = st.board.elements().into_iter().find(|e| st.view.selection.contains(&e.id)).unwrap();
        let p = st.view.camera.to_screen(st.rect, egui::pos2((el.x + el.w) as f32, (el.y + el.h / 2.0) as f32)) + egui::vec2(26.0, 0.0);
        harness.event(egui::Event::PointerMoved(p));
        harness.run_steps(2);
    }
    let img = harness.render().expect("render");
    img.save(&out).expect("save png");
    println!("wrote {out} ({}×{})", img.width(), img.height());
    println!("zoom {:.0} %", harness.state().view.camera.zoom * 100.0);
}
