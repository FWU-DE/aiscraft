//! The board document: a `yrs` (Yjs) CRDT with typed helpers on top.
//!
//! Layout of the shared document (compatible with any Yjs client):
//! - root map `elements`: element id → nested map with the fields of [`Element`]
//! - root map `meta`: `title`
//!
//! Everything read from the document is treated as untrusted: wrong types and missing fields
//! are skipped or replaced by defaults, numbers are clamped, strings and lists are capped.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use yrs::types::ToJson;
use yrs::undo::{Options as UndoOptions, UndoManager};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Any, Doc, In, Map, MapPrelim, MapRef, Origin, Out, ReadTxn, StateVector, Transact, TransactionMut, Update};

/// Elements read from one document at most; the rest is ignored.
pub const MAX_ELEMENTS: usize = 20_000;
/// Characters of text per element.
pub const MAX_TEXT: usize = 10_000;
/// Coordinates per element (`x, y` pairs flattened).
pub const MAX_POINTS: usize = 20_000;
/// Length of element ids, colour names and references.
pub const MAX_ID: usize = 96;
/// Largest update or state blob accepted.
pub const MAX_UPDATE_BYTES: usize = 64 << 20;
/// Positions are clamped to ±this.
pub const COORD_LIMIT: f64 = 1.0e7;
/// Widths and heights are clamped to `0..=this`.
pub const SIZE_LIMIT: f64 = 1.0e6;
const Z_LIMIT: f64 = 1.0e12;
const MAX_FIELDS: usize = 32;
const MAX_TITLE: usize = 300;

/// Origin of transactions made through this API (tracked by undo).
const LOCAL_ORIGIN: u64 = 0x4149_5343_4c4f_4341;
/// Origin of updates received from elsewhere (never undone locally).
const REMOTE_ORIGIN: u64 = 0x4149_5343_5245_4d4f;

static INSTANCES: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("Ungültige Whiteboard-Daten: {0}")]
    Decode(String),
    #[error("Whiteboard-Änderung konnte nicht übernommen werden: {0}")]
    Apply(String),
    #[error("Whiteboard-Daten sind zu groß ({0} Bytes)")]
    TooLarge(usize),
    #[error("Element nicht gefunden: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, BoardError>;

/// What an element is. Stored as a lowercase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Note,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Text,
    Freehand,
    Frame,
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Note, Kind::Rect, Kind::Ellipse, Kind::Arrow, Kind::Line, Kind::Text, Kind::Freehand, Kind::Frame];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Rect => "rect",
            Kind::Ellipse => "ellipse",
            Kind::Arrow => "arrow",
            Kind::Line => "line",
            Kind::Text => "text",
            Kind::Freehand => "freehand",
            Kind::Frame => "frame",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// German name for summaries and tooltips.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Note => "Notiz",
            Kind::Rect => "Rechteck",
            Kind::Ellipse => "Ellipse",
            Kind::Arrow => "Pfeil",
            Kind::Line => "Linie",
            Kind::Text => "Text",
            Kind::Freehand => "Zeichnung",
            Kind::Frame => "Rahmen",
        }
    }

    /// Lines, arrows and drawings: geometry lives in `points`.
    pub fn is_path(self) -> bool {
        matches!(self, Kind::Arrow | Kind::Line | Kind::Freehand)
    }

    /// Kinds that show editable text inside their box.
    pub fn has_text(self) -> bool {
        matches!(self, Kind::Note | Kind::Rect | Kind::Ellipse | Kind::Text | Kind::Frame)
    }

    pub fn default_color(self) -> &'static str {
        match self {
            Kind::Note => "yellow",
            Kind::Frame => "gray",
            _ => "ink",
        }
    }

    fn default_size(self) -> (f64, f64) {
        match self {
            Kind::Note => (200.0, 140.0),
            Kind::Frame => (480.0, 320.0),
            Kind::Text => (240.0, 40.0),
            Kind::Rect | Kind::Ellipse => (160.0, 100.0),
            Kind::Arrow | Kind::Line | Kind::Freehand => (0.0, 0.0),
        }
    }
}

/// One element as plain Rust data.
///
/// Positions are world units. For paths (`arrow`, `line`, `freehand`) `points` holds
/// `x0, y0, x1, y1, …` relative to `x, y`, and `w, h` is their bounding box. An arrow's
/// `from` / `to` bind its ends to other elements; the points are the fallback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Palette name (`yellow`, `blue`, … or `ink`), never a hex value.
    pub color: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub points: Vec<f64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub z: f64,
}

