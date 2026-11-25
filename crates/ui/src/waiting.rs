//! What the chat shows while an answer is on its way: small looping animations painted with the
//! theme's tokens (no Lottie, no new dependency, no glyphs) and a status line that changes every
//! few seconds. Einstellungen → Darstellung → „Warte-Animation“ picks one, *Abwechselnd* (a
//! different one per answer, stable for that answer) or *Schlicht* (the spinner).
//!
//! Time comes from egui's input time, never the wall clock, so snapshots and tests are
//! deterministic. While an animation is visible it asks for about 30 frames a second; nothing
//! runs when it is not on screen.

use std::f32::consts::TAU;
use std::time::Duration;

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Ui, vec2};

use crate::theme::Tokens;

/// One animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anim {
    /// A pencil writes a line of loops and erases it on the way back.
    Pencil,
    /// A light bulb flickers on, small ideas rise from it.
    Bulb,
    /// An open book whose pages turn.
    Books,
    /// Planets circle a small sun.
    Planets,
    /// A paper plane flies a figure of eight.
    Plane,
    /// A brush dabs colour blobs onto a sheet (image chats).
    Brush,
}

/// Every animation, in the order the settings list them.
pub const ALL: [Anim; 6] = [Anim::Pencil, Anim::Bulb, Anim::Books, Anim::Planets, Anim::Plane, Anim::Brush];

/// The ones *Abwechselnd* takes turns with in text chats (image chats get the brush).
const MIX: [Anim; 5] = [Anim::Pencil, Anim::Bulb, Anim::Books, Anim::Planets, Anim::Plane];

/// The values of the `wait_animation` setting with their names, as the settings offer them.
pub const CHOICES: [(&str, &str); 8] = [
    ("mix", "Abwechselnd"),
    ("pencil", "Bleistift"),
    ("bulb", "Glühbirne"),
    ("books", "Bücher"),
    ("planets", "Planeten"),
    ("plane", "Papierflieger"),
    ("brush", "Pinsel"),
    ("plain", "Schlicht"),
];

/// Base size of the drawing area; the animations scale with its height.
pub const SIZE: egui::Vec2 = vec2(104.0, 56.0);

/// Status lines while a text answer is on its way; the first one is what people know.
const CHAT_LINES: [&str; 8] = [
    "Denkt nach …",
    "Sortiert die Gedanken …",
    "Schlägt nach …",
    "Formuliert …",
    "Sucht die passenden Worte …",
    "Spitzt den Bleistift …",
    "Prüft noch einmal …",
    "Bringt es auf den Punkt …",
];

/// Status lines while a picture is drawn.
const IMAGE_LINES: [&str; 5] =
    ["Bild wird gezeichnet …", "Mischt die Farben …", "Setzt die ersten Striche …", "Sucht den richtigen Ausschnitt …", "Gibt den letzten Schliff …"];

/// Seconds each status line stays.
const LINE_SECS: f64 = 3.2;

impl Anim {
    /// The setting's value for this animation.
    pub fn key(self) -> &'static str {
        match self {
            Anim::Pencil => "pencil",
            Anim::Bulb => "bulb",
            Anim::Books => "books",
            Anim::Planets => "planets",
            Anim::Plane => "plane",
            Anim::Brush => "brush",
        }
    }

    /// The German name.
    pub fn name(self) -> &'static str {
        CHOICES.iter().find(|(k, _)| *k == self.key()).map(|(_, n)| *n).unwrap_or("Animation")
    }

    fn from_key(key: &str) -> Option<Anim> {
        ALL.into_iter().find(|a| a.key() == key)
    }
}

/// A stable hash (FNV-1a): the same message id gives the same animation in every run.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Which animation a waiting message shows: `None` for *Schlicht* (the spinner). *Abwechselnd*
/// (and any unknown value) picks one from the message id, so it stays the same while the
/// message waits; image chats get the brush.
pub fn pick(setting: &str, message_id: &str, image_chat: bool) -> Option<Anim> {
    if setting == "plain" {
        return None;
    }
    if let Some(a) = Anim::from_key(setting) {
        return Some(a);
    }
    if image_chat {
        return Some(Anim::Brush);
    }
    let i = (fnv(message_id) % MIX.len() as u64) as usize;
    Some(MIX.get(i).copied().unwrap_or(Anim::Pencil))
}

