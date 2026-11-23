//! `BoardView`: the egui canvas for a [`Board`].
//!
//! The view state (camera, tool, selection, clipboard) is plain Rust in [`ViewState`] and never
//! goes into the CRDT. Colours come from a [`BoardStyle`] the app fills from its theme tokens.

use std::collections::{HashMap, HashSet};

use egui::epaint::EllipseShape;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Frame, Id, Key, Margin, Modifiers, Painter, PointerButton, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2, vec2,
};

use crate::model::{Board, Element, Kind, MAX_POINTS, MAX_TEXT, arrow_endpoints, truncate_chars};

/// Names of the colour palette, in swatch order. `ink` (the text colour) comes on top of these.
pub const PALETTE: [&str; 8] = ["yellow", "orange", "red", "pink", "violet", "blue", "green", "gray"];

/// German name of a palette colour.
pub fn color_label(name: &str) -> &'static str {
    match name {
        "yellow" => "Gelb",
        "orange" => "Orange",
        "red" => "Rot",
        "pink" => "Rosa",
        "violet" => "Lila",
        "blue" => "Blau",
        "green" => "Grün",
        "gray" => "Grau",
        _ => "Standard",
    }
}

const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 8.0;
const NOTE_FONT: f32 = 16.0;
const TEXT_FONT: f32 = 20.0;
const SHAPE_FONT: f32 = 16.0;
const FRAME_FONT: f32 = 14.0;
const GRID: f32 = 24.0;
const HANDLE: f32 = 8.0;
const MAX_PEN_POINTS: usize = MAX_POINTS / 2;
const PASTE_TEXT_LIMIT: usize = 4_000;

/// Colours the board needs; the app fills these from its own theme tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardStyle {
    pub bg: Color32,
    pub grid: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub selection: Color32,
    /// Toolbar background.
    pub surface: Color32,
    pub border: Color32,
    /// Text on notes (notes keep their light colours in dark mode).
    pub note_text: Color32,
    /// Note colours, in the order of [`PALETTE`].
    pub palette: [Color32; 8],
}

impl BoardStyle {
    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xfa, 0xf9, 0xf7),
            grid: Color32::from_rgb(0xd6, 0xd1, 0xc8),
            text: Color32::from_rgb(0x1f, 0x1d, 0x1a),
            muted: Color32::from_rgb(0x6f, 0x6a, 0x62),
            accent: Color32::from_rgb(0xc9, 0x52, 0x1c),
            selection: Color32::from_rgb(0xf6, 0xd9, 0xc8),
            surface: Color32::WHITE,
            border: Color32::from_rgb(0xde, 0xda, 0xd2),
            note_text: Color32::from_rgb(0x1f, 0x1d, 0x1a),
            palette: Self::PASTELS,
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x1e, 0x1d, 0x1b),
            grid: Color32::from_rgb(0x45, 0x41, 0x3b),
            text: Color32::from_rgb(0xf3, 0xf0, 0xea),
            muted: Color32::from_rgb(0xa3, 0x9e, 0x94),
            accent: Color32::from_rgb(0xe0, 0x7a, 0x4a),
            selection: Color32::from_rgb(0x5a, 0x34, 0x22),
            surface: Color32::from_rgb(0x28, 0x26, 0x23),
            border: Color32::from_rgb(0x3d, 0x3a, 0x35),
            note_text: Color32::from_rgb(0x1f, 0x1d, 0x1a),
            palette: Self::PASTELS,
        }
    }

    const PASTELS: [Color32; 8] = [
        Color32::from_rgb(0xfd, 0xe6, 0x8a),
        Color32::from_rgb(0xfd, 0xc9, 0x96),
        Color32::from_rgb(0xf8, 0xa9, 0xa0),
        Color32::from_rgb(0xf6, 0xb8, 0xd4),
        Color32::from_rgb(0xcf, 0xbd, 0xf5),
        Color32::from_rgb(0xa9, 0xcf, 0xf5),
        Color32::from_rgb(0xb4, 0xe2, 0xb0),
        Color32::from_rgb(0xdc, 0xd8, 0xd0),
    ];

    /// Fill colour for a palette name (`ink` and unknown names: the text colour).
    pub fn fill(&self, name: &str) -> Color32 {
        PALETTE.iter().position(|p| *p == name).and_then(|i| self.palette.get(i).copied()).unwrap_or(self.text)
    }

    /// Stroke colour: the palette colour pulled toward the text colour so it reads on the background.
    pub fn ink(&self, name: &str) -> Color32 {
        match PALETTE.iter().position(|p| *p == name).and_then(|i| self.palette.get(i).copied()) {
            Some(c) => mix(c, self.text, 0.42),
            None => self.text,
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Canvas tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    #[default]
    Select,
    Hand,
    Note,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Text,
    Pen,
    Eraser,
    Frame,
}

impl Tool {
    pub const ALL: [Tool; 11] =
        [Tool::Select, Tool::Hand, Tool::Note, Tool::Rect, Tool::Ellipse, Tool::Arrow, Tool::Line, Tool::Text, Tool::Pen, Tool::Eraser, Tool::Frame];

    pub fn key(self) -> Key {
        match self {
            Tool::Select => Key::V,
            Tool::Hand => Key::H,
            Tool::Note => Key::N,
            Tool::Rect => Key::R,
            Tool::Ellipse => Key::O,
            Tool::Arrow => Key::A,
            Tool::Line => Key::L,
            Tool::Text => Key::T,
            Tool::Pen => Key::P,
            Tool::Eraser => Key::E,
            Tool::Frame => Key::F,
        }
    }

    /// German tooltip with the shortcut.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Auswählen (V)",
            Tool::Hand => "Ansicht verschieben (H)",
            Tool::Note => "Notiz (N)",
            Tool::Rect => "Rechteck (R)",
            Tool::Ellipse => "Ellipse (O)",
            Tool::Arrow => "Pfeil (A)",
            Tool::Line => "Linie (L)",
            Tool::Text => "Text (T)",
            Tool::Pen => "Stift (P)",
            Tool::Eraser => "Radierer (E)",
            Tool::Frame => "Rahmen (F)",
        }
    }

    fn creates(self) -> Option<Kind> {
        match self {
            Tool::Note => Some(Kind::Note),
            Tool::Rect => Some(Kind::Rect),
            Tool::Ellipse => Some(Kind::Ellipse),
            Tool::Arrow => Some(Kind::Arrow),
            Tool::Line => Some(Kind::Line),
            Tool::Text => Some(Kind::Text),
            Tool::Frame => Some(Kind::Frame),
            Tool::Pen => Some(Kind::Freehand),
            Tool::Select | Tool::Hand | Tool::Eraser => None,
        }
    }
}

/// World point shown at the canvas centre, and the zoom factor (screen points per world unit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub center: Pos2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { center: Pos2::ZERO, zoom: 1.0 }
    }
}

impl Camera {
    pub fn to_screen(&self, canvas: Rect, world: Pos2) -> Pos2 {
        canvas.center() + (world - self.center) * self.zoom
    }

    pub fn to_world(&self, canvas: Rect, screen: Pos2) -> Pos2 {
        self.center + (screen - canvas.center()) / self.zoom
    }