impl Element {
    /// A new element with the kind's default size and colour. The id is assigned by [`Board::add`].
    pub fn new(kind: Kind, x: f64, y: f64) -> Self {
        let (w, h) = kind.default_size();
        Self { id: String::new(), kind, x, y, w, h, color: kind.default_color().into(), text: String::new(), points: Vec::new(), from: None, to: None, z: 0.0 }
    }

    pub fn with_size(mut self, w: f64, h: f64) -> Self {
        self.w = w;
        self.h = h;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = color.into();
        self
    }

    /// A line or arrow from `a` to `b` (world coordinates).
    pub fn segment(kind: Kind, a: (f64, f64), b: (f64, f64)) -> Self {
        Self::path(kind, &[a, b])
    }

    /// A path through world points; `x, y, w, h` become its bounding box.
    pub fn path(kind: Kind, pts: &[(f64, f64)]) -> Self {
        let mut e = Element::new(kind, 0.0, 0.0);
        e.set_world_points(pts);
        e
    }

    /// An arrow bound to two elements.
    pub fn connector(from: &Element, to: &Element) -> Self {
        let mut e = Element::segment(Kind::Arrow, from.center(), to.center());
        e.from = Some(from.id.clone());
        e.to = Some(to.id.clone());
        e
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Path points in world coordinates.
    pub fn world_points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.points.as_chunks::<2>().0.iter().map(|[a, b]| (self.x + a, self.y + b))
    }

    /// Replace the path with world points; recomputes `x, y, w, h`.
    pub fn set_world_points(&mut self, pts: &[(f64, f64)]) {
        let pts: Vec<(f64, f64)> = pts.iter().take(MAX_POINTS / 2).copied().filter(|(a, b)| a.is_finite() && b.is_finite()).collect();
        let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        if pts.is_empty() {
            self.points.clear();
            self.w = 0.0;
            self.h = 0.0;
            return;
        }
        self.x = min_x;
        self.y = min_y;
        self.w = max_x - min_x;
        self.h = max_y - min_y;
        self.points = pts.iter().flat_map(|(a, b)| [a - min_x, b - min_y]).collect();
    }

    /// Clamp everything into safe ranges; drop non-finite values.
    pub fn sanitize(&mut self) {
        self.x = clamp_coord(self.x);
        self.y = clamp_coord(self.y);
        self.w = clamp_size(self.w);
        self.h = clamp_size(self.h);
        self.z = if self.z.is_finite() { self.z.clamp(-Z_LIMIT, Z_LIMIT) } else { 0.0 };
        truncate_chars(&mut self.text, MAX_TEXT);
        if self.color.is_empty() || self.color.len() > MAX_ID {
            self.color = self.kind.default_color().into();
        }
        self.points.truncate(MAX_POINTS);
        if self.points.len() % 2 == 1 {
            self.points.pop();
        }
        for p in &mut self.points {
            *p = clamp_size_signed(*p);
        }
        for r in [&mut self.from, &mut self.to] {
            if r.as_ref().is_some_and(|s| s.is_empty() || s.len() > MAX_ID) {
                *r = None;
            }
        }
        if self.kind != Kind::Arrow {
            self.from = None;
            self.to = None;
        }
    }
}

fn clamp_coord(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-COORD_LIMIT, COORD_LIMIT) } else { 0.0 }
}

fn clamp_size(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, SIZE_LIMIT) } else { 0.0 }
}

fn clamp_size_signed(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-SIZE_LIMIT, SIZE_LIMIT) } else { 0.0 }
}

pub(crate) fn truncate_chars(s: &mut String, max: usize) {
    if let Some((i, _)) = s.char_indices().nth(max) {
        s.truncate(i);
    }
}