/// The status line after `elapsed` seconds: the first line first, then the others in an order
/// that depends on the message.
pub fn status_line(elapsed: f64, message_id: &str, image_chat: bool) -> &'static str {
    let lines: &[&'static str] = if image_chat { &IMAGE_LINES } else { &CHAT_LINES };
    let fallback = if image_chat { "Bild wird gezeichnet …" } else { "Denkt nach …" };
    let step = if elapsed.is_finite() && elapsed > 0.0 { (elapsed / LINE_SECS) as u64 } else { 0 };
    if step == 0 || lines.len() < 2 {
        return lines.first().copied().unwrap_or(fallback);
    }
    // Then the other lines, in a rotation that depends on the message.
    let others = lines.len() as u64 - 1;
    let start = fnv(message_id) % others;
    let i = 1 + ((start + (step - 1) % others) % others) as usize;
    lines.get(i).copied().unwrap_or(fallback)
}

/// The waiting row under a message: the animation (or the spinner) and the status line.
pub fn show(ui: &mut Ui, t: &Tokens, setting: &str, message_id: &str, image_chat: bool) {
    let now = ui.input(|i| i.time);
    let id = egui::Id::unique(("wait-start", message_id));
    let start = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    let elapsed = (now - start).max(0.0);
    match pick(setting, message_id, image_chat) {
        None => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(t.accent));
                ui.label(crate::widgets::muted(t, if image_chat { "Bild wird gezeichnet …" } else { "Denkt nach …" }));
            });
        }
        Some(anim) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                canvas(ui, t, anim, SIZE, elapsed as f32, "Warte-Animation");
                ui.label(crate::widgets::muted(t, status_line(elapsed, message_id, image_chat)));
            });
        }
    }
}

/// A small live preview of the setting for Einstellungen → Darstellung: *Abwechselnd* shows
/// one animation after another, *Schlicht* the spinner.
pub fn preview(ui: &mut Ui, t: &Tokens, setting: &str) {
    let time = ui.input(|i| i.time);
    let size = SIZE * 0.6;
    let anim = match Anim::from_key(setting) {
        Some(a) => Some(a),
        None if setting == "plain" => None,
        None => {
            let step = if time.is_finite() && time > 0.0 { (time / 4.0) as u64 } else { 0 };
            MIX.get((step % MIX.len() as u64) as usize).copied()
        }
    };
    match anim {
        Some(a) => {
            canvas(ui, t, a, size, time.rem_euclid(3600.0) as f32, "Vorschau");
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            egui::Spinner::new().color(t.accent).paint_at(ui, Rect::from_center_size(rect.center(), vec2(20.0, 20.0)));
        }
    }
}

/// Allocates `size` and paints `anim` at `time` seconds, asking for the next frame while it is
/// on screen.
pub fn canvas(ui: &mut Ui, t: &Tokens, anim: Anim, size: egui::Vec2, time: f32, what: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, format!("{what}: {}", anim.name())));
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), rect, t, anim, time);
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
    resp
}

/// Paints one frame of `anim` into `rect` at `time` seconds. Never panics, whatever the time.
pub fn paint(p: &Painter, rect: Rect, t: &Tokens, anim: Anim, time: f32) {
    let time = if time.is_finite() { time.rem_euclid(3600.0) } else { 0.0 };
    if !rect.is_finite() || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let p = p.with_clip_rect(rect.expand(2.0).intersect(p.clip_rect()));
    let f = Frame::new(rect);
    match anim {
        Anim::Pencil => pencil(&p, &f, t, time),
        Anim::Bulb => bulb(&p, &f, t, time),
        Anim::Books => books(&p, &f, t, time),
        Anim::Planets => planets(&p, &f, t, time),
        Anim::Plane => plane(&p, &f, t, time),
        Anim::Brush => brush(&p, &f, t, time),
    }
}

/// Maps the 104 × 56 design grid onto the rect (centred, keeping the proportions).
struct Frame {
    origin: Pos2,
    s: f32,
}

