//! `BoardView`: the egui canvas for a [`Board`].
//!
//! The view state (camera, tool, selection, clipboard) is plain Rust in [`ViewState`] and never
//! goes into the CRDT. Colours come from a [`BoardStyle`] the app fills from its theme tokens.

use std::collections::{HashMap, HashSet};

use egui::epaint::EllipseShape;
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, Painter, PointerButton, Pos2,
    Rect, Response, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2, vec2,
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
/// The first automatic fit never zooms out further than this, so text stays readable.
pub const READABLE_ZOOM: f32 = 0.65;
/// Fitting never zooms in beyond 100 %.
const FIT_MAX_ZOOM: f32 = 1.0;
/// Notes grow with their text up to this height (world units).
const MAX_NOTE_H: f64 = 520.0;
/// Padding inside notes (world units).
const NOTE_PAD: f32 = 12.0;
/// Distance between a note and the one created next to it (world units).
const NOTE_GAP: f64 = 72.0;
/// Screen distance of the „+“ handles from the selection outline, and their radius.
const PLUS_OFFSET: f32 = 26.0;
const PLUS_R: f32 = 9.0;
/// Screen distance at which moves snap to other elements' edges and centres.
const SNAP_DIST: f32 = 6.0;
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
    /// Alignment guides while moving.
    pub guide: Color32,
    /// Text on the accent (the active tool, the start buttons).
    pub on_accent: Color32,
    /// Tinted accent (halo of the empty state, "Notiz anlegen").
    pub accent_soft: Color32,
    /// Pill-shaped floating bars and the designed empty state with start buttons.
    pub rounded: bool,
    /// Font family for the empty state's title (a bold cut), when the app has one.
    pub title_family: Option<egui::FontFamily>,
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
            guide: Color32::from_rgb(0xe0, 0x3a, 0x6e),
            on_accent: Color32::WHITE,
            accent_soft: Color32::from_rgb(0xf6, 0xd9, 0xc8),
            rounded: false,
            title_family: None,
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
            guide: Color32::from_rgb(0xff, 0x6b, 0x9a),
            on_accent: Color32::from_rgb(0x1e, 0x1d, 0x1b),
            accent_soft: Color32::from_rgb(0x5a, 0x34, 0x22),
            rounded: false,
            title_family: None,
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

    /// One line on how to use the tool, shown while it is active.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Klicken wählt aus · Ziehen verschiebt · Doppelklick: Notiz",
            Tool::Hand => "Ziehen verschiebt die Ansicht · V: Auswählen",
            Tool::Note => "Notiz: Klicken oder Rahmen ziehen · Esc: zurück",
            Tool::Rect => "Rechteck: Ziehen oder Klicken · Esc: zurück",
            Tool::Ellipse => "Ellipse: Ziehen oder Klicken · Esc: zurück",
            Tool::Arrow => "Pfeil: von einem Element zum anderen ziehen · Esc: zurück",
            Tool::Line => "Linie: Ziehen · Esc: zurück",
            Tool::Text => "Text: Klicken und schreiben · Esc: zurück",
            Tool::Pen => "Stift: frei zeichnen · Esc: zurück",
            Tool::Eraser => "Radierer: über Elemente streichen · Esc: zurück",
            Tool::Frame => "Rahmen: Ziehen · Esc: zurück",
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
        /// World bounds of the moved elements at the start (for snapping).
        bounds: Rect,
    },
    /// Dragging one end of a selected arrow; `target` is the element it would bind to.
    ArrowEnd {
        id: String,
        to_end: bool,
        target: Option<String>,
    },
    /// Dragging from a „+“ handle: connect to an element or create a note where released.
    Connect {
        from: String,
        dir: Vec2,
        start: Pos2,
        target: Option<String>,
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
    /// Height of a note when editing started; it grows from there with the text.
    base_h: Option<f64>,
    /// Current height of a growing note (applied on commit).
    grow_h: Option<f64>,
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
    /// Snap moved elements to the grid (hold Alt while dragging to move freely).
    pub snap: bool,
    /// Show the shortcut overlay (F1 or „?“).
    pub help: bool,
    /// "Mindmap aus Chat" on the empty board was clicked; the app takes it from here.
    pub wants_mindmap: bool,
    fit_pending: bool,
    /// Canvas size and camera of the last automatic fit; refit when the canvas size settles.
    fitted: Option<(Vec2, Camera)>,
    /// Alignment guides of the current move: (vertical, coordinate, from, to) in world units.
    guides: Vec<(bool, f32, f32, f32)>,
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
            snap: true,
            help: false,
            wants_mindmap: false,
            fit_pending: true,
            fitted: None,
            guides: Vec::new(),
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
        self.editing = Some(Editing { id: id.to_string(), text: String::new(), focused: false, base_h: None, grow_h: None });
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
    Snap,
    Help,
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
        refresh(st, board);
        st.selection.retain(|id| st.cache.index.contains_key(id));
        st.selection.dedup();
        if st.editing.as_ref().is_some_and(|e| !st.cache.index.contains_key(&e.id)) {
            st.editing = None;
        }
        auto_fit(st, rect);
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }

        navigate(ui, &resp, rect, st);
        if st.help {
            help_input(ui, &resp, st);
        } else {
            pointer(ui, &resp, rect, board, st);
            keyboard(ui, &resp, rect, board, st);
        }
        refresh(st, board);
        cursor(ui, &resp, rect, st);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, style.bg);
        grid(&painter, rect, &st.camera, style);
        paint_elements(&painter, ui, rect, st, style);
        paint_overlay(&painter, ui, &resp, rect, st, style);
        editor(ui, resp.id, rect, board, st, style);
        if st.cache.els.is_empty() && st.editing.is_none() && !st.help {
            if style.rounded {
                empty_start(&painter, ui, rect, resp.id, board, st, style);
            } else {
                empty_hint(&painter, ui, rect, style);
            }
        }
        if st.toolbar {
            toolbar(ui, &painter, rect, resp.id, board, st, style);
            if !st.help {
                context_bar(ui, &painter, rect, resp.id, board, st, style);
            }
        }
        if st.help {
            help_overlay(ui, &painter, rect, style);
        }
        resp
    }
}

/// Refresh the element cache and apply the growing height of the note being edited.
fn refresh(st: &mut ViewState, board: &Board) {
    st.cache.refresh(board);
    if let Some(ed) = &st.editing
        && let Some(h) = ed.grow_h
        && let Some(&i) = st.cache.index.get(&ed.id)
        && let Some(el) = st.cache.els.get_mut(i)
    {
        el.h = h;
    }
}

// ----- camera ---------------------------------------------------------------------------------