/// Where an arrow's ends are, following bound elements when they exist.
pub fn arrow_endpoints<'a>(el: &Element, find: impl Fn(&str) -> Option<&'a Element>) -> ((f64, f64), (f64, f64)) {
    let mut pts = el.world_points();
    let first = pts.next().unwrap_or((el.x, el.y));
    let last = pts.last().unwrap_or((el.x + el.w, el.y + el.h));
    let from = el.from.as_deref().and_then(&find).filter(|e| !e.kind.is_path());
    let to = el.to.as_deref().and_then(&find).filter(|e| !e.kind.is_path());
    let a_target = to.map(|t| t.center()).unwrap_or(last);
    let b_target = from.map(|f| f.center()).unwrap_or(first);
    let a = from.map(|f| boundary_point(f, a_target)).unwrap_or(first);
    let b = to.map(|t| boundary_point(t, b_target)).unwrap_or(last);
    (a, b)
}

/// Point on the outline of `el` in the direction of `toward`, a few units outside.
fn boundary_point(el: &Element, toward: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = el.center();
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let len = (dx * dx + dy * dy).sqrt();
    let (hw, hh) = (el.w / 2.0, el.h / 2.0);
    if len < 1e-9 || hw <= 0.0 || hh <= 0.0 {
        return (cx, cy);
    }
    let t = if el.kind == Kind::Ellipse {
        1.0 / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt()
    } else {
        let tx = if dx.abs() > 1e-9 { hw / dx.abs() } else { f64::INFINITY };
        let ty = if dy.abs() > 1e-9 { hh / dy.abs() } else { f64::INFINITY };
        tx.min(ty)
    };
    if !t.is_finite() || t >= 1.0 {
        return (cx, cy);
    }
    let gap = 6.0 / len;
    (cx + dx * (t + gap), cy + dy * (t + gap))
}

// ----- reading untrusted CRDT values --------------------------------------------------------

fn any_f64(a: &Any) -> Option<f64> {
    match a {
        Any::Number(n) => n.as_f64().filter(|v| v.is_finite()),
        _ => None,
    }
}

fn any_str(a: &Any, max_chars: usize) -> Option<String> {
    match a {
        Any::String(s) => Some(s.chars().take(max_chars).collect()),
        _ => None,
    }
}