    pub fn rect_to_screen(&self, canvas: Rect, world: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(canvas, world.min), self.to_screen(canvas, world.max))
    }

    /// The world area visible in `canvas`.
    pub fn visible(&self, canvas: Rect) -> Rect {
        Rect::from_min_max(self.to_world(canvas, canvas.min), self.to_world(canvas, canvas.max))
    }

    /// Zoom by `factor`, keeping the world point under `screen` in place.
    pub fn zoom_at(&mut self, canvas: Rect, screen: Pos2, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let before = self.to_world(canvas, screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(canvas, screen);
        self.center += before - after;
    }

    fn sanitize(&mut self) {
        if !self.zoom.is_finite() {
            self.zoom = 1.0;
        }
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let lim = crate::model::COORD_LIMIT as f32;
        if !self.center.x.is_finite() || !self.center.y.is_finite() {
            self.center = Pos2::ZERO;
        }
        self.center = self.center.clamp(pos2(-lim, -lim), pos2(lim, lim));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handle {
    hx: i8,
    hy: i8,
}

#[derive(Debug, Clone, Default)]
enum Interaction {
    #[default]
    None,
    Pan,
    Move {
        start: Pos2,
        orig: Vec<(String, f64, f64)>,
        last: Vec2,
    },
    Resize {
        handle: Handle,
        bounds: Rect,
        orig: Vec<Element>,
    },
    Marquee {
        start: Pos2,
        base: Vec<String>,
    },
    Create {
        kind: Kind,
        start: Pos2,
    },
    Pen {
        points: Vec<Pos2>,
    },
    Erase,
}

#[derive(Debug, Clone)]
struct Editing {
    id: String,
    text: String,
    focused: bool,
}

#[derive(Debug, Default)]
struct Cache {
    key: Option<(u64, u64)>,
    /// Draw order: frames first, then everything else by z.
    els: Vec<Element>,
    index: HashMap<String, usize>,
}

impl Cache {
    fn refresh(&mut self, board: &Board) {
        let key = (board.instance(), board.revision());
        if self.key == Some(key) {
            return;
        }
        let mut els = board.elements();
        els.sort_by_key(|e| e.kind != Kind::Frame);
        self.index = els.iter().enumerate().map(|(i, e)| (e.id.clone(), i)).collect();
        self.els = els;
        self.key = Some(key);
    }

    fn get(&self, id: &str) -> Option<&Element> {
        self.index.get(id).and_then(|&i| self.els.get(i))
    }

    fn endpoints(&self, el: &Element) -> (Pos2, Pos2) {
        let (a, b) = arrow_endpoints(el, |id| self.get(id));
        (p(a), p(b))
    }

    /// World bounds (arrows follow their bound elements).
    fn bounds(&self, el: &Element) -> Rect {
        if el.kind == Kind::Arrow {
            let (a, b) = self.endpoints(el);
            Rect::from_two_pos(a, b)
        } else {
            Rect::from_min_size(pos2(el.x as f32, el.y as f32), vec2(el.w as f32, el.h as f32))
        }
    }
}

fn p((x, y): (f64, f64)) -> Pos2 {
    pos2(x as f32, y as f32)
}

/// Everything about the view that is not shared: camera, tool, selection, clipboard.
#[derive(Debug)]
pub struct ViewState {
    pub camera: Camera,
    pub tool: Tool,
    pub selection: Vec<String>,
    /// Colour for new elements; `None` = each kind's default.
    pub color: Option<String>,
    /// Show the floating tool and colour bars.
    pub toolbar: bool,
    fit_pending: bool,
    interaction: Interaction,
    editing: Option<Editing>,
    clipboard: Vec<Element>,
    clipboard_text: String,
    paste_count: u32,
    cache: Cache,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            tool: Tool::Select,
            selection: Vec::new(),
            color: None,
            toolbar: true,
            fit_pending: true,
            interaction: Interaction::None,
            editing: None,
            clipboard: Vec::new(),
            clipboard_text: String::new(),
            paste_count: 0,
            cache: Cache::default(),
        }
    }
}

impl ViewState {
    /// Fit all elements into view on the next frame.
    pub fn zoom_to_fit(&mut self) {
        self.fit_pending = true;
    }

    /// Id of the element whose text is being edited.
    pub fn editing(&self) -> Option<&str> {
        self.editing.as_ref().map(|e| e.id.as_str())
    }

    /// Start editing an element's text in place (next frame).
    pub fn edit(&mut self, id: &str) {
        self.editing = Some(Editing { id: id.to_string(), text: String::new(), focused: false });
        self.selection = vec![id.to_string()];
    }

    fn is_selected(&self, id: &str) -> bool {
        self.selection.iter().any(|s| s == id)
    }
}

/// The whiteboard widget: `BoardView::new(&mut board, &mut view, &style).show(ui)`.
pub struct BoardView<'a> {
    board: &'a mut Board,
    st: &'a mut ViewState,
    style: &'a BoardStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Tool(Tool),
    Undo,
    Redo,
    Fit,
    Duplicate,
    Delete,
    Front,
    Back,
}

impl<'a> BoardView<'a> {
    pub fn new(board: &'a mut Board, state: &'a mut ViewState, style: &'a BoardStyle) -> Self {
        Self { board, st: state, style }
    }

    /// Fill the available space with the canvas.
    pub fn show(self, ui: &mut Ui) -> Response {
        let size = ui.available_size().max(vec2(64.0, 64.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let BoardView { board, st, style } = self;
        st.camera.sanitize();
        st.cache.refresh(board);
        st.selection.retain(|id| st.cache.index.contains_key(id));
        st.selection.dedup();
        if st.editing.as_ref().is_some_and(|e| !st.cache.index.contains_key(&e.id)) {
            st.editing = None;
        }
        if st.fit_pending {
            fit(st, rect);
            st.fit_pending = false;
        }
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }

        navigate(ui, &resp, rect, st);
        pointer(ui, &resp, rect, board, st);
        keyboard(ui, &resp, rect, board, st);
        st.cache.refresh(board);
        cursor(ui, &resp, st);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, style.bg);
        grid(&painter, rect, &st.camera, style);
        paint_elements(&painter, ui, rect, st, style);
        paint_overlay(&painter, ui, &resp, rect, st, style);
        editor(ui, resp.id, rect, board, st, style);
        if st.toolbar {
            toolbar(ui, &painter, rect, resp.id, board, st, style);
        }
        if st.cache.els.is_empty() && st.editing.is_none() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Doppelklick für eine Notiz – oder ein Werkzeug oben wählen",
                FontId::proportional(15.0),
                style.muted,
            );
        }
        resp
    }
}

// ----- camera ---------------------------------------------------------------------------------

fn fit(st: &mut ViewState, rect: Rect) {
    let mut b = Rect::NOTHING;
    for e in &st.cache.els {
        b = b.union(st.cache.bounds(e));
    }
    if !b.min.x.is_finite() || !b.max.x.is_finite() {
        st.camera = Camera::default();
        return;
    }
    let margin = 48.0;
    let top = if st.toolbar { 64.0 } else { 0.0 };
    // Room for the tool bar on top and the colour bar at the bottom.
    let avail = vec2((rect.width() - 2.0 * margin).max(32.0), (rect.height() - 2.0 * margin - 2.0 * top).max(32.0));
    let zoom = (avail.x / b.width().max(1.0)).min(avail.y / b.height().max(1.0)).clamp(MIN_ZOOM, 1.5);
    st.camera = Camera { center: b.center(), zoom };
}