impl Frame {
    fn new(r: Rect) -> Self {
        let s = (r.width() / SIZE.x).min(r.height() / SIZE.y);
        let origin = r.center() - vec2(SIZE.x, SIZE.y) * s * 0.5;
        Frame { origin, s }
    }
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.s
    }
    fn len(&self, v: f32) -> f32 {
        v * self.s
    }
}

/// Smooth start and end (0..1 → 0..1).
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where `x` lies between `a` and `b`, clamped to 0..1.
fn span(x: f32, a: f32, b: f32) -> f32 {
    if b <= a { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / (b - a)).clamp(0.0, 1.0) }
}

/// The phase 0..1 of a loop of `period` seconds.
fn phase(time: f32, period: f32) -> f32 {
    (time / period).rem_euclid(1.0)
}

fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A warm light colour that reads on light and dark backgrounds.
fn light(t: &Tokens) -> Color32 {
    t.warning.lerp_to_gamma(Color32::from_rgb(0xff, 0xcc, 0x33), if t.dark { 0.3 } else { 0.55 })
}

/// A small four-pointed star.
fn star(p: &Painter, c: Pos2, r: f32, color: Color32) {
    let pts: Vec<Pos2> = (0..8)
        .map(|i| {
            let a = i as f32 * TAU / 8.0 - TAU / 4.0;
            let rr = if i % 2 == 0 { r } else { r * 0.38 };
            c + vec2(a.cos(), a.sin()) * rr
        })
        .collect();
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// The pencil: writes a row of loops (like „eeee“), waits, then goes back and erases them.
fn pencil(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let ph = phase(time, 4.2);
    let (x0, len, base, r, loops) = (14.0, 72.0, 38.0, 5.5, 5.0);
    // A trochoid: loops along the line.
    let point = |u: f32| {
        let w = TAU * loops * u;
        f.at(x0 + len * u - r * w.sin(), base - r * (1.0 - w.cos()))
    };
    let (shown, tip_u, lift) = if ph < 0.55 {
        let u = ease(span(ph, 0.0, 0.55));
        (u, u, 0.0)
    } else if ph < 0.62 {
        (1.0, 1.0, ease(span(ph, 0.55, 0.62)) * 4.0)
    } else {
        let u = 1.0 - ease(span(ph, 0.62, 0.97));
        (u, u, 4.0 * (1.0 - ease(span(ph, 0.9, 1.0))))
    };
    // Ruled line under the writing.
    p.line_segment([f.at(8.0, base + 4.0), f.at(96.0, base + 4.0)], Stroke::new(f.len(1.0), alpha(t.muted, 0.35)));
    let n = (shown * 90.0).ceil() as usize;
    if n >= 1 {
        let pts: Vec<Pos2> = (0..=n).map(|i| point(shown * i as f32 / n as f32)).collect();
        p.add(Shape::line(pts, Stroke::new(f.len(2.0), t.text2)));
    }
    let tip = point(tip_u) - vec2(0.0, f.len(lift));
    // The pencil leans to the right: axis from the tip up and to the right.
    let a = (-48.0f32).to_radians();
    let d = vec2(a.cos(), a.sin());
    let n = vec2(-d.y, d.x);
    let w = f.len(3.6);
    let at = |along: f32, side: f32| tip + d * f.len(along) + n * side;
    let wood = t.warning.lerp_to_gamma(Color32::from_rgb(0xf3, 0xd9, 0xa4), 0.75);
    p.add(Shape::convex_polygon(vec![at(0.0, 0.0), at(7.0, w), at(7.0, -w)], wood, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(0.0, 0.0), at(2.6, w * 0.37), at(2.6, -w * 0.37)], t.text, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(7.0, w), at(24.0, w), at(24.0, -w), at(7.0, -w)], t.accent, Stroke::NONE));
    p.line_segment([at(7.0, 0.0), at(24.0, 0.0)], Stroke::new(f.len(0.8), alpha(Color32::WHITE, 0.35)));
    p.add(Shape::convex_polygon(vec![at(24.0, w), at(26.5, w), at(26.5, -w), at(24.0, -w)], t.muted, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(26.5, w), at(29.5, w * 0.8), at(30.5, 0.0), at(29.5, -w * 0.8), at(26.5, -w)], t.accent2, Stroke::NONE));
}