fn fields_of<T: ReadTxn>(txn: &T, value: &Out) -> Option<HashMap<String, Any>> {
    match value {
        Out::YMap(m) => Some(
            m.iter(txn)
                .take(MAX_FIELDS * 4)
                .filter_map(|(k, v)| match v {
                    Out::Any(a) if k.len() <= MAX_ID => Some((k.to_string(), a)),
                    _ => None,
                })
                .take(MAX_FIELDS)
                .collect(),
        ),
        Out::Any(Any::Map(m)) => Some(m.iter().filter(|(k, _)| k.len() <= MAX_ID).take(MAX_FIELDS).map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => None,
    }
}

fn parse_element(id: &str, f: &HashMap<String, Any>) -> Option<Element> {
    if id.is_empty() || id.len() > MAX_ID {
        return None;
    }
    let kind = f.get("kind").and_then(|a| any_str(a, MAX_ID)).and_then(|k| Kind::parse(&k))?;
    let num = |k: &str| f.get(k).and_then(any_f64);
    let (dw, dh) = kind.default_size();
    let points = match f.get("points") {
        Some(Any::Array(a)) => a.iter().take(MAX_POINTS).map(|v| any_f64(v).unwrap_or(0.0)).collect(),
        _ => Vec::new(),
    };
    let mut e = Element {
        id: id.to_string(),
        kind,
        x: num("x").unwrap_or(0.0),
        y: num("y").unwrap_or(0.0),
        w: num("w").unwrap_or(dw),
        h: num("h").unwrap_or(dh),
        color: f.get("color").and_then(|a| any_str(a, MAX_ID)).unwrap_or_else(|| kind.default_color().into()),
        text: f.get("text").and_then(|a| any_str(a, MAX_TEXT)).unwrap_or_default(),
        points,
        from: f.get("from").and_then(|a| any_str(a, MAX_ID + 1)),
        to: f.get("to").and_then(|a| any_str(a, MAX_ID + 1)),
        z: num("z").unwrap_or(0.0),
    };
    e.sanitize();
    Some(e)
}

fn read_one<T: ReadTxn>(map: &MapRef, txn: &T, id: &str) -> Option<Element> {
    let v = map.get(txn, id)?;
    parse_element(id, &fields_of(txn, &v)?)
}

fn read_all<T: ReadTxn>(map: &MapRef, txn: &T) -> Vec<Element> {
    let mut out: Vec<Element> = map.iter(txn).take(MAX_ELEMENTS).filter_map(|(id, v)| fields_of(txn, &v).and_then(|f| parse_element(id, &f))).collect();
    out.sort_by(|a, b| a.z.total_cmp(&b.z).then_with(|| a.id.cmp(&b.id)));
    out
}

fn points_any(points: &[f64]) -> Any {
    Any::Array(points.iter().map(|p| Any::from(*p)).collect::<Vec<Any>>().into())
}

fn full_fields(e: &Element) -> Vec<(&'static str, In)> {
    let mut v: Vec<(&'static str, In)> = vec![
        ("kind", In::Any(Any::from(e.kind.as_str()))),
        ("x", In::Any(Any::from(e.x))),
        ("y", In::Any(Any::from(e.y))),
        ("w", In::Any(Any::from(e.w))),
        ("h", In::Any(Any::from(e.h))),
        ("color", In::Any(Any::from(e.color.as_str()))),
        ("text", In::Any(Any::from(e.text.as_str()))),
        ("z", In::Any(Any::from(e.z))),
    ];
    if !e.points.is_empty() {
        v.push(("points", In::Any(points_any(&e.points))));
    }
    if let Some(f) = &e.from {
        v.push(("from", In::Any(Any::from(f.as_str()))));
    }
    if let Some(t) = &e.to {
        v.push(("to", In::Any(Any::from(t.as_str()))));
    }
    v
}

/// Write `new` over `old` inside an open transaction, touching only changed fields.
fn write_element(elements: &MapRef, txn: &mut TransactionMut, old: &Element, new: &Element) {
    let map = match elements.get(txn, &new.id) {
        Some(Out::YMap(m)) => m,
        // Plain-JSON or broken entries are replaced by a proper nested map.
        _ => {
            elements.insert(txn, new.id.as_str(), MapPrelim::from_iter(full_fields(new)));
            return;
        }
    };
    if old.kind != new.kind {
        map.insert(txn, "kind", new.kind.as_str());
    }
    for (k, a, b) in [("x", old.x, new.x), ("y", old.y, new.y), ("w", old.w, new.w), ("h", old.h, new.h), ("z", old.z, new.z)] {
        if a.to_bits() != b.to_bits() {
            map.insert(txn, k, b);
        }
    }
    if old.color != new.color {
        map.insert(txn, "color", new.color.as_str());
    }
    if old.text != new.text {
        map.insert(txn, "text", new.text.as_str());
    }
    if old.points != new.points {
        if new.points.is_empty() {
            map.remove(txn, "points");
        } else {
            map.insert(txn, "points", points_any(&new.points));
        }
    }
    for (k, a, b) in [("from", &old.from, &new.from), ("to", &old.to, &new.to)] {
        if a != b {
            match b {
                Some(v) => {
                    map.insert(txn, k, v.as_str());
                }
                None => {
                    map.remove(txn, k);
                }
            }
        }
    }
}

fn fresh_id<T: ReadTxn>(elements: &MapRef, txn: &T, client: u64, next: &mut u64, taken: &HashSet<String>) -> String {
    loop {
        *next = next.wrapping_add(1);
        let id = format!("{client:x}-{next}");
        if !taken.contains(&id) && elements.get(txn, &id).is_none() {
            return id;
        }
    }
}

fn decode_update(data: &[u8]) -> Result<Update> {
    if data.len() > MAX_UPDATE_BYTES {
        return Err(BoardError::TooLarge(data.len()));
    }
    // The decoder is pure; guard against panics on hostile input anyway.
    match catch_unwind(AssertUnwindSafe(|| Update::decode_v1(data))) {
        Ok(Ok(u)) => Ok(u),
        Ok(Err(e)) => Err(BoardError::Decode(e.to_string())),
        Err(_) => Err(BoardError::Decode("Dekodierung abgebrochen".into())),
    }
}

/// Plain JSON view of a board, see [`Board::to_json`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardJson {
    pub title: String,
    pub elements: Vec<Element>,
}

/// A shared whiteboard document.
pub struct Board {
    doc: Doc,
    elements: MapRef,
    meta: MapRef,
    undo: UndoManager<()>,
    instance: u64,
    revision: u64,
    next_id: u64,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Board").field("client_id", &self.doc.client_id()).field("revision", &self.revision).finish_non_exhaustive()
    }
}