fn navigate(ui: &mut Ui, resp: &Response, rect: Rect, st: &mut ViewState) {
    if resp.hovered() || resp.contains_pointer() {
        let (scroll, zoom, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if (zoom - 1.0).abs() > 1e-4 {
            if let Some(h) = hover {
                st.camera.zoom_at(rect, h, zoom);
            }
        } else if scroll != Vec2::ZERO {
            st.camera.center -= scroll / st.camera.zoom;
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    if resp.dragged_by(PointerButton::Middle) || matches!(st.interaction, Interaction::Pan) && resp.dragged_by(PointerButton::Primary) {
        st.camera.center -= resp.drag_delta() / st.camera.zoom;
    }
}

fn cursor(ui: &Ui, resp: &Response, st: &ViewState) {
    if !resp.hovered() {
        return;
    }
    let space = ui.input(|i| i.key_down(Key::Space));
    let icon = match (&st.interaction, st.tool) {
        (Interaction::Pan, _) => CursorIcon::Grabbing,
        _ if space => CursorIcon::Grab,
        (_, Tool::Hand) => CursorIcon::Grab,
        (Interaction::Move { .. }, _) => CursorIcon::Grabbing,
        (_, Tool::Select) => CursorIcon::Default,
        (_, Tool::Text) => CursorIcon::Text,
        _ => CursorIcon::Crosshair,
    };
    ui.ctx().set_cursor_icon(icon);
}

// ----- hit testing ----------------------------------------------------------------------------

fn seg_dist(pt: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 {
        return pt.distance(a);
    }
    let t = ((pt - a).dot(ab) / len2).clamp(0.0, 1.0);
    pt.distance(a + ab * t)
}

fn hits(cache: &Cache, el: &Element, w: Pos2, tol: f32) -> bool {
    let b = cache.bounds(el);
    match el.kind {
        Kind::Note | Kind::Rect | Kind::Text => b.expand(tol).contains(w),
        Kind::Ellipse => {
            let c = b.center();
            let (rx, ry) = (b.width() / 2.0 + tol, b.height() / 2.0 + tol);
            rx > 0.0 && ry > 0.0 && ((w.x - c.x) / rx).powi(2) + ((w.y - c.y) / ry).powi(2) <= 1.0
        }
        Kind::Frame => {
            let label = Rect::from_min_size(b.min - vec2(0.0, 24.0), vec2(b.width().min(260.0), 24.0));
            label.contains(w) || (b.expand(tol).contains(w) && !b.shrink(tol).contains(w))
        }
        Kind::Arrow => {
            let (a, c) = cache.endpoints(el);
            seg_dist(w, a, c) <= tol + 2.0
        }
        Kind::Line | Kind::Freehand => {
            if !b.expand(tol + 4.0).contains(w) {
                return false;
            }
            let pts: Vec<Pos2> = el.world_points().map(p).collect();
            pts.windows(2).any(|s| matches!(s, [a, c] if seg_dist(w, *a, *c) <= tol + 2.0))
                || (pts.len() == 1 && pts.first().is_some_and(|a| a.distance(w) <= tol + 2.0))
        }
    }
}

/// Topmost element at a world point.
fn hit_test(cache: &Cache, w: Pos2, zoom: f32) -> Option<&Element> {
    let tol = 6.0 / zoom;
    cache.els.iter().rev().find(|e| hits(cache, e, w, tol))
}

fn selection_bounds(st: &ViewState) -> Option<Rect> {
    let mut b = Rect::NOTHING;
    for id in &st.selection {
        if let Some(e) = st.cache.get(id) {
            b = b.union(st.cache.bounds(e));
        }
    }
    b.min.x.is_finite().then_some(b)
}

/// Whether the selection can be resized with handles (bound arrows only follow).
fn resizable(st: &ViewState) -> bool {
    st.selection.iter().filter_map(|id| st.cache.get(id)).any(|e| e.kind != Kind::Arrow)
}

fn handles(screen: Rect) -> [(Handle, Pos2); 8] {
    let (l, c, r) = (screen.min.x, screen.center().x, screen.max.x);
    let (t, m, b) = (screen.min.y, screen.center().y, screen.max.y);
    [
        (Handle { hx: -1, hy: -1 }, pos2(l, t)),
        (Handle { hx: 0, hy: -1 }, pos2(c, t)),
        (Handle { hx: 1, hy: -1 }, pos2(r, t)),
        (Handle { hx: 1, hy: 0 }, pos2(r, m)),
        (Handle { hx: 1, hy: 1 }, pos2(r, b)),
        (Handle { hx: 0, hy: 1 }, pos2(c, b)),
        (Handle { hx: -1, hy: 1 }, pos2(l, b)),
        (Handle { hx: -1, hy: 0 }, pos2(l, m)),
    ]
}

fn handle_at(st: &ViewState, rect: Rect, screen: Pos2) -> Option<Handle> {
    if !resizable(st) {
        return None;
    }
    let b = st.camera.rect_to_screen(rect, selection_bounds(st)?).expand(4.0);
    handles(b).into_iter().find(|(_, hp)| (*hp - screen).abs().max_elem() <= HANDLE).map(|(h, _)| h)
}

// ----- pointer --------------------------------------------------------------------------------

fn new_color(st: &ViewState, kind: Kind) -> String {
    match (&st.color, kind) {
        // Notes always get a note colour; ink is for strokes and text.
        (Some(c), Kind::Note) if c == "ink" => Kind::Note.default_color().into(),
        (Some(c), Kind::Frame) if c == "ink" => Kind::Frame.default_color().into(),
        (Some(c), _) => c.clone(),
        (None, k) => k.default_color().into(),
    }
}

/// Create an element of `kind` at a world point (click) and select it.
fn create_at(board: &mut Board, st: &mut ViewState, kind: Kind, w: Pos2) {
    let (x, y) = (w.x as f64, w.y as f64);
    let color = new_color(st, kind);
    let el = match kind {
        Kind::Arrow | Kind::Line => Element::segment(kind, (x, y), (x + 160.0, y)).with_color(color),
        Kind::Text => Element::new(kind, x, y - 14.0).with_color(color),
        Kind::Freehand => return,
        _ => {
            let e = Element::new(kind, 0.0, 0.0);
            Element::new(kind, x - e.w / 2.0, y - e.h / 2.0).with_color(color)
        }
    };
    let id = board.add(el);
    st.selection = vec![id.clone()];
    if matches!(kind, Kind::Note | Kind::Text) {
        st.edit(&id);
    }
    st.tool = Tool::Select;
}

fn pointer(ui: &mut Ui, resp: &Response, rect: Rect, board: &mut Board, st: &mut ViewState) {
    let cam = st.camera;
    let shift = ui.input(|i| i.modifiers.shift);
    let space = st.editing.is_none() && ui.input(|i| i.key_down(Key::Space));
    let latest = ui.input(|i| i.pointer.latest_pos());
    let pos = resp.interact_pointer_pos().or(latest).unwrap_or(rect.center());
    let w = cam.to_world(rect, pos);

    if resp.drag_started_by(PointerButton::Primary) {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let ow = cam.to_world(rect, origin);
        st.interaction = if space || st.tool == Tool::Hand {
            Interaction::Pan
        } else {
            match st.tool {
                Tool::Select => start_select_drag(board, st, rect, origin, ow, shift),
                Tool::Eraser => {
                    board.checkpoint();
                    Interaction::Erase
                }
                Tool::Pen => Interaction::Pen { points: vec![ow] },
                t => match t.creates() {
                    Some(kind) => Interaction::Create { kind, start: ow },
                    None => Interaction::None,
                },
            }
        };
    }

    if resp.dragged_by(PointerButton::Primary) {
        drag_update(board, st, w, shift);
    }

    if resp.drag_stopped() {
        let it = std::mem::take(&mut st.interaction);
        match it {
            Interaction::Create { kind, start } => finish_create(board, st, kind, start, w),
            Interaction::Pen { mut points } => {
                points.push(w);
                let pts: Vec<(f64, f64)> = points.iter().map(|q| (q.x as f64, q.y as f64)).collect();
                if pts.len() >= 2 {
                    let el = Element::path(Kind::Freehand, &pts).with_color(new_color(st, Kind::Freehand));
                    board.add(el);
                }
            }
            Interaction::Move { .. } | Interaction::Resize { .. } => board.checkpoint(),
            _ => {}
        }
    }

    if resp.clicked() && !space {
        match st.tool {
            Tool::Select => {
                let hit = hit_test(&st.cache, w, cam.zoom).map(|e| e.id.clone());
                match hit {
                    Some(id) if shift => {
                        if st.is_selected(&id) {
                            st.selection.retain(|s| *s != id);
                        } else {
                            st.selection.push(id);
                        }
                    }
                    Some(id) => st.selection = vec![id],
                    None if !shift => st.selection.clear(),
                    None => {}
                }
            }
            Tool::Eraser => {
                if let Some(id) = hit_test(&st.cache, w, cam.zoom).map(|e| e.id.clone()) {
                    board.delete(&[id]);
                }
            }
            Tool::Hand | Tool::Pen => {}
            t => {
                if let Some(kind) = t.creates() {
                    create_at(board, st, kind, w);
                }
            }
        }
    }

    if resp.double_clicked() && st.tool == Tool::Select && !space {
        match hit_test(&st.cache, w, cam.zoom).map(|e| (e.id.clone(), e.kind)) {
            Some((id, kind)) if kind.has_text() => st.edit(&id),
            Some(_) => {}
            None => create_at(board, st, Kind::Note, w),
        }
    }
}

fn start_select_drag(board: &mut Board, st: &mut ViewState, rect: Rect, origin: Pos2, ow: Pos2, shift: bool) -> Interaction {
    if let Some(handle) = handle_at(st, rect, origin)
        && let Some(bounds) = selection_bounds(st)
    {
        board.checkpoint();
        let orig = st.selection.iter().filter_map(|id| st.cache.get(id).cloned()).collect();
        return Interaction::Resize { handle, bounds, orig };
    }
    match hit_test(&st.cache, ow, st.camera.zoom).map(|e| e.id.clone()) {
        Some(id) => {
            if shift {
                if st.is_selected(&id) {
                    st.selection.retain(|s| *s != id);
                    return Interaction::None;
                }
                st.selection.push(id);
            } else if !st.is_selected(&id) {
                st.selection = vec![id];
            }
            board.checkpoint();
            let orig = st.selection.iter().filter_map(|id| st.cache.get(id)).map(|e| (e.id.clone(), e.x, e.y)).collect();
            Interaction::Move { start: ow, orig, last: Vec2::ZERO }
        }
        None => {
            if !shift {
                st.selection.clear();
            }
            Interaction::Marquee { start: ow, base: st.selection.clone() }
        }
    }
}

fn drag_update(board: &mut Board, st: &mut ViewState, w: Pos2, shift: bool) {
    let zoom = st.camera.zoom;
    match &mut st.interaction {
        Interaction::Move { start, orig, last } => {
            let d = w - *start;
            if d != *last {
                *last = d;
                for (id, x, y) in orig.iter() {
                    board.move_to(id, x + d.x as f64, y + d.y as f64);
                }
            }
        }
        Interaction::Resize { handle, bounds, orig } => {
            let ob = *bounds;
            let min = 8.0;
            let mut nb = ob;
            match handle.hx {
                -1 => nb.min.x = w.x.min(ob.max.x - min),
                1 => nb.max.x = w.x.max(ob.min.x + min),
                _ => {}
            }
            match handle.hy {
                -1 => nb.min.y = w.y.min(ob.max.y - min),
                1 => nb.max.y = w.y.max(ob.min.y + min),
                _ => {}
            }
            if shift && handle.hx != 0 && handle.hy != 0 && ob.width() > 0.0 && ob.height() > 0.0 {
                let s = (nb.width() / ob.width()).max(nb.height() / ob.height());
                let size = ob.size() * s;
                if handle.hx < 0 {
                    nb.min.x = nb.max.x - size.x;
                } else {
                    nb.max.x = nb.min.x + size.x;
                }
                if handle.hy < 0 {
                    nb.min.y = nb.max.y - size.y;
                } else {
                    nb.max.y = nb.min.y + size.y;
                }
            }
            let sx = if ob.width() > 1e-3 { (nb.width() / ob.width()) as f64 } else { 1.0 };
            let sy = if ob.height() > 1e-3 { (nb.height() / ob.height()) as f64 } else { 1.0 };
            for e in orig.iter() {
                if e.kind == Kind::Arrow && (e.from.is_some() || e.to.is_some()) {
                    continue;
                }
                let nx = nb.min.x as f64 + (e.x - ob.min.x as f64) * sx;
                let ny = nb.min.y as f64 + (e.y - ob.min.y as f64) * sy;
                board.resize(&e.id, nx, ny, e.w * sx, e.h * sy);
            }
        }
        Interaction::Marquee { start, base } => {
            let area = Rect::from_two_pos(*start, w);
            let mut sel = base.clone();
            let seen: HashSet<String> = sel.iter().cloned().collect();
            for e in &st.cache.els {
                if !seen.contains(&e.id) && area.contains_rect(st.cache.bounds(e)) {
                    sel.push(e.id.clone());
                }
            }
            st.selection = sel;
        }
        Interaction::Pen { points } => {
            if points.len() < MAX_PEN_POINTS && points.last().is_none_or(|l| l.distance(w) * zoom >= 1.5) {
                points.push(w);
            }
        }
        Interaction::Erase => {
            if let Some(id) = hit_test(&st.cache, w, zoom).map(|e| e.id.clone()) {
                board.remove(&[id]);
                st.cache.refresh(board);
            }
        }
        Interaction::None | Interaction::Pan | Interaction::Create { .. } => {}
    }
}

fn finish_create(board: &mut Board, st: &mut ViewState, kind: Kind, start: Pos2, end: Pos2) {
    let zoom = st.camera.zoom;
    if start.distance(end) * zoom < 4.0 {
        create_at(board, st, kind, start);
        return;
    }
    let color = new_color(st, kind);
    let el = match kind {
        Kind::Arrow | Kind::Line => {
            let mut e = Element::segment(kind, (start.x as f64, start.y as f64), (end.x as f64, end.y as f64)).with_color(color);
            if kind == Kind::Arrow {
                let bindable = |w: Pos2| hit_test(&st.cache, w, zoom).filter(|e| !e.kind.is_path()).map(|e| e.id.clone());
                e.from = bindable(start);
                e.to = bindable(end).filter(|t| Some(t) != e.from.as_ref());
            }
            e
        }
        _ => {
            let r = Rect::from_two_pos(start, end);
            Element::new(kind, r.min.x as f64, r.min.y as f64).with_size(r.width() as f64, r.height() as f64).with_color(color)
        }
    };
    let id = board.add(el);
    st.selection = vec![id.clone()];
    if matches!(kind, Kind::Note | Kind::Text) {
        st.edit(&id);
    }
    st.tool = Tool::Select;
}

// ----- keyboard / clipboard -------------------------------------------------------------------

fn keyboard(ui: &mut Ui, resp: &Response, rect: Rect, board: &mut Board, st: &mut ViewState) {
    if st.editing.is_some() {
        return;
    }
    let other_focus = ui.memory(|m| m.focused().is_some_and(|f| f != resp.id));
    if other_focus || !(resp.has_focus() || resp.contains_pointer()) {
        return;
    }
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let (redo, undo, dup, all, del, esc, enter, fit_key, reset_zoom) = ui.input_mut(|i| {
        let redo = i.consume_key(cmd_shift, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
        let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
        let dup = i.consume_key(Modifiers::COMMAND, Key::D);
        let all = i.consume_key(Modifiers::COMMAND, Key::A);
        let del = i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace);
        let esc = i.consume_key(Modifiers::NONE, Key::Escape);
        let enter = i.consume_key(Modifiers::NONE, Key::Enter);
        let fit = i.consume_key(Modifiers::SHIFT, Key::Num1) || i.consume_key(Modifiers::SHIFT, Key::Exclamationmark);
        let reset = i.consume_key(Modifiers::COMMAND, Key::Num0);
        (redo, undo, dup, all, del, esc, enter, fit, reset)
    });
    if redo {
        board.redo();
    } else if undo {
        board.undo();
    }
    if dup {
        let els = selected_elements(st);
        paste(board, st, els, Some(vec2(24.0, 24.0)));
    }
    if all {
        st.selection = st.cache.els.iter().map(|e| e.id.clone()).collect();
    }
    if del && !st.selection.is_empty() {
        let ids = std::mem::take(&mut st.selection);
        board.delete(&ids);
    }
    if esc {
        st.selection.clear();
        st.tool = Tool::Select;
        st.interaction = Interaction::None;
    }
    if enter
        && let [id] = st.selection.as_slice()
        && st.cache.get(id).is_some_and(|e| e.kind.has_text())
    {
        let id = id.clone();
        st.edit(&id);
    }
    if fit_key {
        fit(st, rect);
    }
    if reset_zoom {
        st.camera.zoom = 1.0;
    }
    // Nudge with the arrow keys.
    let nudge = ui.input_mut(|i| {
        let step = if i.modifiers.shift { 10.0 } else { 1.0 };
        let mut d = Vec2::ZERO;
        for (k, v) in [(Key::ArrowLeft, vec2(-1.0, 0.0)), (Key::ArrowRight, vec2(1.0, 0.0)), (Key::ArrowUp, vec2(0.0, -1.0)), (Key::ArrowDown, vec2(0.0, 1.0))]
        {
            if i.consume_key(Modifiers::NONE, k) || i.consume_key(Modifiers::SHIFT, k) {
                d += v * step;
            }
        }
        d
    });
    if nudge != Vec2::ZERO && !st.selection.is_empty() {
        board.checkpoint();
        board.move_by(&st.selection, nudge.x as f64, nudge.y as f64);
    }
    // Tool shortcuts: plain letters only.
    let tool = ui.input_mut(|i| Tool::ALL.into_iter().find(|t| i.consume_key(Modifiers::NONE, t.key())));
    if let Some(t) = tool {
        st.tool = t;
    }
    // Clipboard: ctrl+C/X/V arrive as events (the platform clipboard), ctrl+V also as a key when
    // no clipboard is available.
    let events: Vec<egui::Event> = ui.input(|i| i.events.clone());
    let mut pasted = false;
    for ev in events {
        match ev {
            egui::Event::Copy | egui::Event::Cut => {
                let els = selected_elements(st);
                if els.is_empty() {
                    continue;
                }
                let text = copy_text(&els);
                ui.ctx().copy_text(text.clone());
                st.clipboard_text = text;
                st.clipboard = els;
                st.paste_count = 0;
                if matches!(ev, egui::Event::Cut) {
                    let ids = std::mem::take(&mut st.selection);
                    board.delete(&ids);
                }
            }
            egui::Event::Paste(text) if !pasted => {
                pasted = true;
                if !st.clipboard.is_empty() && text.trim() == st.clipboard_text.trim() {
                    paste_clipboard(ui, rect, board, st);
                } else if !text.trim().is_empty() {
                    let mut t = text.trim().to_string();
                    truncate_chars(&mut t, PASTE_TEXT_LIMIT.min(MAX_TEXT));
                    let at = ui.input(|i| i.pointer.hover_pos()).filter(|h| rect.contains(*h)).unwrap_or(rect.center());
                    let w = st.camera.to_world(rect, at);
                    let el = Element::new(Kind::Note, w.x as f64 - 100.0, w.y as f64 - 70.0).with_text(t).with_color(new_color(st, Kind::Note));
                    let id = board.add(el);
                    st.selection = vec![id];
                }
            }
            egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command && !pasted => {
                pasted = true;
                paste_clipboard(ui, rect, board, st);
            }
            _ => {}
        }
    }
}

fn selected_elements(st: &ViewState) -> Vec<Element> {
    // Keep draw order so pasted copies stack the same way.
    let sel: HashSet<&str> = st.selection.iter().map(String::as_str).collect();
    st.cache.els.iter().filter(|e| sel.contains(e.id.as_str())).cloned().collect()
}

fn copy_text(els: &[Element]) -> String {
    let texts: Vec<&str> = els.iter().map(|e| e.text.trim()).filter(|t| !t.is_empty()).collect();
    if texts.is_empty() { format!("Whiteboard-Auswahl ({} Elemente)", els.len()) } else { texts.join("\n\n") }
}

fn paste_clipboard(ui: &Ui, rect: Rect, board: &mut Board, st: &mut ViewState) {
    if st.clipboard.is_empty() {
        return;
    }
    let els = st.clipboard.clone();
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|h| rect.contains(*h));
    let offset = match hover {
        Some(h) => {
            let mut b = Rect::NOTHING;
            for e in &els {
                b = b.union(Rect::from_min_size(pos2(e.x as f32, e.y as f32), vec2(e.w as f32, e.h as f32)));
            }
            let target = st.camera.to_world(rect, h);
            if b.min.x.is_finite() { target - b.center() } else { Vec2::ZERO }
        }
        None => {
            st.paste_count = st.paste_count.saturating_add(1);
            Vec2::splat(24.0 * st.paste_count.min(40) as f32)
        }
    };
    paste(board, st, els, Some(offset));
}

/// Insert copies of `els` shifted by `offset`; arrows between copied elements stay bound to the copies.
fn paste(board: &mut Board, st: &mut ViewState, els: Vec<Element>, offset: Option<Vec2>) {
    if els.is_empty() {
        return;
    }
    let d = offset.unwrap_or(Vec2::ZERO);
    let copies: Vec<Element> = els
        .into_iter()
        .map(|mut e| {
            e.x += d.x as f64;
            e.y += d.y as f64;
            // Fresh ids are assigned by add_many; keep the old ones so references are remapped.
            e.id = format!("{}~", e.id);
            e
        })
        .collect();
    let wanted: HashSet<String> = copies.iter().map(|e| e.id.clone()).collect();
    let copies: Vec<Element> = copies
        .into_iter()
        .map(|mut e| {
            for r in [&mut e.from, &mut e.to] {
                if let Some(old) = r.as_ref() {
                    let tilde = format!("{old}~");
                    if wanted.contains(&tilde) {
                        *r = Some(tilde);
                    }
                }
            }
            e
        })
        .collect();
    st.selection = board.add_many(copies);
}

// ----- painting -------------------------------------------------------------------------------

fn grid(painter: &Painter, rect: Rect, cam: &Camera, style: &BoardStyle) {
    let mut step = GRID;
    for _ in 0..24 {
        if step * cam.zoom >= 14.0 {
            break;
        }
        step *= 2.0;
    }
    let vis = cam.visible(rect);
    let x0 = (vis.min.x / step).floor() as i64;
    let x1 = (vis.max.x / step).ceil() as i64;
    let y0 = (vis.min.y / step).floor() as i64;
    let y1 = (vis.max.y / step).ceil() as i64;
    if (x1 - x0).saturating_mul(y1 - y0) > 40_000 {
        return;
    }
    let r = (1.1 * cam.zoom.sqrt()).clamp(0.8, 1.6);
    for gx in x0..=x1 {
        for gy in y0..=y1 {
            let s = cam.to_screen(rect, pos2(gx as f32 * step, gy as f32 * step));
            painter.circle_filled(s, r, style.grid);
        }
    }
}

fn paint_text(painter: &Painter, ui: &Ui, text: &str, area: Rect, size: f32, color: Color32, center: bool) {
    if size < 4.0 || text.is_empty() || area.width() < 4.0 {
        return;
    }
    let galley = ui.fonts_mut(|f| f.layout(text.to_string(), FontId::proportional(size), color, area.width().max(1.0)));
    let gs = galley.size();
    let pos = if center {
        pos2(area.center().x - gs.x / 2.0, (area.center().y - gs.y / 2.0).max(area.min.y))
    } else if gs.y < area.height() {
        pos2(area.min.x, area.center().y - gs.y / 2.0)
    } else {
        area.min
    };
    painter.with_clip_rect(area.expand(2.0).intersect(painter.clip_rect())).galley(pos, galley, color);
}

fn arrow_head(painter: &Painter, from: Pos2, tip: Pos2, size: f32, color: Color32) {
    let dir = (tip - from).normalized();
    if !dir.x.is_finite() || !dir.y.is_finite() || dir == Vec2::ZERO {
        return;
    }
    let n = vec2(-dir.y, dir.x);
    let base = tip - dir * size;
    painter.add(Shape::convex_polygon(vec![tip, base + n * size * 0.5, base - n * size * 0.5], color, Stroke::NONE));
}

fn paint_elements(painter: &Painter, ui: &Ui, rect: Rect, st: &ViewState, style: &BoardStyle) {
    let cam = st.camera;
    let z = cam.zoom;
    let vis = cam.visible(rect).expand(40.0 / z);
    let editing = st.editing.as_ref().map(|e| e.id.as_str());
    for el in &st.cache.els {
        let wb = st.cache.bounds(el);
        let label_room = if el.kind == Kind::Frame { Rect::from_min_size(wb.min - vec2(0.0, 24.0), vec2(wb.width(), 24.0)) } else { wb };
        if !vis.intersects(wb.union(label_room)) {
            continue;
        }
        let r = cam.rect_to_screen(rect, wb);
        let hide_text = editing == Some(el.id.as_str());
        let sw = (2.0 * z).max(1.0);
        match el.kind {
            Kind::Note => {
                let cr = CornerRadius::same((6.0 * z).clamp(0.0, 40.0) as u8);
                painter.rect_filled(r.translate(vec2(0.0, (3.0 * z).max(1.0))), cr, Color32::from_black_alpha(26));
                painter.rect_filled(r, cr, style.fill(&el.color));
                if !hide_text {
                    paint_text(painter, ui, &el.text, r.shrink(12.0 * z), NOTE_FONT * z, style.note_text, false);
                }
            }
            Kind::Rect | Kind::Ellipse => {
                let ink = style.ink(&el.color);
                let fill = if el.color == "ink" { Color32::TRANSPARENT } else { style.fill(&el.color).gamma_multiply(0.35) };
                if el.kind == Kind::Rect {
                    let cr = CornerRadius::same((4.0 * z).clamp(0.0, 30.0) as u8);
                    painter.rect_filled(r, cr, fill);
                    painter.rect_stroke(r, cr, Stroke::new(sw, ink), StrokeKind::Middle);
                } else {
                    painter.add(EllipseShape { center: r.center(), radius: r.size() / 2.0, fill, stroke: Stroke::new(sw, ink), angle: 0.0 });
                }
                if !hide_text {
                    paint_text(painter, ui, &el.text, r.shrink(10.0 * z), SHAPE_FONT * z, style.text, true);
                }
            }
            Kind::Frame => {
                let cr = CornerRadius::same((8.0 * z).clamp(0.0, 40.0) as u8);
                painter.rect_filled(r, cr, style.surface.gamma_multiply(0.55));
                painter.rect_stroke(r, cr, Stroke::new(1.5_f32.max(z), style.ink(&el.color)), StrokeKind::Middle);
                if !hide_text {
                    let t = if el.text.trim().is_empty() { "Rahmen" } else { el.text.as_str() };
                    let label = Rect::from_min_size(r.min - vec2(0.0, 22.0 * z), vec2(r.width(), 20.0 * z));
                    let mut first = t.lines().next().unwrap_or_default().to_string();
                    truncate_chars(&mut first, 80);
                    paint_text(painter, ui, &first, label, FRAME_FONT * z, style.muted, false);
                }
            }
            Kind::Text => {
                if !hide_text {
                    let area = Rect::from_min_size(r.min, vec2(r.width().max(TEXT_FONT * z), r.height().max(TEXT_FONT * z * 1.4)));
                    paint_text(painter, ui, &el.text, area, TEXT_FONT * z, style.ink(&el.color), false);
                }
            }
            Kind::Line | Kind::Freehand => {
                let pts: Vec<Pos2> = el.world_points().map(|q| cam.to_screen(rect, p(q))).collect();
                let w = if el.kind == Kind::Freehand { (3.0 * z).max(1.0) } else { (2.5 * z).max(1.0) };
                let ink = style.ink(&el.color);
                if pts.len() == 1 {
                    if let Some(c) = pts.first() {
                        painter.circle_filled(*c, w / 2.0, ink);
                    }
                } else {
                    painter.add(Shape::line(pts, Stroke::new(w, ink)));
                }
            }
            Kind::Arrow => {
                let (a, b) = st.cache.endpoints(el);
                let (a, b) = (cam.to_screen(rect, a), cam.to_screen(rect, b));
                let ink = style.ink(&el.color);
                let head = (12.0 * z).clamp(5.0, 40.0);
                let dir = (b - a).normalized();
                let shaft_end = if (b - a).length() > head { b - dir * head * 0.8 } else { b };
                painter.line_segment([a, shaft_end], Stroke::new(sw, ink));
                arrow_head(painter, a, b, head, ink);
            }
        }
    }
}

fn paint_overlay(painter: &Painter, ui: &Ui, resp: &Response, rect: Rect, st: &ViewState, style: &BoardStyle) {
    let cam = st.camera;
    let accent = style.accent;
    // Hover outline.
    if st.tool == Tool::Select
        && matches!(st.interaction, Interaction::None)
        && let Some(h) = resp.hover_pos()
        && let Some(e) = hit_test(&st.cache, cam.to_world(rect, h), cam.zoom)
        && !st.is_selected(&e.id)
    {
        let r = cam.rect_to_screen(rect, st.cache.bounds(e)).expand(3.0);
        painter.rect_stroke(r, 4.0, Stroke::new(1.0, accent.gamma_multiply(0.5)), StrokeKind::Middle);
    }
    // Selection.
    for id in &st.selection {
        let Some(e) = st.cache.get(id) else { continue };
        if e.kind == Kind::Arrow || e.kind == Kind::Line {
            let (a, b) = if e.kind == Kind::Arrow {
                st.cache.endpoints(e)
            } else {
                let mut pts = e.world_points().map(p);
                let a = pts.next().unwrap_or(Pos2::ZERO);
                (a, pts.last().unwrap_or(a))
            };
            for q in [a, b] {
                painter.circle(cam.to_screen(rect, q), 4.5, style.surface, Stroke::new(1.5, accent));
            }
        } else {
            let r = cam.rect_to_screen(rect, st.cache.bounds(e)).expand(3.0);
            painter.rect_stroke(r, 4.0, Stroke::new(1.5, accent), StrokeKind::Middle);
        }
    }
    if resizable(st)
        && st.editing.is_none()
        && let Some(b) = selection_bounds(st)
    {
        let sb = cam.rect_to_screen(rect, b).expand(4.0);
        if st.selection.len() > 1 {
            painter.rect_stroke(sb, 2.0, Stroke::new(1.0, accent.gamma_multiply(0.7)), StrokeKind::Middle);
        }
        for (_, hp) in handles(sb) {
            let hr = Rect::from_center_size(hp, Vec2::splat(HANDLE));
            painter.rect_filled(hr, 2.0, style.surface);
            painter.rect_stroke(hr, 2.0, Stroke::new(1.5, accent), StrokeKind::Middle);
        }
    }
    let ptr = ui.input(|i| i.pointer.latest_pos()).map(|q| cam.to_world(rect, q));
    match &st.interaction {
        Interaction::Marquee { start, .. } => {
            if let Some(w) = ptr {
                let r = cam.rect_to_screen(rect, Rect::from_two_pos(*start, w));
                painter.rect_filled(r, 2.0, style.selection.gamma_multiply(0.35));
                painter.rect_stroke(r, 2.0, Stroke::new(1.0, accent), StrokeKind::Middle);
            }
        }
        Interaction::Create { kind, start } => {
            if let Some(w) = ptr {
                let (a, b) = (cam.to_screen(rect, *start), cam.to_screen(rect, w));
                let stroke = Stroke::new(1.5, accent);
                match kind {
                    Kind::Arrow | Kind::Line => {
                        painter.line_segment([a, b], stroke);
                        if *kind == Kind::Arrow {
                            arrow_head(painter, a, b, 12.0, accent);
                        }
                    }
                    Kind::Ellipse => {
                        let r = Rect::from_two_pos(a, b);
                        painter.add(EllipseShape { center: r.center(), radius: r.size() / 2.0, fill: Color32::TRANSPARENT, stroke, angle: 0.0 });
                    }
                    _ => {
                        painter.rect_stroke(Rect::from_two_pos(a, b), 4.0, stroke, StrokeKind::Middle);
                    }
                }
            }
        }
        Interaction::Pen { points } => {
            let pts: Vec<Pos2> = points.iter().map(|q| cam.to_screen(rect, *q)).collect();
            let color = style.ink(st.color.as_deref().unwrap_or("ink"));
            painter.add(Shape::line(pts, Stroke::new((3.0 * cam.zoom).max(1.0), color)));
        }
        _ => {}
    }
}

fn editor(ui: &mut Ui, base: Id, rect: Rect, board: &mut Board, st: &mut ViewState, style: &BoardStyle) {
    let Some(ed) = st.editing.as_mut() else { return };
    let Some(el) = st.cache.get(&ed.id).cloned() else {
        st.editing = None;
        return;
    };
    let cam = st.camera;
    let z = cam.zoom;
    if !ed.focused && ed.text.is_empty() {
        ed.text = el.text.clone();
    }
    let r = cam.rect_to_screen(rect, st.cache.bounds(&el));
    let (area, size, color) = match el.kind {
        Kind::Note => (r.shrink(12.0 * z), NOTE_FONT * z, style.note_text),
        Kind::Rect | Kind::Ellipse => (r.shrink(10.0 * z), SHAPE_FONT * z, style.text),
        Kind::Frame => (Rect::from_min_size(r.min - vec2(0.0, 22.0 * z), vec2(r.width().max(120.0), 20.0 * z)), FRAME_FONT * z, style.muted),
        _ => (Rect::from_min_size(r.min, vec2(r.width().max(160.0 * z), r.height().max(TEXT_FONT * z * 1.4))), TEXT_FONT * z, style.ink(&el.color)),
    };
    let area = area.intersect(rect);
    if area.width() < 8.0 || area.height() < 8.0 {
        // Too small to type in: commit and stop.
        let text = std::mem::take(&mut ed.text);
        let id = ed.id.clone();
        st.editing = None;
        if text != el.text {
            board.set_text(&id, &text);
        }
        return;
    }
    let edit_id = base.with(("edit", &ed.id));
    let out = ui.put(
        area,
        TextEdit::multiline(&mut ed.text)
            .id(edit_id)
            .font(FontId::proportional(size.max(6.0)))
            .text_color(color)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .background_color(Color32::TRANSPARENT)
            .desired_width(area.width())
            .char_limit(MAX_TEXT),
    );
    if !ed.focused {
        out.request_focus();
        ed.focused = true;
    } else if out.lost_focus() || !out.has_focus() {
        let text = std::mem::take(&mut ed.text);
        let id = ed.id.clone();
        st.editing = None;
        if text != el.text {
            board.set_text(&id, &text);
        }
    }
}

// ----- toolbar --------------------------------------------------------------------------------

fn paint_icon(painter: &Painter, icon: Icon, c: Pos2, color: Color32, accent: Color32) {
    let s = Stroke::new(1.6, color);
    let r = |w: f32, h: f32| Rect::from_center_size(c, vec2(w, h));
    match icon {
        Icon::Tool(Tool::Select) => {
            let o = c + vec2(-4.0, -7.0);
            let pts = vec![o, o + vec2(0.0, 13.0), o + vec2(3.5, 9.5), o + vec2(6.5, 14.5), o + vec2(8.5, 13.5), o + vec2(5.5, 8.5), o + vec2(10.0, 8.0)];
            painter.add(Shape::convex_polygon(pts, Color32::TRANSPARENT, s));
        }
        Icon::Tool(Tool::Hand) => {
            painter.rect_stroke(Rect::from_min_size(c + vec2(-5.0, -1.0), vec2(10.0, 8.0)), 3.0, s, StrokeKind::Middle);
            for (i, h) in [5.0_f32, 7.0, 7.0, 5.5].iter().enumerate() {
                let x = c.x - 4.0 + i as f32 * 2.7;
                painter.line_segment([pos2(x, c.y - 1.0), pos2(x, c.y - 1.0 - h)], s);
            }
        }
        Icon::Tool(Tool::Note) => {
            let b = r(14.0, 14.0);
            painter.add(Shape::convex_polygon(
                vec![b.left_top(), b.right_top(), b.right_bottom() - vec2(0.0, 5.0), b.right_bottom() - vec2(5.0, 0.0), b.left_bottom()],
                Color32::TRANSPARENT,
                s,
            ));
            painter.line_segment([b.right_bottom() - vec2(5.0, 0.0), b.right_bottom() - vec2(5.0, 5.0)], s);
            painter.line_segment([b.right_bottom() - vec2(5.0, 5.0), b.right_bottom() - vec2(0.0, 5.0)], s);
        }
        Icon::Tool(Tool::Rect) => {
            painter.rect_stroke(r(15.0, 11.0), 2.0, s, StrokeKind::Middle);
        }
        Icon::Tool(Tool::Ellipse) => {
            painter.add(EllipseShape { center: c, radius: vec2(8.0, 6.0), fill: Color32::TRANSPARENT, stroke: s, angle: 0.0 });
        }
        Icon::Tool(Tool::Arrow) => {
            let (a, b) = (c + vec2(-6.0, 6.0), c + vec2(7.0, -7.0));
            painter.line_segment([a, b - vec2(3.0, -3.0)], s);
            arrow_head(painter, a, b, 6.5, color);
        }
        Icon::Tool(Tool::Line) => {
            painter.line_segment([c + vec2(-7.0, 7.0), c + vec2(7.0, -7.0)], s);
        }
        Icon::Tool(Tool::Text) => {
            painter.line_segment([c + vec2(-6.0, -6.0), c + vec2(6.0, -6.0)], Stroke::new(2.0, color));
            painter.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 7.0)], Stroke::new(2.0, color));
        }
        Icon::Tool(Tool::Pen) => {
            let pts: Vec<Pos2> = (0..=12).map(|i| i as f32 / 12.0).map(|t| c + vec2(-7.0 + 14.0 * t, 3.0 * (t * 6.5).sin() - 1.0)).collect();
            painter.add(Shape::line(pts, s));
        }
        Icon::Tool(Tool::Eraser) => {
            let d = vec2(5.0, -5.0) * std::f32::consts::FRAC_1_SQRT_2;
            let n = vec2(5.0, 5.0) * std::f32::consts::FRAC_1_SQRT_2;
            let (a, b) = (c - d * 1.6 - n * 0.8, c + d * 1.6 - n * 0.8);
            painter.add(Shape::convex_polygon(vec![a, b, b + n * 1.6, a + n * 1.6], Color32::TRANSPARENT, s));
            painter.line_segment([c - n * 0.1 - d * 0.2, c - n * 0.1 + d * 1.6 + n * 0.0], s);
            painter.line_segment([c + vec2(-7.0, 7.5), c + vec2(6.0, 7.5)], Stroke::new(1.2, color));
        }
        Icon::Tool(Tool::Frame) => {
            let b = r(12.0, 12.0);
            for (x, y) in [(b.min.x, b.min.y), (b.max.x, b.min.y), (b.min.x, b.max.y), (b.max.x, b.max.y)] {
                let sx = if x < c.x { -1.0 } else { 1.0 };
                let sy = if y < c.y { -1.0 } else { 1.0 };
                painter.line_segment([pos2(x, y + sy * 3.0), pos2(x, y - sy * 3.0)], s);
                painter.line_segment([pos2(x + sx * 3.0, y), pos2(x - sx * 3.0, y)], s);
            }
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| std::f32::consts::PI + i as f32 / 16.0 * 1.2 * std::f32::consts::PI)
                .map(|t| c + vec2(flip * 6.0 * t.cos(), 6.0 * t.sin() + 2.0))
                .collect();
            let tip = c + vec2(-6.0 * flip, 6.5);
            painter.add(Shape::line(pts, s));
            arrow_head(painter, c + vec2(-6.0 * flip, 0.5), tip, 5.5, color);
        }
        Icon::Fit => {
            let b = r(14.0, 14.0);
            for corner in [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()] {
                let sx = if corner.x < c.x { 1.0 } else { -1.0 };
                let sy = if corner.y < c.y { 1.0 } else { -1.0 };
                painter.line_segment([corner, corner + vec2(sx * 4.5, 0.0)], s);
                painter.line_segment([corner, corner + vec2(0.0, sy * 4.5)], s);
            }
            painter.circle_stroke(c, 2.5, s);
        }
        Icon::Duplicate => {
            painter.rect_stroke(Rect::from_min_size(c + vec2(-7.0, -7.0), vec2(10.0, 10.0)), 2.0, s, StrokeKind::Middle);
            painter.rect_filled(Rect::from_min_size(c + vec2(-3.0, -3.0), vec2(10.0, 10.0)), 2.0, color);
        }
        Icon::Delete => {
            painter.line_segment([c + vec2(-7.0, -4.5), c + vec2(7.0, -4.5)], s);
            painter.line_segment([c + vec2(-2.5, -7.0), c + vec2(2.5, -7.0)], s);
            painter.add(Shape::convex_polygon(
                vec![c + vec2(-5.0, -4.5), c + vec2(5.0, -4.5), c + vec2(4.0, 7.0), c + vec2(-4.0, 7.0)],
                Color32::TRANSPARENT,
                s,
            ));
        }
        Icon::Front | Icon::Back => {
            let (top, bottom) = (Rect::from_min_size(c + vec2(-7.0, -7.0), vec2(9.0, 9.0)), Rect::from_min_size(c + vec2(-2.0, -2.0), vec2(9.0, 9.0)));
            if icon == Icon::Front {
                painter.rect_stroke(top, 2.0, s, StrokeKind::Middle);
                painter.rect_filled(bottom, 2.0, accent);
            } else {
                painter.rect_filled(top, 2.0, accent);
                painter.rect_stroke(bottom, 2.0, s, StrokeKind::Middle);
            }
        }
    }
}