/// The light bulb: dark, two flickers, on with a soft halo and rays while small ideas rise; then
/// it dims again.
fn bulb(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let ph = phase(time, 5.0);
    let pulse = |a: f32, b: f32| ease(span(ph, a, (a + b) * 0.5)) * (1.0 - ease(span(ph, (a + b) * 0.5, b)));
    let glow = (0.55 * pulse(0.08, 0.14) + 0.75 * pulse(0.17, 0.23) + ease(span(ph, 0.26, 0.34)) * (1.0 - ease(span(ph, 0.86, 0.98)))).clamp(0.0, 1.0);
    let c = f.at(52.0, 23.0);
    let r = f.len(12.5);
    let warm = light(t);
    // Halo and rays.
    for (k, rr) in [(0.10, 23.0), (0.16, 18.5)] {
        p.circle_filled(c, f.len(rr), alpha(warm, glow * k));
    }
    for i in 0..7 {
        let a = TAU * (0.5 + i as f32 / 12.0) + TAU / 24.0;
        let dir = vec2(a.cos(), a.sin());
        let from = c + dir * f.len(16.5);
        let to = c + dir * f.len(16.5 + 5.0 * glow);
        if glow > 0.05 {
            p.line_segment([from, to], Stroke::new(f.len(1.6), alpha(warm, glow)));
        }
    }
    // Ideas: dots and stars rising while the light is on.
    for k in 0..5 {
        let life = phase(time + k as f32 * 0.53, 2.65);
        let x = 52.0 + (k as f32 * 2.4).sin() * 22.0 + (life * TAU + k as f32).sin() * 3.0;
        let y = 14.0 - life * 16.0;
        let fade = ease(span(life, 0.0, 0.2)) * (1.0 - ease(span(life, 0.6, 1.0))) * glow;
        if fade > 0.01 {
            let color = if k % 2 == 0 { t.accent } else { t.accent2 };
            if k % 3 == 1 {
                star(p, f.at(x, y), f.len(3.4), alpha(color, fade));
            } else {
                p.circle_filled(f.at(x, y), f.len(1.8), alpha(color, fade));
            }
        }
    }
    // Glass, filament, screw base.
    let glass = t.opaque(t.surface).lerp_to_gamma(warm, glow * 0.85);
    p.circle(c, r, glass, Stroke::new(f.len(1.5), t.text2));
    let neck = [f.at(46.5, 32.0), f.at(57.5, 32.0), f.at(56.0, 38.5), f.at(48.0, 38.5)];
    p.add(Shape::convex_polygon(neck.to_vec(), glass, Stroke::new(f.len(1.5), t.text2)));
    let fil: Vec<Pos2> = (0..=6).map(|i| f.at(48.0 + i as f32 * 1.33, 24.0 + if i % 2 == 0 { 0.0 } else { -2.5 })).collect();
    p.add(Shape::line(fil, Stroke::new(f.len(1.2), t.muted.lerp_to_gamma(t.warning, glow))));
    p.line_segment([f.at(49.0, 24.0), f.at(49.5, 33.0)], Stroke::new(f.len(1.0), t.muted));
    p.line_segment([f.at(55.0, 24.0), f.at(54.5, 33.0)], Stroke::new(f.len(1.0), t.muted));
    for (i, w) in [8.0, 7.0, 5.0].iter().enumerate() {
        let y = 40.0 + i as f32 * 3.2;
        let rr = Rect::from_center_size(f.at(52.0, y), vec2(f.len(*w * 2.0 - 4.0), f.len(2.6)));
        p.rect_filled(rr, f.len(1.3), t.muted);
    }
}

