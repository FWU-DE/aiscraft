//! Render a demo whiteboard offscreen (no window, no focus stealing) and save a PNG.
//!
//! ```sh
//! cargo run -p aiscraft-board --example board_snapshot -- --out board.png --size 1400x900 --theme dark
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_board::{Board, BoardStyle, BoardView, Element, Kind, Tool, ViewState};

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

struct App {
    board: Board,
    view: ViewState,
    style: BoardStyle,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").unwrap_or_else(|| "board.png".into());
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
        .unwrap_or((1400.0, 900.0));
    let scale: f32 = arg(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let dark = arg(&args, "--theme").is_some_and(|t| t == "dark");

    let (board, selected) = demo();
    let mut view = ViewState::default();
    view.selection = selected;
    view.tool = Tool::Select;
    view.zoom_to_fit();
    let style = if dark { BoardStyle::dark() } else { BoardStyle::light() };
    let app = App { board, view, style };
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(scale)
        .with_max_steps(64)
        .allow_missing_glyphs()
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                ui.style_mut().visuals.panel_fill = app.style.bg;
                BoardView::new(&mut app.board, &mut app.view, &app.style).show(ui);
            },
            app,
        );
    harness.run_steps(6);
    let img = harness.render().expect("render");
    img.save(&out).expect("save png");
    println!("wrote {out} ({}×{})", img.width(), img.height());
    println!("{}", harness.state().board.summary_text());
}