fn bar_background(painter: &Painter, r: Rect, style: &BoardStyle) {
    painter.rect_filled(r.translate(vec2(0.0, 2.0)).expand(1.0), 12.0, Color32::from_black_alpha(22));
    painter.rect_filled(r, 12.0, style.surface);
    painter.rect_stroke(r, 12.0, Stroke::new(1.0, style.border), StrokeKind::Inside);
}

#[allow(clippy::too_many_arguments)]
fn icon_button(ui: &Ui, painter: &Painter, r: Rect, id: Id, icon: Icon, tip: &str, active: bool, enabled: bool, style: &BoardStyle) -> bool {
    let resp = ui.interact(r, id, if enabled { Sense::click() } else { Sense::hover() }).on_hover_text(tip);
    if active {
        painter.rect_filled(r.shrink(2.0), 8.0, style.selection);
    } else if resp.hovered() && enabled {
        painter.rect_filled(r.shrink(2.0), 8.0, style.border.gamma_multiply(0.6));
    }
    let color = if !enabled {
        style.muted.gamma_multiply(0.45)
    } else if active {
        style.accent
    } else {
        style.text
    };
    paint_icon(painter, icon, r.center(), color, style.accent);
    enabled && resp.clicked()
}

fn toolbar(ui: &mut Ui, painter: &Painter, rect: Rect, base: Id, board: &mut Board, st: &mut ViewState, style: &BoardStyle) {
    let btn = 34.0;
    let gap = 10.0;
    let n_tools = Tool::ALL.len() as f32;
    let width = btn * (n_tools + 3.0) + gap * 2.0 + 12.0;
    let x0 = (rect.center().x - width / 2.0).max(rect.min.x + 8.0);
    let bar = Rect::from_min_size(pos2(x0, rect.min.y + 10.0), vec2(width, btn + 8.0));
    bar_background(painter, bar, style);
    let mut x = bar.min.x + 6.0;
    let y = bar.min.y + 4.0;
    for t in Tool::ALL {
        let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
        if icon_button(ui, painter, r, base.with(("tool", t.label())), Icon::Tool(t), t.label(), st.tool == t, true, style) {
            st.tool = t;
            st.interaction = Interaction::None;
        }
        x += btn;
    }
    let sep = |painter: &Painter, x: f32| painter.line_segment([pos2(x, y + 7.0), pos2(x, y + btn - 7.0)], Stroke::new(1.0, style.border));
    sep(painter, x + gap / 2.0);
    x += gap;
    let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
    if icon_button(ui, painter, r, base.with("undo"), Icon::Undo, "Rückgängig (Strg+Z)", false, board.can_undo(), style) {
        board.undo();
    }
    x += btn;
    let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
    if icon_button(ui, painter, r, base.with("redo"), Icon::Redo, "Wiederholen (Strg+Umschalt+Z)", false, board.can_redo(), style) {
        board.redo();
    }
    x += btn;
    sep(painter, x + gap / 2.0);
    x += gap;
    let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
    if icon_button(ui, painter, r, base.with("fit"), Icon::Fit, "Alles zeigen (Umschalt+1)", false, true, style) {
        fit(st, rect);
    }

    // Bottom bar: colours, selection actions, zoom.
    let sw = 26.0;
    let has_sel = !st.selection.is_empty();
    let n_actions = if has_sel { 4.0 } else { 0.0 };
    let zoom_w = 58.0;
    let width = sw * 9.0 + 12.0 + if has_sel { gap + btn * n_actions } else { 0.0 } + gap + zoom_w;
    let x0 = (rect.center().x - width / 2.0).max(rect.min.x + 8.0);
    let bar = Rect::from_min_size(pos2(x0, rect.max.y - 10.0 - (btn + 8.0)), vec2(width, btn + 8.0));
    bar_background(painter, bar, style);
    let current = st.selection.first().and_then(|id| st.cache.get(id)).map(|e| e.color.clone()).or_else(|| st.color.clone());
    let mut x = bar.min.x + 6.0;
    let cy = bar.center().y;
    for name in std::iter::once("ink").chain(PALETTE) {
        let r = Rect::from_center_size(pos2(x + sw / 2.0, cy), Vec2::splat(sw));
        let resp = ui.interact(r, base.with(("color", name)), Sense::click()).on_hover_text(color_label(name));
        let fill = if name == "ink" { style.text } else { style.fill(name) };
        let active = current.as_deref() == Some(name);
        if active {
            painter.circle_stroke(r.center(), 11.0, Stroke::new(2.0, style.accent));
        } else if resp.hovered() {
            painter.circle_stroke(r.center(), 11.0, Stroke::new(1.0, style.border));
        }
        painter.circle(r.center(), 8.0, fill, Stroke::new(1.0, mix(fill, style.text, 0.25)));
        if resp.clicked() {
            st.color = Some(name.to_string());
            if has_sel {
                let ids = st.selection.clone();
                board.set_color(&ids, name);
            }
        }
        x += sw;
    }
    x += 6.0;
    if has_sel {
        sep(painter, x + gap / 2.0);
        x += gap;
        let actions = [
            (Icon::Duplicate, "Duplizieren (Strg+D)", "dup"),
            (Icon::Front, "Nach vorne", "front"),
            (Icon::Back, "Nach hinten", "back"),
            (Icon::Delete, "Löschen (Entf)", "del"),
        ];
        for (icon, tip, key) in actions {
            let r = Rect::from_min_size(pos2(x, bar.min.y + 4.0), Vec2::splat(btn));
            if icon_button(ui, painter, r, base.with(key), icon, tip, false, true, style) {
                let ids = st.selection.clone();
                match icon {
                    Icon::Duplicate => {
                        let els = selected_elements(st);
                        paste(board, st, els, Some(vec2(24.0, 24.0)));
                    }
                    Icon::Front => board.bring_to_front(&ids),
                    Icon::Back => board.send_to_back(&ids),
                    _ => {
                        board.delete(&ids);
                        st.selection.clear();
                    }
                }
            }
            x += btn;
        }
    }
    sep(painter, x + gap / 2.0);
    x += gap;
    let r = Rect::from_min_size(pos2(x, bar.min.y + 4.0), vec2(zoom_w - 6.0, btn));
    let resp = ui.interact(r, base.with("zoom"), Sense::click()).on_hover_text("Auf 100 % zurücksetzen (Strg+0)");
    if resp.hovered() {
        painter.rect_filled(r.shrink(2.0), 8.0, style.border.gamma_multiply(0.6));
    }
    painter.text(r.center(), Align2::CENTER_CENTER, format!("{:.0} %", st.camera.zoom * 100.0), FontId::proportional(13.0), style.muted);
    if resp.clicked() {
        let c = st.camera.center;
        st.camera = Camera { center: c, zoom: 1.0 };
    }
}