/// An open book; one page after another turns from right to left, and a ribbon sways.
fn books(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let ph = phase(time, 3.6);
    let cx = 52.0;
    let paper = t.opaque(t.surface);
    let edge = Stroke::new(f.len(1.2), t.text2);
    // Cover, a little larger than the pages.
    p.add(Shape::convex_polygon(vec![f.at(cx - 40.0, 13.0), f.at(cx, 17.0), f.at(cx, 49.0), f.at(cx - 40.0, 45.0)], t.accent, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![f.at(cx, 17.0), f.at(cx + 40.0, 13.0), f.at(cx + 40.0, 45.0), f.at(cx, 49.0)], t.accent, Stroke::NONE));
    let page = |side: f32| vec![f.at(cx, 18.0), f.at(cx + side * 36.0, 15.0), f.at(cx + side * 36.0, 43.0), f.at(cx, 46.0)];
    for side in [-1.0, 1.0] {
        p.add(Shape::convex_polygon(page(side), paper, edge));
        // Lines of text.
        for i in 0..5 {
            let y = 21.0 + i as f32 * 4.5;
            let (a, b) = if side < 0.0 { (cx - 31.0, cx - 5.0 - (i % 2) as f32 * 6.0) } else { (cx + 5.0, cx + 31.0 - (i % 3) as f32 * 5.0) };
            let tilt = |x: f32| y - (x - cx).abs() / 36.0 * 3.0;
            p.line_segment([f.at(a, tilt(a)), f.at(b, tilt(b))], Stroke::new(f.len(1.1), alpha(t.muted, 0.6)));
        }
    }
    // Turning pages: hinged at the spine, the free edge sweeps over (cos for the perspective).
    for k in 0..2 {
        let start = 0.08 + k as f32 * 0.32;
        let turn = ease(span(ph, start, start + 0.42));
        if turn <= 0.0 || turn >= 1.0 {
            continue;
        }
        let ang = turn * std::f32::consts::PI;
        let ex = 36.0 * ang.cos();
        let lift = 7.0 * ang.sin();
        let (top, edge_top, edge_bottom, bottom) = (f.at(cx, 18.0), f.at(cx + ex, 15.0 - lift), f.at(cx + ex, 43.0 - lift), f.at(cx, 46.0));
        let shade = paper.lerp_to_gamma(t.text2, 0.12 * ang.sin());
        // Clockwise either way; a nearly edge-on page is a line.
        if ex.abs() > 0.6 {
            let pts = if ex < 0.0 { vec![top, bottom, edge_bottom, edge_top] } else { vec![top, edge_top, edge_bottom, bottom] };
            p.add(Shape::convex_polygon(pts, shade, edge));
        } else {
            p.line_segment([top, bottom], edge);
        }
    }
    // Ribbon from the spine.
    let sway = (time * 1.7).sin() * 2.5;
    let ribbon = vec![f.at(cx - 1.5, 46.0), f.at(cx + 1.5, 46.0), f.at(cx + 1.5 + sway, 53.0), f.at(cx + sway, 51.5), f.at(cx - 1.5 + sway, 53.0)];
    p.add(Shape::convex_polygon(ribbon, t.accent2, Stroke::NONE));
}

/// A small sun with three planets on tilted orbits; the ones behind the sun pass behind it.
fn planets(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let c = f.at(52.0, 28.0);
    let warm = light(t);
    let orbits = [(17.0, 3.2, 2.6, t.accent2, 0.0), (29.0, 5.6, 4.0, t.accent, 1.9), (42.0, 9.0, 3.2, t.text2, 4.1)];
    let ry = 0.34;
    for (rx, ..) in orbits {
        let pts: Vec<Pos2> = (0..=64).map(|i| i as f32 * TAU / 64.0).map(|a| c + vec2(a.cos() * f.len(rx), a.sin() * f.len(rx * ry))).collect();
        p.add(Shape::closed_line(pts, Stroke::new(f.len(0.9), alpha(t.muted, 0.35))));
    }
    // Positions; sin > 0 is in front of the sun.
    let bodies: Vec<(Pos2, f32, Color32, bool)> = orbits
        .iter()
        .map(|(rx, period, size, color, off)| {
            let a = time * TAU / period + off;
            (c + vec2(a.cos() * f.len(*rx), a.sin() * f.len(rx * ry)), f.len(*size), *color, a.sin() > 0.0)
        })
        .collect();
    let draw = |(pos, size, color, _): &(Pos2, f32, Color32, bool)| {
        p.circle_filled(*pos, *size, *color);
        p.circle_filled(*pos - vec2(*size, *size) * 0.3, *size * 0.35, alpha(Color32::WHITE, 0.35));
    };
    bodies.iter().filter(|b| !b.3).for_each(draw);
    // The sun breathes a little.
    let breathe = 1.0 + 0.06 * (time * TAU / 3.0).sin();
    p.circle_filled(c, f.len(11.0 * breathe), alpha(warm, 0.16));
    p.circle_filled(c, f.len(7.0), warm);
    bodies.iter().filter(|b| b.3).for_each(draw);
    // A moon around the middle planet.
    if let Some((pos, ..)) = bodies.get(1) {
        let a = time * TAU / 1.6;
        p.circle_filled(*pos + vec2(a.cos() * f.len(7.0), a.sin() * f.len(3.0)), f.len(1.3), t.muted);
    }
}