/// Fit all elements into the canvas, never zooming out below `min_zoom`.
fn fit(st: &mut ViewState, rect: Rect, min_zoom: f32) {
    let mut b = Rect::NOTHING;
    for e in &st.cache.els {
        b = b.union(st.cache.bounds(e));
    }
    if !b.min.x.is_finite() || !b.max.x.is_finite() {
        st.camera = Camera::default();
        return;
    }
    // Room for the „+“ handles around the content.
    let margin = 44.0;
    // Room for the tool bar on top and the colour bar at the bottom.
    let bars = if st.toolbar { 56.0 } else { 0.0 };
    let avail = vec2((rect.width() - 2.0 * margin).max(32.0), (rect.height() - 2.0 * margin - 2.0 * bars).max(32.0));
    let zoom = (avail.x / b.width().max(1.0)).min(avail.y / b.height().max(1.0)).clamp(min_zoom.clamp(MIN_ZOOM, FIT_MAX_ZOOM), FIT_MAX_ZOOM);
    // The centre stays in view even when not everything fits (a mind map's root is there).
    st.camera = Camera { center: b.center(), zoom };
}

/// The fit requested with [`ViewState::zoom_to_fit`], applied once the canvas has a real size,
/// and repeated while the canvas is still settling (until the user moves the camera).
fn auto_fit(st: &mut ViewState, rect: Rect) {
    let ready = rect.width() >= 120.0 && rect.height() >= 120.0;
    if st.fit_pending {
        if ready {
            fit(st, rect, READABLE_ZOOM);
            st.fit_pending = false;
            st.fitted = Some((rect.size(), st.camera));
        }
        return;
    }
    if let Some((size, cam)) = st.fitted {
        if cam != st.camera || !matches!(st.interaction, Interaction::None) {
            st.fitted = None;
        } else if (size - rect.size()).length() > 0.5 && ready {
            fit(st, rect, READABLE_ZOOM);
            st.fitted = Some((rect.size(), st.camera));
        }
    }
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

fn cursor(ui: &Ui, resp: &Response, rect: Rect, st: &ViewState) {
    if !resp.hovered() {
        return;
    }
    let space = ui.input(|i| i.key_down(Key::Space));
    let hover = resp.hover_pos();
    let on_handle = st.tool == Tool::Select
        && matches!(st.interaction, Interaction::None)
        && hover.is_some_and(|h| plus_at(st, rect, h).is_some() || arrow_end_at(st, rect, h).is_some());
    let icon = match (&st.interaction, st.tool) {
        _ if st.help => CursorIcon::Default,
        (Interaction::Pan, _) => CursorIcon::Grabbing,
        _ if space => CursorIcon::Grab,
        (_, Tool::Hand) => CursorIcon::Grab,
        (Interaction::Move { .. }, _) => CursorIcon::Grabbing,
        (Interaction::ArrowEnd { .. } | Interaction::Connect { .. }, _) => CursorIcon::Crosshair,
        _ if on_handle => CursorIcon::PointingHand,
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

/// The single selected box (note, rectangle, ellipse) that shows „+“ handles.
fn plus_source(st: &ViewState) -> Option<&Element> {
    if st.tool != Tool::Select || st.editing.is_some() {
        return None;
    }
    let [id] = st.selection.as_slice() else { return None };
    st.cache.get(id).filter(|e| matches!(e.kind, Kind::Note | Kind::Rect | Kind::Ellipse))
}

/// Screen positions of the four „+“ handles with their direction.
fn plus_handles(st: &ViewState, rect: Rect) -> Option<(String, [(Vec2, Pos2); 4])> {
    let el = plus_source(st)?;
    let r = st.camera.rect_to_screen(rect, st.cache.bounds(el));
    if r.width() < 24.0 || r.height() < 16.0 {
        return None;
    }
    let c = r.center();
    Some((
        el.id.clone(),
        [
            (vec2(1.0, 0.0), pos2(r.max.x + PLUS_OFFSET, c.y)),
            (vec2(0.0, 1.0), pos2(c.x, r.max.y + PLUS_OFFSET)),
            (vec2(-1.0, 0.0), pos2(r.min.x - PLUS_OFFSET, c.y)),
            (vec2(0.0, -1.0), pos2(c.x, r.min.y - PLUS_OFFSET)),
        ],
    ))
}

fn plus_at(st: &ViewState, rect: Rect, screen: Pos2) -> Option<(String, Vec2)> {
    let (id, hs) = plus_handles(st, rect)?;
    hs.into_iter().find(|(_, hp)| hp.distance(screen) <= PLUS_R + 3.0).map(|(d, _)| (id, d))
}

/// The end of the single selected arrow under `screen`: (arrow id, is the `to` end).
fn arrow_end_at(st: &ViewState, rect: Rect, screen: Pos2) -> Option<(String, bool)> {
    if st.tool != Tool::Select || st.editing.is_some() {
        return None;
    }
    let [id] = st.selection.as_slice() else { return None };
    let el = st.cache.get(id).filter(|e| e.kind == Kind::Arrow)?;
    let (a, b) = st.cache.endpoints(el);
    let (sa, sb) = (st.camera.to_screen(rect, a), st.camera.to_screen(rect, b));
    let (da, db) = (sa.distance(screen), sb.distance(screen));
    let tol = 9.0;
    if db <= tol && db <= da {
        Some((id.clone(), true))
    } else if da <= tol {
        Some((id.clone(), false))
    } else {
        None
    }
}

/// The element an arrow end at `w` binds to: the topmost box under the pointer (a little
/// tolerance), never a path and never one of `exclude`.
fn bind_target(cache: &Cache, w: Pos2, zoom: f32, exclude: &[&str]) -> Option<String> {
    let tol = 10.0 / zoom;
    cache.els.iter().rev().find(|e| !e.kind.is_path() && !exclude.contains(&e.id.as_str()) && hits(cache, e, w, tol)).map(|e| e.id.clone())
}

/// The arrow as it would be with the dragged end at `w` (or bound to `target`).
fn rebound_arrow(cache: &Cache, id: &str, to_end: bool, target: Option<&str>, w: Pos2) -> Option<Element> {
    let mut el = cache.get(id)?.clone();
    let (a, b) = cache.endpoints(&el);
    let wp = (w.x as f64, w.y as f64);
    if to_end {
        el.set_world_points(&[(a.x as f64, a.y as f64), wp]);
        el.to = target.map(str::to_string);
    } else {
        el.set_world_points(&[wp, (b.x as f64, b.y as f64)]);
        el.from = target.map(str::to_string);
    }
    Some(el)
}

/// The arrow a „+“ drag would create.
fn connect_arrow(cache: &Cache, from: &str, target: Option<&str>, w: Pos2) -> Option<Element> {
    let src = cache.get(from)?;
    let mut el = Element::segment(Kind::Arrow, src.center(), (w.x as f64, w.y as f64)).with_color("gray");
    el.from = Some(from.to_string());
    el.to = target.map(str::to_string);
    Some(el)
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

/// Whether a box element (not a path or frame) overlaps `r` (world units).
fn occupied(cache: &Cache, r: Rect) -> bool {
    cache.els.iter().any(|e| !e.kind.is_path() && e.kind != Kind::Frame && cache.bounds(e).intersects(r))
}

/// Create a note next to `from` in direction `dir` (or centred on `at`), connected by an arrow
/// from `from`, in one undo step, and start editing it. `size` defaults to the source's size.
fn create_next(board: &mut Board, st: &mut ViewState, from: &str, dir: Vec2, size: Option<(f64, f64)>, at: Option<Pos2>) -> Option<String> {
    let src = st.cache.get(from)?.clone();
    let default = Element::new(Kind::Note, 0.0, 0.0);
    let (w, h) = size.unwrap_or(if src.kind == Kind::Note { (src.w, src.h) } else { (default.w, default.h) });
    let (w, h) = (w.clamp(40.0, 2000.0), h.clamp(40.0, MAX_NOTE_H));
    let color = if src.kind == Kind::Note { src.color.clone() } else { new_color(st, Kind::Note) };
    let (x, y) = match at {
        Some(p) => (p.x as f64 - w / 2.0, p.y as f64 - h / 2.0),
        None => {
            let (cx, cy) = src.center();
            let x = if dir.x > 0.0 {
                src.x + src.w + NOTE_GAP
            } else if dir.x < 0.0 {
                src.x - NOTE_GAP - w
            } else {
                cx - w / 2.0
            };
            let y = if dir.y > 0.0 {
                src.y + src.h + NOTE_GAP
            } else if dir.y < 0.0 {
                src.y - NOTE_GAP - h
            } else {
                cy - h / 2.0
            };
            // Next to existing neighbours: step sideways until the spot is free.
            let step = if dir.x != 0.0 { vec2(0.0, h as f32 + 24.0) } else { vec2(w as f32 + 24.0, 0.0) };
            let mut r = Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32));
            for _ in 0..60 {
                if !occupied(&st.cache, r.expand(4.0)) {
                    break;
                }
                r = r.translate(step);
            }
            (r.min.x as f64, r.min.y as f64)
        }
    };
    let mut note = Element::new(Kind::Note, x, y).with_size(w, h).with_color(color);
    note.id = board.fresh_element_id();
    let arrow = Element::connector(&src, &note).with_color("gray");
    let ids = board.add_many(vec![note, arrow]);
    let id = ids.into_iter().next()?;
    st.selection = vec![id.clone()];
    st.edit(&id);
    st.tool = Tool::Select;
    Some(id)
}

/// Direction of a mind-map child: away from the node's parent, or to the right.
fn child_dir(cache: &Cache, id: &str) -> Vec2 {
    let Some(el) = cache.get(id) else { return vec2(1.0, 0.0) };
    let parent = cache.els.iter().filter(|a| a.kind == Kind::Arrow && a.to.as_deref() == Some(id)).find_map(|a| a.from.as_deref().and_then(|f| cache.get(f)));
    let Some(p) = parent else { return vec2(1.0, 0.0) };
    let (d, e) = (el.center(), p.center());
    let (dx, dy) = (d.0 - e.0, d.1 - e.1);
    if dx.abs() >= dy.abs() { vec2(if dx < 0.0 { -1.0 } else { 1.0 }, 0.0) } else { vec2(0.0, if dy < 0.0 { -1.0 } else { 1.0 }) }
}

fn pointer(ui: &mut Ui, resp: &Response, rect: Rect, board: &mut Board, st: &mut ViewState) {
    let cam = st.camera;
    let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
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
                Tool::Select => {
                    if let Some((id, to_end)) = arrow_end_at(st, rect, origin) {
                        Interaction::ArrowEnd { id, to_end, target: None }
                    } else if let Some((from, dir)) = plus_at(st, rect, origin) {
                        Interaction::Connect { from, dir, start: ow, target: None }
                    } else {
                        start_select_drag(board, st, rect, origin, ow, shift)
                    }
                }
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
        drag_update(board, st, w, shift, alt);
    }

    if resp.drag_stopped() {
        let it = std::mem::take(&mut st.interaction);
        st.guides.clear();
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
            Interaction::ArrowEnd { id, to_end, target } => {
                if let Some(el) = rebound_arrow(&st.cache, &id, to_end, target.as_deref(), w) {
                    let _ = board.update(&el);
                }
            }
            Interaction::Connect { from, dir, start, target } => match target {
                Some(t) => {
                    if let Some(el) = connect_arrow(&st.cache, &from, Some(&t), w) {
                        let id = board.add(el);
                        st.selection = vec![id];
                    }
                }
                None if start.distance(w) * cam.zoom < 6.0 => {
                    create_next(board, st, &from, dir, None, None);
                }
                None => {
                    create_next(board, st, &from, dir, None, Some(w));
                }
            },
            _ => {}
        }
    }

    if resp.clicked() && !space {
        match st.tool {
            Tool::Select => {
                if let Some((from, dir)) = plus_at(st, rect, pos) {
                    create_next(board, st, &from, dir, None, None);
                    return;
                }
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
            let mut bounds = Rect::NOTHING;
            for e in st.selection.iter().filter_map(|id| st.cache.get(id)).filter(|e| e.kind != Kind::Arrow) {
                bounds = bounds.union(st.cache.bounds(e));
            }
            Interaction::Move { start: ow, orig, last: Vec2::ZERO, bounds }
        }
        None => {
            if !shift {
                st.selection.clear();
            }
            Interaction::Marquee { start: ow, base: st.selection.clone() }
        }
    }
}

fn drag_update(board: &mut Board, st: &mut ViewState, w: Pos2, shift: bool, alt: bool) {
    let zoom = st.camera.zoom;
    match &mut st.interaction {
        Interaction::Move { start, orig, last, bounds } => {
            let mut d = w - *start;
            st.guides.clear();
            if !alt && bounds.min.x.is_finite() {
                let moving: HashSet<&str> = orig.iter().map(|(id, _, _)| id.as_str()).collect();
                d = snap_move(&st.cache, &moving, *bounds, d, zoom, st.snap, &mut st.guides);
            }
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
        Interaction::ArrowEnd { id, to_end, target } => {
            let other = st.cache.get(id).and_then(|a| if *to_end { a.from.clone() } else { a.to.clone() });
            let mut exclude = vec![id.as_str()];
            if let Some(o) = other.as_deref() {
                exclude.push(o);
            }
            *target = bind_target(&st.cache, w, zoom, &exclude);
        }
        Interaction::Connect { from, target, .. } => {
            *target = bind_target(&st.cache, w, zoom, &[from.as_str()]);
        }
        Interaction::None | Interaction::Pan | Interaction::Create { .. } => {}
    }
}

/// Snap a move by `d` of elements with world `bounds`: to other elements' edges and centres
/// (recording guides), else to the grid when `grid` is on.
fn snap_move(cache: &Cache, moving: &HashSet<&str>, bounds: Rect, d: Vec2, zoom: f32, grid: bool, guides: &mut Vec<(bool, f32, f32, f32)>) -> Vec2 {
    let tol = SNAP_DIST / zoom.max(1e-3);
    let mb = bounds.translate(d);
    let xs = [mb.min.x, mb.center().x, mb.max.x];
    let ys = [mb.min.y, mb.center().y, mb.max.y];
    // Best (|diff|, diff, target rect) per axis.
    let mut bx: Option<(f32, f32)> = None;
    let mut by: Option<(f32, f32)> = None;
    let near = mb.expand(1200.0 / zoom.max(1e-3));
    let others: Vec<Rect> =
        cache.els.iter().filter(|e| !moving.contains(e.id.as_str()) && !e.kind.is_path()).map(|e| cache.bounds(e)).filter(|b| b.intersects(near)).collect();
    for b in &others {
        for tx in [b.min.x, b.center().x, b.max.x] {
            for mx in xs {
                let diff = tx - mx;
                if diff.abs() <= tol && bx.is_none_or(|(a, _)| diff.abs() < a) {
                    bx = Some((diff.abs(), diff));
                }
            }
        }
        for ty in [b.min.y, b.center().y, b.max.y] {
            for my in ys {
                let diff = ty - my;
                if diff.abs() <= tol && by.is_none_or(|(a, _)| diff.abs() < a) {
                    by = Some((diff.abs(), diff));
                }
            }
        }
    }
    let mut d = d;
    match bx {
        Some((_, diff)) => d.x += diff,
        None if grid => d.x += (mb.min.x / GRID).round() * GRID - mb.min.x,
        None => {}
    }
    match by {
        Some((_, diff)) => d.y += diff,
        None if grid => d.y += (mb.min.y / GRID).round() * GRID - mb.min.y,
        None => {}
    }
    // Guides for every edge or centre that now lines up exactly.
    let mb = bounds.translate(d);
    let eps = 0.01 / zoom.max(1e-3) + 1e-3;
    for b in &others {
        if bx.is_some() {
            for tx in [b.min.x, b.center().x, b.max.x] {
                if [mb.min.x, mb.center().x, mb.max.x].iter().any(|m| (m - tx).abs() <= eps) {
                    guides.push((true, tx, mb.min.y.min(b.min.y), mb.max.y.max(b.max.y)));
                }
            }
        }
        if by.is_some() {
            for ty in [b.min.y, b.center().y, b.max.y] {
                if [mb.min.y, mb.center().y, mb.max.y].iter().any(|m| (m - ty).abs() <= eps) {
                    guides.push((false, ty, mb.min.x.min(b.min.x), mb.max.x.max(b.max.x)));
                }
            }
        }
        if guides.len() > 24 {
            break;
        }
    }
    d
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

/// While the shortcut overlay is open: any click, Esc, F1 or „?“ closes it.
fn help_input(ui: &mut Ui, resp: &Response, st: &mut ViewState) {
    let close = ui.input_mut(|i| {
        i.consume_key(Modifiers::NONE, Key::Escape) || i.consume_key(Modifiers::NONE, Key::F1) || i.consume_key(Modifiers::NONE, Key::Questionmark)
    });
    if close || resp.clicked() || resp.drag_started() {
        st.help = false;
        st.interaction = Interaction::None;
    }
}

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
        if i.consume_key(Modifiers::NONE, Key::F1) || i.consume_key(Modifiers::NONE, Key::Questionmark) {
            st.help = true;
        }
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
        fit(st, rect, MIN_ZOOM);
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

/// Wrapped text in `area`: centred lines (notes, shapes) or left-aligned, vertically centred
/// when it fits, else from the top.
fn paint_text(painter: &Painter, ui: &Ui, text: &str, area: Rect, size: f32, color: Color32, center: bool) {
    if size < 4.0 || text.is_empty() || area.width() < 4.0 {
        return;
    }
    let galley = ui.fonts_mut(|f| f.layout_job(text_job(text, size, color, area.width(), center)));
    let gs = galley.size();
    let top = if gs.y < area.height() { area.center().y - gs.y / 2.0 } else { area.min.y };
    let x = if center { area.center().x } else { area.min.x };
    painter.with_clip_rect(area.expand(2.0).intersect(painter.clip_rect())).galley(pos2(x, top), galley, color);
}

fn text_job(text: &str, size: f32, color: Color32, wrap: f32, center: bool) -> LayoutJob {
    let mut job = LayoutJob::simple(text.to_string(), FontId::proportional(size), color, wrap.max(1.0));
    if center {
        job.halign = Align::Center;
    }
    job
}

/// Height (world units) a note of width `w` needs for `text`.
fn note_fit_height(ui: &Ui, text: &str, w: f64) -> f64 {
    let wrap = (w as f32 - 2.0 * NOTE_PAD).max(8.0);
    let t = if text.is_empty() { " " } else { text };
    let galley = ui.fonts_mut(|f| f.layout_job(text_job(t, NOTE_FONT, Color32::BLACK, wrap, true)));
    // A trailing newline starts a row the galley does not count yet.
    let extra = if text.ends_with('\n') { NOTE_FONT * 1.3 } else { 0.0 };
    (galley.size().y + extra + 2.0 * NOTE_PAD + 8.0) as f64
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
    let rebinding = match &st.interaction {
        Interaction::ArrowEnd { id, .. } => Some(id.as_str()),
        _ => None,
    };
    for el in &st.cache.els {
        if rebinding == Some(el.id.as_str()) {
            continue;
        }
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
                    paint_text(painter, ui, &el.text, r.shrink(NOTE_PAD * z), NOTE_FONT * z, style.note_text, true);
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
    // Selection (an arrow being re-bound is drawn as a preview below).
    let rebinding = match &st.interaction {
        Interaction::ArrowEnd { id, .. } => Some(id.as_str()),
        _ => None,
    };
    for id in &st.selection {
        let Some(e) = st.cache.get(id) else { continue };
        if rebinding == Some(id.as_str()) {
            continue;
        }
        if e.kind == Kind::Arrow || e.kind == Kind::Line {
            let (a, b) = if e.kind == Kind::Arrow {
                st.cache.endpoints(e)
            } else {
                let mut pts = e.world_points().map(p);
                let a = pts.next().unwrap_or(Pos2::ZERO);
                (a, pts.last().unwrap_or(a))
            };
            let hover = resp.hover_pos();
            for q in [a, b] {
                let sq = cam.to_screen(rect, q);
                let hot = e.kind == Kind::Arrow && hover.is_some_and(|h| h.distance(sq) <= 9.0);
                painter.circle(sq, if hot { 6.5 } else { 5.0 }, if hot { accent } else { style.surface }, Stroke::new(1.5, accent));
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
    // „+“ handles on a single selected box: click for a connected note, drag to connect.
    if matches!(st.interaction, Interaction::None)
        && let Some((_, hs)) = plus_handles(st, rect)
    {
        let hover = resp.hover_pos();
        for (_, hp) in hs {
            let hot = hover.is_some_and(|h| h.distance(hp) <= PLUS_R + 3.0);
            let (fill, ink) = if hot { (accent, style.surface) } else { (style.surface, accent) };
            painter.circle(hp, PLUS_R, fill, Stroke::new(1.5, accent));
            let a = PLUS_R * 0.5;
            painter.line_segment([hp - vec2(a, 0.0), hp + vec2(a, 0.0)], Stroke::new(1.6, ink));
            painter.line_segment([hp - vec2(0.0, a), hp + vec2(0.0, a)], Stroke::new(1.6, ink));
        }
    }
    // Alignment guides while moving.
    for &(vertical, c, from, to) in &st.guides {
        let (a, b) = if vertical { (pos2(c, from), pos2(c, to)) } else { (pos2(from, c), pos2(to, c)) };
        let (a, b) = (cam.to_screen(rect, a), cam.to_screen(rect, b));
        painter.line_segment([a, b], Stroke::new(1.0, style.guide));
    }
    let ptr = ui.input(|i| i.pointer.latest_pos()).map(|q| cam.to_world(rect, q));
    let preview = match (&st.interaction, ptr) {
        (Interaction::ArrowEnd { id, to_end, target }, Some(w)) => rebound_arrow(&st.cache, id, *to_end, target.as_deref(), w).map(|e| (e, target.clone())),
        (Interaction::Connect { from, target, .. }, Some(w)) => connect_arrow(&st.cache, from, target.as_deref(), w).map(|e| (e, target.clone())),
        _ => None,
    };
    if let Some((el, target)) = preview {
        if let Some(t) = target.as_deref().and_then(|t| st.cache.get(t)) {
            let r = cam.rect_to_screen(rect, st.cache.bounds(t)).expand(5.0);
            painter.rect_stroke(r, 6.0, Stroke::new(2.0, accent), StrokeKind::Middle);
        }
        let (a, b) = st.cache.endpoints(&el);
        let (a, b) = (cam.to_screen(rect, a), cam.to_screen(rect, b));
        painter.line_segment([a, b], Stroke::new(2.0, accent));
        arrow_head(painter, a, b, 12.0, accent);
        painter.circle(b, 5.5, accent, Stroke::new(1.5, style.surface));
    }
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

/// Consume a press of `key` without Shift, Alt or Ctrl (checked on the event itself).
fn consume_plain(i: &mut egui::InputState, key: Key) -> bool {
    let before = i.events.len();
    i.events.retain(|e| !matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && !modifiers.shift && !modifiers.alt && !modifiers.command && !modifiers.ctrl));
    i.events.len() != before
}

/// Stop editing: write the text (and a grown note height) as one undo step.
fn commit_edit(board: &mut Board, st: &mut ViewState) {
    let Some(ed) = st.editing.take() else { return };
    let Some(mut el) = board.element(&ed.id) else { return };
    let mut changed = false;
    if el.text != ed.text {
        el.text = ed.text;
        changed = true;
    }
    if let Some(h) = ed.grow_h
        && (h - el.h).abs() > 0.5
    {
        el.h = h;
        changed = true;
    }
    if changed {
        let _ = board.update(&el);
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
    let edit_id = base.with(("edit", &ed.id));
    let has_focus = ui.memory(|m| m.has_focus(edit_id));
    // Enter finishes (Shift+Enter is a new line); Tab on a note adds a connected child note.
    let (enter, tab) = if ed.focused && has_focus {
        ui.input_mut(|i| (consume_plain(i, Key::Enter), el.kind == Kind::Note && consume_plain(i, Key::Tab)))
    } else {
        (false, false)
    };
    if enter || tab {
        let id = ed.id.clone();
        commit_edit(board, st);
        refresh(st, board);
        if tab {
            let dir = child_dir(&st.cache, &id);
            let w = st.cache.get(&id).map(|e| e.w).unwrap_or(200.0);
            create_next(board, st, &id, dir, Some((w, 64.0)), None);
            refresh(st, board);
        } else {
            st.selection = vec![id];
            ui.memory_mut(|m| m.request_focus(base));
        }
        return;
    }
    // Notes grow with their text while typing (never below their height at the start).
    let mut el = el;
    if el.kind == Kind::Note {
        let base_h = *ed.base_h.get_or_insert(el.h);
        let need = note_fit_height(ui, &ed.text, el.w).clamp(base_h.min(MAX_NOTE_H), MAX_NOTE_H.max(base_h));
        if ed.grow_h != Some(need) && (need - el.h).abs() > 0.5 {
            ed.grow_h = Some(need);
            el.h = need;
            if let Some(&i) = st.cache.index.get(&el.id)
                && let Some(c) = st.cache.els.get_mut(i)
            {
                c.h = need;
            }
        }
    }
    let Some(ed) = st.editing.as_mut() else { return };
    let r = cam.rect_to_screen(rect, st.cache.bounds(&el));
    let center = matches!(el.kind, Kind::Note | Kind::Rect | Kind::Ellipse);
    let (area, size, color) = match el.kind {
        Kind::Note => (r.shrink(NOTE_PAD * z), NOTE_FONT * z, style.note_text),
        Kind::Rect | Kind::Ellipse => (r.shrink(10.0 * z), SHAPE_FONT * z, style.text),
        Kind::Frame => (Rect::from_min_size(r.min - vec2(0.0, 22.0 * z), vec2(r.width().max(120.0), 20.0 * z)), FRAME_FONT * z, style.muted),
        _ => (Rect::from_min_size(r.min, vec2(r.width().max(160.0 * z), r.height().max(TEXT_FONT * z * 1.4))), TEXT_FONT * z, style.ink(&el.color)),
    };
    // Centred kinds: put the editor where the painted text would be (vertically centred).
    let area = if center {
        let probe = if ed.text.is_empty() { " " } else { ed.text.as_str() };
        let gh = ui.fonts_mut(|f| f.layout_job(text_job(probe, size.max(6.0), color, area.width(), true))).size().y;
        let extra = if ed.text.ends_with('\n') { size * 1.3 } else { 0.0 };
        let gh = gh + extra;
        if gh < area.height() { Rect::from_min_size(pos2(area.min.x, area.center().y - gh / 2.0), vec2(area.width(), gh)) } else { area }
    } else {
        area
    };
    let area = area.intersect(rect);
    if area.width() < 8.0 || area.height() < 6.0 {
        // Too small to type in: commit and stop.
        commit_edit(board, st);
        return;
    }
    let mut te = TextEdit::multiline(&mut ed.text)
        .id(edit_id)
        .font(FontId::proportional(size.max(6.0)))
        .text_color(color)
        .frame(Frame::NONE)
        .margin(Margin::ZERO)
        .background_color(Color32::TRANSPARENT)
        .desired_width(area.width())
        .desired_rows(1)
        .lock_focus(true)
        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
        .char_limit(MAX_TEXT);
    if center {
        te = te.align(Align2::CENTER_TOP);
    }
    let out = ui.put(area, te);
    let Some(ed) = st.editing.as_mut() else { return };
    if !ed.focused {
        out.request_focus();
        ed.focused = true;
    } else if out.lost_focus() || !out.has_focus() {
        commit_edit(board, st);
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
        Icon::Snap => {
            for gx in [-5.0_f32, 0.0, 5.0] {
                for gy in [-5.0_f32, 0.0, 5.0] {
                    painter.circle_filled(c + vec2(gx, gy), 1.3, color);
                }
            }
            painter.rect_stroke(Rect::from_min_size(c + vec2(-5.0, -5.0), vec2(5.0, 5.0)), 1.0, Stroke::new(1.6, color), StrokeKind::Middle);
        }
        Icon::Help => {
            painter.circle_stroke(c, 8.0, s);
            painter.text(c + vec2(0.0, 0.5), Align2::CENTER_CENTER, "?", FontId::proportional(12.5), color);
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
    let radius = if style.rounded { r.height() / 2.0 } else { 12.0 };
    if style.rounded {
        let shade = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_rgba_unmultiplied(0x0b, 0x2a, 0x45, 0x1c) };
        painter.add(shade.as_shape(r, egui::CornerRadius::same(radius.clamp(0.0, 255.0) as u8)));
    } else {
        painter.rect_filled(r.translate(vec2(0.0, 2.0)).expand(1.0), radius, Color32::from_black_alpha(22));
    }
    painter.rect_filled(r, radius, style.surface);
    painter.rect_stroke(r, radius, Stroke::new(1.0, style.border), StrokeKind::Inside);
}

#[allow(clippy::too_many_arguments)]
fn icon_button(ui: &Ui, painter: &Painter, r: Rect, id: Id, icon: Icon, tip: &str, active: bool, enabled: bool, style: &BoardStyle) -> bool {
    let resp = ui.interact(r, id, if enabled { Sense::click() } else { Sense::hover() }).on_hover_text(tip);
    let rad = if style.rounded { r.width() / 2.0 - 1.0 } else { 8.0 };
    if active && style.rounded {
        painter.rect_filled(r.shrink(1.0), rad, style.accent);
    } else if active {
        painter.rect_filled(r.shrink(2.0), rad, style.selection);
    } else if resp.hovered() && enabled {
        painter.rect_filled(r.shrink(2.0), rad, style.border.gamma_multiply(0.6));
    }
    let color = if !enabled {
        style.muted.gamma_multiply(0.45)
    } else if active && style.rounded {
        style.on_accent
    } else if active {
        style.accent
    } else {
        style.text
    };
    paint_icon(painter, icon, r.center(), color, style.accent);
    enabled && resp.clicked()
}

fn toolbar(ui: &mut Ui, painter: &Painter, rect: Rect, base: Id, board: &mut Board, st: &mut ViewState, style: &BoardStyle) {
    let gap = 10.0;
    let n_buttons = Tool::ALL.len() as f32 + 5.0;
    // Shrink the buttons in narrow panels so the bar always fits.
    let btn = ((rect.width() - 16.0 - 12.0 - 3.0 * gap) / n_buttons).clamp(24.0, 34.0);
    let width = btn * n_buttons + gap * 3.0 + 12.0;
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
        fit(st, rect, MIN_ZOOM);
    }
    x += btn;
    let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
    let snap_tip = if st.snap { "Am Raster einrasten: an (Alt beim Ziehen: frei)" } else { "Am Raster einrasten: aus" };
    if icon_button(ui, painter, r, base.with("snap"), Icon::Snap, snap_tip, st.snap, true, style) {
        st.snap = !st.snap;
    }
    x += btn;
    sep(painter, x + gap / 2.0);
    x += gap;
    let r = Rect::from_min_size(pos2(x, y), Vec2::splat(btn));
    if icon_button(ui, painter, r, base.with("help"), Icon::Help, "Tastenkürzel (F1 oder ?)", st.help, true, style) {
        st.help = !st.help;
    }

    // What the active tool does, right under the bar.
    if st.tool != Tool::Select && !st.help {
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(st.tool.hint().to_string(), FontId::proportional(12.5), style.text));
        let chip = Rect::from_center_size(pos2(bar.center().x, bar.max.y + 18.0), galley.size() + vec2(20.0, 10.0));
        painter.rect_filled(chip, 10.0, style.selection);
        painter.rect_stroke(chip, 10.0, Stroke::new(1.0, style.accent.gamma_multiply(0.6)), StrokeKind::Inside);
        painter.galley(chip.center() - galley.size() / 2.0, galley, style.text);
    }

    // Bottom bar: colour for new elements (and the selection), zoom.
    let sw = 26.0;
    let zoom_w = 58.0;
    let width = sw * 9.0 + 12.0 + gap + zoom_w;
    let x0 = (rect.center().x - width / 2.0).max(rect.min.x + 8.0);
    let bar = Rect::from_min_size(pos2(x0, rect.max.y - 10.0 - (btn + 8.0)), vec2(width, btn + 8.0));
    bar_background(painter, bar, style);
    let current = st.selection.first().and_then(|id| st.cache.get(id)).map(|e| e.color.clone()).or_else(|| st.color.clone());
    let mut x = bar.min.x + 6.0;
    let cy = bar.center().y;
    for name in std::iter::once("ink").chain(PALETTE) {
        let r = Rect::from_center_size(pos2(x + sw / 2.0, cy), Vec2::splat(sw));
        if swatch(ui, painter, r, base.with(("color", name)), name, current.as_deref() == Some(name), style) {
            st.color = Some(name.to_string());
            if !st.selection.is_empty() {
                let ids = st.selection.clone();
                board.set_color(&ids, name);
            }
        }
        x += sw;
    }
    x += 6.0;
    let sep = |painter: &Painter, x: f32| painter.line_segment([pos2(x, bar.min.y + 11.0), pos2(x, bar.max.y - 11.0)], Stroke::new(1.0, style.border));
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

/// One colour swatch; returns whether it was clicked.
fn swatch(ui: &Ui, painter: &Painter, r: Rect, id: Id, name: &str, active: bool, style: &BoardStyle) -> bool {
    let resp = ui.interact(r, id, Sense::click()).on_hover_text(color_label(name));
    let fill = if name == "ink" { style.text } else { style.fill(name) };
    let rad = r.width() * 0.42;
    if active {
        painter.circle_stroke(r.center(), rad, Stroke::new(2.0, style.accent));
    } else if resp.hovered() {
        painter.circle_stroke(r.center(), rad, Stroke::new(1.0, style.border));
    }
    painter.circle(r.center(), rad - 3.0, fill, Stroke::new(1.0, mix(fill, style.text, 0.25)));
    resp.clicked()
}

/// Floating bar next to the selection: colours, duplicate, to front, to back, delete.
fn context_bar(ui: &mut Ui, painter: &Painter, rect: Rect, base: Id, board: &mut Board, st: &mut ViewState, style: &BoardStyle) {
    if st.selection.is_empty() || st.editing.is_some() || !matches!(st.interaction, Interaction::None) {
        return;
    }
    let Some(b) = selection_bounds(st) else { return };
    let sb = st.camera.rect_to_screen(rect, b);
    let (sw, btn, gap) = (24.0, 30.0, 10.0);
    let width = sw * 9.0 + gap + btn * 4.0 + 12.0;
    let height = btn + 8.0;
    // Above the selection, clear of its „+“ handle; below it when there is no room on top.
    let room = if plus_source(st).is_some() { PLUS_OFFSET + PLUS_R + 8.0 } else { 12.0 };
    let top_limit = rect.min.y + 64.0;
    let y = if sb.min.y - room - height >= top_limit { sb.min.y - room - height } else { sb.max.y + room };
    let y = y.clamp(top_limit, (rect.max.y - 64.0 - height).max(top_limit));
    let x = (sb.center().x - width / 2.0).clamp(rect.min.x + 8.0, (rect.max.x - 8.0 - width).max(rect.min.x + 8.0));
    let bar = Rect::from_min_size(pos2(x, y), vec2(width, height));
    if !bar.intersects(rect) {
        return;
    }
    bar_background(painter, bar, style);
    let current = st.selection.first().and_then(|id| st.cache.get(id)).map(|e| e.color.clone());
    let mut x = bar.min.x + 6.0;
    let cy = bar.center().y;
    for name in std::iter::once("ink").chain(PALETTE) {
        let r = Rect::from_center_size(pos2(x + sw / 2.0, cy), Vec2::splat(sw));
        if swatch(ui, painter, r, base.with(("ctx-color", name)), name, current.as_deref() == Some(name), style) {
            let ids = st.selection.clone();
            board.set_color(&ids, name);
        }
        x += sw;
    }
    painter.line_segment([pos2(x + gap / 2.0, bar.min.y + 10.0), pos2(x + gap / 2.0, bar.max.y - 10.0)], Stroke::new(1.0, style.border));
    x += gap;
    let actions = [
        (Icon::Duplicate, "Duplizieren (Strg+D)", "dup"),
        (Icon::Front, "Nach vorne", "front"),
        (Icon::Back, "Nach hinten", "back"),
        (Icon::Delete, "Löschen (Entf)", "del"),
    ];
    for (icon, tip, key) in actions {
        let r = Rect::from_min_size(pos2(x, bar.min.y + 4.0), Vec2::splat(btn));
        if icon_button(ui, painter, r, base.with(("ctx", key)), icon, tip, false, true, style) {
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

/// Empty state of the designed looks: the note symbol in an accent circle with a tinted halo,
/// "Noch leer", one sentence, and two ways to start: a note in the middle, or a mind map from
/// the chat (handed to the app through `ViewState::wants_mindmap`).
fn empty_start(painter: &Painter, ui: &Ui, rect: Rect, base: Id, board: &mut Board, st: &mut ViewState, style: &BoardStyle) {
    let title_font = FontId::new(22.0, style.title_family.clone().unwrap_or(egui::FontFamily::Proportional));
    let title = ui.fonts_mut(|f| f.layout_no_wrap("Noch leer".to_string(), title_font, style.text));
    let wrap = (rect.width() - 64.0).clamp(160.0, 330.0);
    let body = ui.fonts_mut(|f| {
        f.layout_job(text_job("Doppelklick legt eine Notiz an. Oder lass die KI aus einem Chat eine Mindmap bauen.", 14.5, style.muted, wrap, true))
    });
    let foot = ui.fonts_mut(|f| f.layout_no_wrap("F1 zeigt alle Tastenkürzel".to_string(), FontId::proportional(12.5), style.muted));
    let labels = ["Notiz anlegen", "Mindmap aus Chat"];
    let galleys: Vec<_> = labels.iter().map(|l| ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), FontId::proportional(13.5), style.text))).collect();
    let btn_w: Vec<f32> = galleys.iter().map(|g| g.size().x + 26.0 + 20.0).collect();
    let h = 72.0 + 12.0 + title.size().y + 10.0 + body.size().y + 14.0 + 34.0 + 10.0 + foot.size().y;
    let mut y = rect.center().y - h / 2.0;
    let cx = rect.center().x;
    painter.rect_filled(Rect::from_center_size(rect.center(), vec2(wrap + 48.0, h + 40.0)), 24.0, style.bg.gamma_multiply(0.92));
    let c = pos2(cx, y + 36.0);
    painter.circle_filled(c, 36.0, style.accent_soft);
    painter.circle_filled(c, 24.0, style.accent);
    paint_icon(painter, Icon::Tool(Tool::Note), c, style.on_accent, style.on_accent);
    y += 72.0 + 12.0;
    let th = title.size();
    painter.galley(pos2(cx - th.x / 2.0, y), title, style.text);
    y += th.y + 10.0;
    let bh = body.size().y;
    painter.galley(pos2(cx, y), body, style.muted);
    y += bh + 14.0;
    let total = btn_w.iter().sum::<f32>() + 8.0;
    let mut x = cx - total / 2.0;
    for (i, g) in galleys.into_iter().enumerate() {
        let w = btn_w.get(i).copied().unwrap_or(120.0);
        let r = Rect::from_min_size(pos2(x, y), vec2(w, 34.0));
        let resp = ui.interact(r, base.with(("empty-start", i)), Sense::click());
        let label = labels.get(i).copied().unwrap_or_default();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        let (fill, fg, stroke) = if i == 0 { (style.accent_soft, style.text, Color32::TRANSPARENT) } else { (style.surface, style.text, style.border) };
        painter.rect(r, 17.0, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
        if resp.hovered() {
            painter.rect_filled(r, 17.0, style.text.gamma_multiply(0.06));
        }
        let ic = pos2(r.left() + 20.0, r.center().y);
        if i == 0 {
            painter.line_segment([ic + vec2(-5.0, 0.0), ic + vec2(5.0, 0.0)], Stroke::new(1.8, fg));
            painter.line_segment([ic + vec2(0.0, -5.0), ic + vec2(0.0, 5.0)], Stroke::new(1.8, fg));
        } else {
            // A small four-pointed spark.
            let pts = vec![
                ic + vec2(0.0, -6.0),
                ic + vec2(1.6, -1.6),
                ic + vec2(6.0, 0.0),
                ic + vec2(1.6, 1.6),
                ic + vec2(0.0, 6.0),
                ic + vec2(-1.6, 1.6),
                ic + vec2(-6.0, 0.0),
                ic + vec2(-1.6, -1.6),
            ];
            painter.add(Shape::closed_line(pts, Stroke::new(1.5, fg)));
        }
        painter.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, fg);
        if resp.clicked() {
            if i == 0 {
                let w = st.camera.to_world(rect, rect.center());
                create_at(board, st, Kind::Note, w);
            } else {
                st.wants_mindmap = true;
            }
        }
        x += w + 8.0;
    }
    y += 34.0 + 10.0;
    let fw = foot.size().x;
    painter.galley(pos2(cx - fw / 2.0, y), foot, style.muted);
}

/// Empty state of a board (design review 2026-11): a symbol in a tonal circle, one sentence on
/// the quickest start and where mind maps come from; the shortcut list stays behind F1.
fn empty_hint(painter: &Painter, ui: &Ui, rect: Rect, style: &BoardStyle) {
    let lines = [
        ("Noch leer", 19.0, style.text),
        ("Doppelklick auf die Fläche legt eine Notiz an.\nOder unter einer Chat-Antwort „Als Whiteboard“ wählen: daraus wird eine Mindmap.", 14.0, style.muted),
        ("Werkzeuge oben · F1 zeigt alle Tastenkürzel", 12.5, style.muted.gamma_multiply(0.85)),
    ];
    let wrap = (rect.width() - 64.0).clamp(120.0, 420.0);
    let galleys: Vec<_> = lines.iter().map(|(t, size, color)| ui.fonts_mut(|f| f.layout_job(text_job(t, *size, *color, wrap, true)))).collect();
    let icon = 56.0;
    let h: f32 = icon + 16.0 + galleys.iter().map(|g| g.size().y).sum::<f32>() + 10.0 * (galleys.len() as f32 - 1.0);
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    // A soft backdrop so the dot grid does not run through the text.
    painter.rect_filled(Rect::from_center_size(rect.center(), vec2(w + 48.0, h + 40.0)), 20.0, style.bg.gamma_multiply(0.9));
    let mut y = rect.center().y - h / 2.0;
    let c = pos2(rect.center().x, y + icon / 2.0);
    painter.circle_filled(c, icon / 2.0, style.accent.gamma_multiply(0.14));
    paint_icon(painter, Icon::Tool(Tool::Note), c, style.accent, style.accent);
    y += icon + 16.0;
    for g in galleys {
        let gh = g.size().y;
        painter.galley(pos2(rect.center().x, y), g, style.text);
        y += gh + 10.0;
    }
}

const SHORTCUTS: [(&str, &str); 18] = [
    ("V · H · N · R · O", "Auswählen · Hand · Notiz · Rechteck · Ellipse"),
    ("A · L · T · P · E · F", "Pfeil · Linie · Text · Stift · Radierer · Rahmen"),
    ("Doppelklick", "Notiz anlegen oder Text bearbeiten"),
    ("Enter", "Text bearbeiten / fertig"),
    ("Umschalt+Enter", "Neue Zeile"),
    ("Tab beim Schreiben", "Verbundene Unteridee anlegen"),
    ("„+“ an einer Notiz", "Klicken: verbundene Notiz · Ziehen: verbinden"),
    ("Pfeilende ziehen", "Pfeil an ein anderes Element hängen"),
    ("Entf", "Auswahl löschen"),
    ("Strg+D", "Duplizieren"),
    ("Strg+C · X · V", "Kopieren · Ausschneiden · Einfügen"),
    ("Strg+A", "Alles auswählen"),
    ("Strg+Z · Strg+Umschalt+Z", "Rückgängig · Wiederholen"),
    ("Pfeiltasten", "Auswahl verschieben (Umschalt: 10er-Schritte)"),
    ("Alt beim Ziehen", "Ohne Einrasten verschieben"),
    ("Leertaste + Ziehen", "Ansicht verschieben"),
    ("Strg+Mausrad · Umschalt+1 · Strg+0", "Zoomen · Alles zeigen · 100 %"),
    ("Esc · F1 · ?", "Hilfe schließen"),
];

/// Overlay listing all shortcuts.
fn help_overlay(ui: &Ui, painter: &Painter, rect: Rect, style: &BoardStyle) {
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(70));
    let key_w = 210.0_f32.min(rect.width() * 0.38);
    let desc_w = (rect.width() - 48.0 - key_w - 24.0).clamp(120.0, 320.0);
    let rows: Vec<_> = SHORTCUTS
        .iter()
        .map(|(k, d)| {
            let kg = ui.fonts_mut(|f| f.layout(k.to_string(), FontId::proportional(13.0), style.accent, key_w));
            let dg = ui.fonts_mut(|f| f.layout(d.to_string(), FontId::proportional(13.0), style.text, desc_w));
            (kg, dg)
        })
        .collect();
    let row_h: Vec<f32> = rows.iter().map(|(k, d)| k.size().y.max(d.size().y) + 7.0).collect();
    let title_h = 34.0;
    let h = title_h + row_h.iter().sum::<f32>() + 24.0;
    let w = key_w + desc_w + 16.0 + 40.0;
    let card = Rect::from_center_size(rect.center(), vec2(w, h.min(rect.height() - 16.0)));
    painter.rect_filled(card.translate(vec2(0.0, 3.0)), 16.0, Color32::from_black_alpha(40));
    painter.rect_filled(card, 16.0, style.surface);
    painter.rect_stroke(card, 16.0, Stroke::new(1.0, style.border), StrokeKind::Inside);
    let clip = painter.with_clip_rect(card.intersect(rect));
    clip.text(card.min + vec2(20.0, 16.0), Align2::LEFT_TOP, "Tastenkürzel", FontId::proportional(17.0), style.text);
    let mut y = card.min.y + 16.0 + title_h;
    for ((kg, dg), rh) in rows.into_iter().zip(row_h) {
        clip.galley(pos2(card.min.x + 20.0, y), kg, style.accent);
        clip.galley(pos2(card.min.x + 20.0 + key_w + 16.0, y), dg, style.text);
        y += rh;
    }
}