impl Board {
    pub fn new() -> Self {
        let doc = Doc::new();
        let elements = doc.get_or_insert_map("elements");
        let meta = doc.get_or_insert_map("meta");
        let mut undo = UndoManager::with_options(UndoOptions { capture_timeout_millis: 500, ..UndoOptions::default() });
        undo.expand_scope(&doc, &elements);
        undo.expand_scope(&doc, &meta);
        undo.include_origin(Origin::from(LOCAL_ORIGIN));
        Self { doc, elements, meta, undo, instance: INSTANCES.fetch_add(1, Ordering::Relaxed), revision: 0, next_id: 0 }
    }

    /// Load a board from a full state (as written by [`Board::encode_state`]).
    pub fn from_state(state: &[u8]) -> Result<Board> {
        let mut b = Board::new();
        b.apply_update(state)?;
        Ok(b)
    }

    /// The whole document as one v1 update.
    pub fn encode_state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    /// The document's state vector (v1-encoded), to ask a peer for what is missing.
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Everything a peer with state vector `sv` is missing, as one v1 update.
    pub fn encode_diff(&self, sv: &[u8]) -> Result<Vec<u8>> {
        if sv.len() > MAX_UPDATE_BYTES {
            return Err(BoardError::TooLarge(sv.len()));
        }
        let sv = match catch_unwind(AssertUnwindSafe(|| StateVector::decode_v1(sv))) {
            Ok(Ok(sv)) => sv,
            Ok(Err(e)) => return Err(BoardError::Decode(e.to_string())),
            Err(_) => return Err(BoardError::Decode("Dekodierung abgebrochen".into())),
        };
        Ok(self.doc.transact().encode_state_as_update_v1(&sv))
    }

    /// Apply an update from a peer or a saved state. Not recorded for local undo.
    pub fn apply_update(&mut self, update: &[u8]) -> Result<()> {
        let u = decode_update(update)?;
        let mut txn = self.doc.transact_mut_with(Origin::from(REMOTE_ORIGIN));
        let r = txn.apply_update(u).map_err(|e| BoardError::Apply(e.to_string()));
        drop(txn);
        self.revision += 1;
        r
    }