/// A paper plane flies a figure of eight and leaves a dotted trail.
fn plane(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let period = 5.0;
    let path = |u: f32| {
        let a = u * TAU;
        f.at(52.0 + 38.0 * a.sin(), 28.0 + 15.0 * (2.0 * a).sin() * 0.9)
    };
    let u = phase(time, period);
    // Trail: dots behind the plane, fading.
    for i in 1..14 {
        let back = i as f32 * 0.022;
        let q = path(u - back);
        p.circle_filled(q, f.len(1.1), alpha(t.muted, 0.55 * (1.0 - i as f32 / 14.0)));
    }
    let pos = path(u);
    let ahead = path(u + 0.004);
    let dir = ahead - pos;
    let d = if dir.length() > 1e-4 { dir.normalized() } else { vec2(1.0, 0.0) };
    let n = vec2(-d.y, d.x);
    let at = |along: f32, side: f32| pos + d * f.len(along) + n * f.len(side);
    let nose = at(10.0, 0.0);
    let fold = at(-6.0, 0.0);
    // Two wings: one in the accent, the other darker, and the fold line.
    p.add(Shape::convex_polygon(vec![nose, at(-8.0, -7.0), fold], t.accent, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![nose, fold, at(-8.0, 5.0)], t.accent.lerp_to_gamma(t.text, 0.35), Stroke::NONE));
    p.line_segment([nose, fold], Stroke::new(f.len(1.0), alpha(Color32::WHITE, 0.6)));
}

/// A brush dabs three colour blobs onto a sheet, one after the other; then they fade.
fn brush(p: &Painter, f: &Frame, t: &Tokens, time: f32) {
    let ph = phase(time, 4.8);
    let sheet = Rect::from_min_max(f.at(14.0, 8.0), f.at(90.0, 50.0));
    p.rect(sheet, f.len(3.0), t.opaque(t.surface), Stroke::new(f.len(1.2), t.text2), egui::StrokeKind::Inside);
    let fade = 1.0 - ease(span(ph, 0.85, 0.98));
    let blobs = [(32.0, 32.0, 9.0, t.accent, 0.08), (54.0, 22.0, 8.0, t.accent2, 0.3), (72.0, 34.0, 7.5, light(t), 0.52)];
    let mut tip = f.at(24.0, 18.0);
    for (i, (x, y, r, color, at)) in blobs.iter().enumerate() {
        let grow = ease(span(ph, *at + 0.08, *at + 0.2));
        if grow > 0.0 {
            let c = f.at(*x, *y);
            let col = alpha(*color, fade * 0.9);
            p.circle_filled(c, f.len(r * grow), col);
            p.circle_filled(c + vec2(f.len(r * 0.6), f.len(-r * 0.35)) * grow, f.len(r * 0.55 * grow), col);
            p.circle_filled(c + vec2(f.len(-r * 0.55), f.len(r * 0.45)) * grow, f.len(r * 0.45 * grow), col);
        }
        // The brush travels to the blob, touches down while it grows, then moves on.
        let arrive = ease(span(ph, *at, *at + 0.08));
        if ph >= *at {
            let from = if i == 0 { f.at(24.0, 18.0) } else { blobs.get(i - 1).map(|b| f.at(b.0, b.1)).unwrap_or(tip) };
            tip = from.lerp(f.at(*x, *y), arrive);
        }
    }
    // After the last blob the brush goes back to its start.
    let home = ease(span(ph, 0.72, 0.95));
    if home > 0.0 {
        tip = tip.lerp(f.at(24.0, 18.0), home);
    }
    let dab = blobs.iter().map(|b| (span(ph, b.4 + 0.08, b.4 + 0.2) * std::f32::consts::PI).sin()).fold(0.0f32, f32::max);
    let tip = tip - vec2(0.0, f.len(3.0 * (1.0 - dab)));
    let current = blobs.iter().rev().find(|b| ph >= b.4).map(|b| b.3).unwrap_or(t.accent);
    // Handle up and to the right, ferrule, bristles in the current colour.
    let d = vec2(0.85, -0.53);
    let n = vec2(-d.y, d.x);
    let at = |along: f32, side: f32| tip + d * f.len(along) + n * f.len(side);
    p.add(Shape::convex_polygon(vec![at(0.0, 0.0), at(5.0, 2.6), at(8.0, 2.4), at(8.0, -2.4), at(5.0, -2.6)], current, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(8.0, 2.4), at(13.0, 2.0), at(13.0, -2.0), at(8.0, -2.4)], t.muted, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(13.0, 1.8), at(30.0, 1.2), at(31.0, 0.0), at(30.0, -1.2), at(13.0, -1.8)], t.text2, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    /// Paints every animation at many times (also odd ones) and sizes into a real shape list:
    /// nothing panics, and every frame draws something.
    #[test]
    fn every_animation_paints_at_any_time() {
        let ctx = egui::Context::default();
        let layer = egui::LayerId::background();
        let count = |ctx: &egui::Context| ctx.graphics(|g| g.get(layer).map(|l| l.all_entries().len()).unwrap_or(0));
        let mut times: Vec<f32> = (0..600).map(|i| i as f32 * 0.037).collect();
        times.extend([-1.0, 1e9, -1e9, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 3599.99]);
        for dark in [false, true] {
            for look in [crate::theme::Look::Modern, crate::theme::Look::Glas] {
                let t = crate::theme::tokens(look, dark);
                for anim in ALL {
                    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let painter = ui.ctx().layer_painter(layer);
                        let rect = Rect::from_min_size(pos2(10.0, 10.0), SIZE);
                        for &time in &times {
                            let before = count(ui.ctx());
                            paint(&painter, rect, &t, anim, time);
                            assert!(count(ui.ctx()) > before, "{anim:?} draws nothing at {time}");
                        }
                        for size in [vec2(0.0, 0.0), vec2(1.0, 400.0), vec2(30.0, 16.0), vec2(f32::NAN, 5.0), vec2(4000.0, 2000.0)] {
                            paint(&painter, Rect::from_min_size(pos2(0.0, 0.0), size), &t, anim, 1.0);
                        }
                    });
                    out.textures_delta.clear();
                }
            }
        }
    }

    /// Abwechselnd picks per message and keeps it; single choices and Schlicht are honoured;
    /// image chats get the brush; the status line starts with what people know.
    #[test]
    fn choice_is_stable_per_message_and_lines_rotate() {
        assert_eq!(pick("plain", "m1", false), None);
        assert_eq!(pick("books", "m1", true), Some(Anim::Books));
        assert_eq!(pick("mix", "m1", true), Some(Anim::Brush));
        assert_eq!(pick("mix", "m1", false), pick("mix", "m1", false));
        let seen: std::collections::HashSet<_> = (0..200).filter_map(|i| pick("mix", &format!("msg-{i}"), false)).map(|a| a.key()).collect();
        assert_eq!(seen.len(), MIX.len(), "every animation comes up: {seen:?}");
        assert!(!seen.contains("brush"));
        for (key, _) in CHOICES {
            assert!(key == "mix" || key == "plain" || Anim::from_key(key).is_some(), "{key}");
        }
        assert_eq!(status_line(0.0, "m", false), "Denkt nach …");
        assert_eq!(status_line(1.0, "m", true), "Bild wird gezeichnet …");
        assert_ne!(status_line(LINE_SECS + 0.1, "m", false), "Denkt nach …");
        for s in [-5.0, f64::NAN, 1e300, 12.0, 100.0] {
            assert!(status_line(s, "m", false).ends_with('…'));
        }
    }
}