    /// Changes whenever the document may have changed; use it to cache derived data.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Unique per `Board` value in this process (together with `revision`, a cache key).
    pub fn instance(&self) -> u64 {
        self.instance
    }

    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    fn txn(&self) -> TransactionMut<'_> {
        self.doc.transact_mut_with(Origin::from(LOCAL_ORIGIN))
    }

    fn touched(&mut self) {
        self.revision += 1;
    }

    // ----- reading ----------------------------------------------------------------------

    /// All valid elements, bottom to top.
    pub fn elements(&self) -> Vec<Element> {
        read_all(&self.elements, &self.doc.transact())
    }

    pub fn element(&self, id: &str) -> Option<Element> {
        read_one(&self.elements, &self.doc.transact(), id)
    }

    pub fn len(&self) -> usize {
        self.elements.len(&self.doc.transact()) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn title(&self) -> String {
        let txn = self.doc.transact();
        match self.meta.get(&txn, "title") {
            Some(Out::Any(a)) => any_str(&a, MAX_TITLE).unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.checkpoint();
        let mut t = title.to_string();
        truncate_chars(&mut t, MAX_TITLE);
        let mut txn = self.txn();
        self.meta.insert(&mut txn, "title", t);
        drop(txn);
        self.touched();
    }

    /// Plain JSON: `{"title": …, "elements": [ … ]}`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&BoardJson { title: self.title(), elements: self.elements() }).unwrap_or_else(|_| "{}".into())
    }

    /// The raw CRDT content as JSON (whatever peers wrote), for debugging.
    pub fn to_raw_json(&self) -> String {
        let txn = self.doc.transact();
        let mut s = String::new();
        self.elements.to_json(&txn).to_json(&mut s);
        s
    }

    // ----- writing ----------------------------------------------------------------------

    /// Start a new undo step: the next change is not merged into the previous one.
    pub fn checkpoint(&mut self) {
        self.undo.reset();
    }

    /// A fresh id that is not in use, to link elements (an arrow to a new note) before adding
    /// them together with [`Board::add_many`].
    pub fn fresh_element_id(&mut self) -> String {
        let client = self.client_id();
        let txn = self.doc.transact();
        fresh_id(&self.elements, &txn, client, &mut self.next_id, &HashSet::new())
    }

    /// Add an element on top; returns its id (the given one if free, else a new one).
    pub fn add(&mut self, el: Element) -> String {
        self.add_many(vec![el]).into_iter().next().unwrap_or_default()
    }

    /// Add several elements in one undo step, on top, keeping their relative order.
    /// Ids already in use are replaced, and `from` / `to` references inside the batch follow.
    pub fn add_many(&mut self, els: Vec<Element>) -> Vec<String> {
        self.checkpoint();
        let client = self.client_id();
        let mut txn = self.doc.transact_mut_with(Origin::from(LOCAL_ORIGIN));
        let mut z = read_all(&self.elements, &txn).last().map(|e| e.z).unwrap_or(0.0);
        let mut taken = HashSet::new();
        let mut remap: HashMap<String, String> = HashMap::new();
        let mut prepared = Vec::with_capacity(els.len().min(MAX_ELEMENTS));
        for mut e in els.into_iter().take(MAX_ELEMENTS) {
            let wanted = std::mem::take(&mut e.id);
            let id = if !wanted.is_empty() && wanted.len() <= MAX_ID && !taken.contains(&wanted) && self.elements.get(&txn, &wanted).is_none() {
                wanted.clone()
            } else {
                fresh_id(&self.elements, &txn, client, &mut self.next_id, &taken)
            };
            taken.insert(id.clone());
            if !wanted.is_empty() {
                remap.insert(wanted, id.clone());
            }
            e.id = id;
            prepared.push(e);
        }
        let mut ids = Vec::with_capacity(prepared.len());
        for mut e in prepared {
            for r in [&mut e.from, &mut e.to] {
                if let Some(new) = r.as_ref().and_then(|old| remap.get(old)) {
                    *r = Some(new.clone());
                }
            }
            z += 1.0;
            e.z = z;
            e.sanitize();
            self.elements.insert(&mut txn, e.id.as_str(), MapPrelim::from_iter(full_fields(&e)));
            ids.push(e.id);
        }
        drop(txn);
        self.touched();
        ids
    }

    fn edit(&mut self, ids: &[String], mut f: impl FnMut(&mut Element)) -> usize {
        let mut txn = self.txn();
        let mut n = 0;
        for id in ids {
            let Some(old) = read_one(&self.elements, &txn, id) else { continue };
            let mut new = old.clone();
            f(&mut new);
            new.id = old.id.clone();
            new.sanitize();
            if new != old {
                write_element(&self.elements, &mut txn, &old, &new);
                n += 1;
            }
        }
        drop(txn);
        if n > 0 {
            self.touched();
        }
        n
    }

    /// Replace all fields of an existing element (matched by `el.id`). One undo step.
    pub fn update(&mut self, el: &Element) -> Result<()> {
        if self.element(&el.id).is_none() {
            return Err(BoardError::NotFound(el.id.clone()));
        }
        self.checkpoint();
        let new = el.clone();
        self.edit(std::slice::from_ref(&el.id), |e| *e = new.clone());
        Ok(())
    }

    pub fn set_text(&mut self, id: &str, text: &str) {
        self.checkpoint();
        self.edit(&[id.to_string()], |e| e.text = text.to_string());
    }

    pub fn set_color(&mut self, ids: &[String], color: &str) {
        self.checkpoint();
        self.edit(ids, |e| e.color = color.to_string());
    }

    /// Move to an absolute position. Continuous: merges into the current undo step.
    pub fn move_to(&mut self, id: &str, x: f64, y: f64) {
        self.edit(&[id.to_string()], |e| {
            e.x = x;
            e.y = y;
        });
    }

    /// Move by a delta. Continuous: merges into the current undo step.
    pub fn move_by(&mut self, ids: &[String], dx: f64, dy: f64) {
        self.edit(ids, |e| {
            e.x += dx;
            e.y += dy;
        });
    }

    /// Set the bounding box; path points are scaled with it. Continuous like [`Board::move_to`].
    pub fn resize(&mut self, id: &str, x: f64, y: f64, w: f64, h: f64) {
        self.edit(&[id.to_string()], |e| {
            let (w, h) = (clamp_size(w), clamp_size(h));
            let sx = if e.w > 1e-9 { w / e.w } else { 1.0 };
            let sy = if e.h > 1e-9 { h / e.h } else { 1.0 };
            for (i, p) in e.points.iter_mut().enumerate() {
                *p *= if i % 2 == 0 { sx } else { sy };
            }
            e.x = x;
            e.y = y;
            e.w = if e.kind.is_path() && e.w <= 1e-9 { 0.0 } else { w };
            e.h = if e.kind.is_path() && e.h <= 1e-9 { 0.0 } else { h };
        });
    }

    /// Delete elements. Arrows bound to them keep their current position. One undo step.
    pub fn delete(&mut self, ids: &[String]) -> usize {
        self.checkpoint();
        self.remove(ids)
    }

    /// Like [`Board::delete`] but continuous: merges into the current undo step (eraser strokes).
    pub fn remove(&mut self, ids: &[String]) -> usize {
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut txn = self.txn();
        let all = read_all(&self.elements, &txn);
        let by_id: HashMap<&str, &Element> = all.iter().map(|e| (e.id.as_str(), e)).collect();
        for arrow in all.iter().filter(|e| e.kind == Kind::Arrow && !gone.contains(e.id.as_str())) {
            let bound_gone = |r: &Option<String>| r.as_deref().is_some_and(|r| gone.contains(r));
            if !bound_gone(&arrow.from) && !bound_gone(&arrow.to) {
                continue;
            }
            let (a, b) = arrow_endpoints(arrow, |id| by_id.get(id).copied());
            let mut new = arrow.clone();
            new.set_world_points(&[a, b]);
            if bound_gone(&arrow.from) {
                new.from = None;
            }
            if bound_gone(&arrow.to) {
                new.to = None;
            }
            new.sanitize();
            write_element(&self.elements, &mut txn, arrow, &new);
        }
        let mut n = 0;
        for id in ids {
            if self.elements.remove(&mut txn, id).is_some() {
                n += 1;
            }
        }
        drop(txn);
        self.touched();
        n
    }

    pub fn clear(&mut self) {
        let ids: Vec<String> = self.elements().into_iter().map(|e| e.id).collect();
        self.delete(&ids);
    }

    /// Put elements above everything else, keeping their relative order.
    pub fn bring_to_front(&mut self, ids: &[String]) {
        self.checkpoint();
        let all = self.elements();
        let top = all.last().map(|e| e.z).unwrap_or(0.0);
        let sel: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let order: Vec<String> = all.iter().filter(|e| sel.contains(e.id.as_str())).map(|e| e.id.clone()).collect();
        for (i, id) in order.iter().enumerate() {
            self.edit(std::slice::from_ref(id), |e| e.z = top + 1.0 + i as f64);
        }
    }

    /// Put elements below everything else, keeping their relative order.
    pub fn send_to_back(&mut self, ids: &[String]) {
        self.checkpoint();
        let all = self.elements();
        let bottom = all.first().map(|e| e.z).unwrap_or(0.0);
        let sel: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let order: Vec<String> = all.iter().filter(|e| sel.contains(e.id.as_str())).map(|e| e.id.clone()).collect();
        let n = order.len() as f64;
        for (i, id) in order.iter().enumerate() {
            self.edit(std::slice::from_ref(id), |e| e.z = bottom - n + i as f64);
        }
    }

    // ----- undo -------------------------------------------------------------------------

    /// Undo the last local change (changes from peers are never undone).
    pub fn undo(&mut self) -> bool {
        let r = self.undo.undo_blocking();
        self.touched();
        r
    }

    pub fn redo(&mut self) -> bool {
        let r = self.undo.redo_blocking();
        self.touched();
        r
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Forget the undo history (e.g. after loading or generating a board).
    pub fn clear_history(&mut self) {
        self.undo.clear_all();
    }

    /// Write raw values into an element's map, bypassing validation. Tests only.
    #[cfg(test)]
    pub(crate) fn raw_insert(&mut self, id: &str, fields: Vec<(&'static str, Any)>) {
        let mut txn = self.txn();
        self.elements.insert(&mut txn, id, MapPrelim::from_iter(fields.into_iter().map(|(k, v)| (k, In::Any(v)))));
        drop(txn);
        self.touched();
    }

    #[cfg(test)]
    pub(crate) fn raw_insert_any(&mut self, id: &str, value: Any) {
        let mut txn = self.txn();
        self.elements.insert(&mut txn, id, value);
        drop(txn);
        self.touched();
    }
}
