//! AI hooks: Markdown outline → mind map, board → text for an LLM. Pure, no I/O.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::model::{Board, Element, Kind, truncate_chars};

const MAX_INPUT: usize = 200_000;
const MAX_LINES: usize = 4_000;
const MAX_NODES: usize = 300;
const MAX_DEPTH: usize = 6;
const MAX_NODE_TEXT: usize = 200;
const MAX_SUMMARY: usize = 40_000;

const ROOT_W: f64 = 240.0;
const NODE_W: f64 = 200.0;
const COL_GAP: f64 = 56.0;
const ROW_GAP: f64 = 16.0;
/// Extra room between two main branches on the same side.
const BRANCH_GAP: f64 = 20.0;
/// View (world units at 100 %) the layout is chosen for: the board usually sits in a side panel.
const REF_VIEW: (f64, f64) = (900.0, 800.0);
const BRANCH_COLORS: [&str; 6] = ["orange", "blue", "green", "pink", "violet", "red"];

/// One node of a parsed outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    pub text: String,
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    /// Number of nodes in this subtree.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(OutlineNode::count).sum::<usize>()
    }

    /// Number of leaves in this subtree.
    pub fn leaves(&self) -> usize {
        if self.children.is_empty() { 1 } else { self.children.iter().map(OutlineNode::leaves).sum() }
    }
}

/// Strip inline Markdown (emphasis, code, links) and list/heading noise.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            '[' => {
                // [text](url) → text
                let mut label = String::new();
                let mut closed = false;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    label.push(d);
                }
                out.push_str(&label);
                if closed && chars.peek() == Some(&'(') {
                    for d in chars.by_ref() {
                        if d == ')' {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    let mut t = out.split_whitespace().collect::<Vec<_>>().join(" ");
    while t.ends_with(':') {
        t.pop();
    }
    truncate_chars(&mut t, MAX_NODE_TEXT);
    t.trim().to_string()
}

/// `- item`, `* item`, `+ item`, `1. item`, `1) item`; returns the item text.
fn list_item(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix(['-', '*', '+']) {
        return rest.strip_prefix([' ', '\t']).map(|r| r.trim_start_matches("[ ] ").trim_start_matches("[x] "));
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let rest = s.get(digits..)?;
    rest.strip_prefix(['.', ')']).and_then(|r| r.strip_prefix([' ', '\t']))
}

fn indent_width(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// Parse headings and (nested) list items into a tree with a single root.
/// Text without any structure becomes one child per line under „Übersicht“.
pub fn parse_outline(text: &str) -> OutlineNode {
    let mut text = text.to_string();
    if text.len() > MAX_INPUT {
        let mut cut = MAX_INPUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    // (depth, text) in document order.
    let mut flat: Vec<(usize, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    let mut heading_depth: Option<usize> = None;
    let mut indents: Vec<usize> = Vec::new();
    let mut in_code = false;
    for line in text.lines().take(MAX_LINES) {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || trimmed.is_empty() || flat.len() >= MAX_NODES {
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed.get(hashes..).is_some_and(|r| r.starts_with(' ')) {
            let t = clean(trimmed.get(hashes..).unwrap_or_default());
            if !t.is_empty() {
                let d = (hashes - 1).min(MAX_DEPTH);
                flat.push((d, t));
                heading_depth = Some(d);
                indents.clear();
            }
            continue;
        }
        if let Some(item) = list_item(trimmed) {
            let ind = indent_width(line);
            while indents.last().is_some_and(|&top| top > ind) {
                indents.pop();
            }
            match indents.last() {
                Some(&top) if top == ind => {}
                Some(&top) if top > ind => {
                    indents.pop();
                    indents.push(ind);
                }
                _ => indents.push(ind),
            }
            let base = heading_depth.map(|d| d + 1).unwrap_or(0);
            let t = clean(item);
            if !t.is_empty() {
                flat.push(((base + indents.len().saturating_sub(1)).min(MAX_DEPTH), t));
            }
            continue;
        }
        if plain.len() < MAX_NODES {
            let t = clean(trimmed);
            if !t.is_empty() && !t.starts_with('|') && !t.starts_with('>') {
                plain.push(t);
            }
        }
    }
    if flat.is_empty() {
        let mut it = plain.into_iter();
        let root_text = it.next().unwrap_or_else(|| "Übersicht".into());
        return OutlineNode { text: root_text, children: it.take(MAX_NODES).map(|t| OutlineNode { text: t, children: Vec::new() }).collect() };
    }
    // Normalise so the shallowest level is 0, then build the forest with a stack.
    let min = flat.iter().map(|(d, _)| *d).min().unwrap_or(0);
    let mut forest: Vec<OutlineNode> = Vec::new();
    let mut stack: Vec<(usize, OutlineNode)> = Vec::new();
    fn close(stack: &mut Vec<(usize, OutlineNode)>, forest: &mut Vec<OutlineNode>) {
        if let Some((_, node)) = stack.pop() {
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(node),
                None => forest.push(node),
            }
        }
    }
    for (d, t) in flat {
        let d = d - min;
        // A jump of several levels attaches to the nearest open ancestor.
        let d = match stack.last() {
            Some((top, _)) => d.min(top + 1),
            None => 0,
        };
        while stack.last().is_some_and(|(top, _)| *top >= d) {
            close(&mut stack, &mut forest);
        }
        stack.push((d, OutlineNode { text: t, children: Vec::new() }));
    }
    while !stack.is_empty() {
        close(&mut stack, &mut forest);
    }
    if forest.len() == 1
        && let Some(root) = forest.pop()
    {
        return root;
    }
    OutlineNode { text: "Übersicht".into(), children: forest }
}

/// Height of a note that fits `text` at width `w` (rough estimate: ~8.5 px per character).
pub(crate) fn note_height(text: &str, w: f64) -> f64 {
    let per_line = ((w - 24.0) / 8.5).max(1.0);
    let lines = (text.chars().count() as f64 / per_line).ceil().max(1.0);
    (lines * 21.0 + 34.0).clamp(64.0, 240.0)
}

/// How the main branches are arranged around the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapLayout {
    /// Root in the middle, branches split into a right and a left half.
    TwoSided,
    /// Root on the left, all branches stacked above and below it on the right (narrow panels).
    OneSided,
}

struct Layout<'a> {
    els: Vec<Element>,
    next: usize,
    prefix: &'a str,
}

impl Layout<'_> {
    fn id(&mut self) -> String {
        self.next += 1;
        format!("{}{}", self.prefix, self.next)
    }

    /// Vertical span a subtree needs.
    fn span(node: &OutlineNode) -> f64 {
        let own = note_height(&node.text, NODE_W) + ROW_GAP;
        let kids: f64 = node.children.iter().map(Self::span).sum();
        own.max(kids)
    }

    /// Span of a main branch, with the gap to its neighbour.
    fn branch_span(node: &OutlineNode) -> f64 {
        Self::span(node) + BRANCH_GAP
    }

    /// Place `node` in column `depth` on `side` (+1 right, -1 left) inside `[top, top + span)`.
    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, node: &OutlineNode, parent: &Element, depth: usize, side: f64, top: f64, color: &str, cx: f64) {
        let span = Self::span(node);
        let h = note_height(&node.text, NODE_W);
        let col = ROOT_W / 2.0 + COL_GAP + (depth as f64 - 1.0) * (NODE_W + COL_GAP);
        let x = if side > 0.0 { cx + col } else { cx - col - NODE_W };
        let y = top + span / 2.0 - h / 2.0;
        let mut el = Element::new(Kind::Note, x, y).with_size(NODE_W, h).with_text(node.text.clone()).with_color(color);
        el.id = self.id();
        let mut arrow = Element::connector(parent, &el).with_color("gray");
        arrow.id = self.id();
        self.els.push(el.clone());
        self.els.push(arrow);
        let kids_span: f64 = node.children.iter().map(Self::span).sum();
        let mut t = top + (span - kids_span) / 2.0;
        for c in &node.children {
            self.place(c, &el, depth + 1, side, t, color, cx);
            t += Self::span(c);
        }
    }
}

/// Split the branches into a right part (first `k`) and a left part with heights as equal as
/// possible, keeping reading order (clockwise: right top to bottom, then left).
fn balanced_split(spans: &[f64]) -> usize {
    let total: f64 = spans.iter().sum();
    let n = spans.len();
    let mut best = (f64::INFINITY, usize::MAX, n.div_ceil(2));
    let mut acc = 0.0;
    for k in 0..=n {
        if k > 0 {
            acc += spans.get(k - 1).copied().unwrap_or(0.0);
        }
        let worst = acc.max(total - acc);
        let count_diff = k.abs_diff(n - k);
        // Taller side first, then fewer extra nodes on one side; prefer the right side on ties.
        if worst < best.0 - 1e-6 || ((worst - best.0).abs() <= 1e-6 && count_diff < best.1) {
            best = (worst, count_diff, k);
        }
    }
    best.2
}

/// Mind-map elements for an outline, root centred on `(cx, cy)`.
fn mind_map_with(root: &OutlineNode, cx: f64, cy: f64, prefix: &str, layout: MapLayout) -> Vec<Element> {
    let rh = note_height(&root.text, ROOT_W).max(110.0);
    let mut root_el = Element::new(Kind::Note, cx - ROOT_W / 2.0, cy - rh / 2.0).with_size(ROOT_W, rh).with_text(root.text.clone()).with_color("yellow");
    let mut lay = Layout { els: Vec::new(), next: 0, prefix };
    root_el.id = lay.id();
    lay.els.push(root_el.clone());
    let spans: Vec<f64> = root.children.iter().map(Layout::branch_span).collect();
    let k = match layout {
        MapLayout::TwoSided => balanced_split(&spans),
        MapLayout::OneSided => root.children.len(),
    };
    let branches: Vec<(usize, &OutlineNode)> = root.children.iter().enumerate().collect();
    let (right, left) = branches.split_at(k.min(branches.len()));
    // Left side reads bottom to top (clockwise), so reverse it.
    let left: Vec<(usize, &OutlineNode)> = left.iter().rev().copied().collect();
    for (side, list) in [(1.0, right.to_vec()), (-1.0, left)] {
        let span: f64 = list.iter().map(|(_, c)| Layout::branch_span(c)).sum();
        let mut top = cy - span / 2.0 + BRANCH_GAP / 2.0;
        for (i, c) in list {
            let color = BRANCH_COLORS.get(i % BRANCH_COLORS.len()).copied().unwrap_or("orange");
            lay.place(c, &root_el, 1, side, top, color, cx);
            top += Layout::branch_span(c);
        }
    }
    lay.els
}

/// World bounds of the notes of a layout.
fn bounds(els: &[Element]) -> (f64, f64) {
    let notes = els.iter().filter(|e| e.kind == Kind::Note);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for e in notes {
        x0 = x0.min(e.x);
        y0 = y0.min(e.y);
        x1 = x1.max(e.x + e.w);
        y1 = y1.max(e.y + e.h);
    }
    if x0.is_finite() { (x1 - x0, y1 - y0) } else { (0.0, 0.0) }
}

/// Zoom at which a layout of this size fits the reference view (capped at 1).
fn fit_score((w, h): (f64, f64)) -> f64 {
    (REF_VIEW.0 / w.max(1.0)).min(REF_VIEW.1 / h.max(1.0)).min(1.0)
}

/// The layout that is most readable once zoomed to fit: two-sided unless one side stacks better.
pub(crate) fn choose_layout(root: &OutlineNode) -> MapLayout {
    let two = fit_score(bounds(&mind_map_with(root, 0.0, 0.0, "t", MapLayout::TwoSided)));
    let one = fit_score(bounds(&mind_map_with(root, 0.0, 0.0, "t", MapLayout::OneSided)));
    if one > two * 1.05 { MapLayout::OneSided } else { MapLayout::TwoSided }
}

fn mind_map(root: &OutlineNode, cx: f64, cy: f64, prefix: &str) -> Vec<Element> {
    mind_map_with(root, cx, cy, prefix, choose_layout(root))
}

impl Board {
    /// A new board with a mind map of an indented Markdown list or headings
    /// (root in the centre, branches to both sides, arrows between). For „Aus Chat als Whiteboard“.
    pub fn from_outline(text: &str) -> Board {
        let mut b = Board::new();
        let root = parse_outline(text);
        b.set_title(&root.text);
        b.add_many(mind_map(&root, 0.0, 0.0, "m"));
        b.clear_history();
        b
    }

    /// Add a mind map to this board, root centred on `(cx, cy)`; one undo step. Returns the ids.
    pub fn add_outline(&mut self, text: &str, cx: f64, cy: f64) -> Vec<String> {
        let root = parse_outline(text);
        let prefix = format!("m{:x}-", self.revision());
        self.add_many(mind_map(&root, cx, cy, &prefix))
    }

    /// A readable description of the board for an LLM: texts grouped by frame, then connections.
    pub fn summary_text(&self) -> String {
        let els = self.elements();
        let by_id: HashMap<&str, &Element> = els.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut out = String::new();
        let title = self.title();
        let _ = writeln!(out, "Whiteboard: {}", if title.trim().is_empty() { "ohne Titel" } else { title.trim() });
        let _ = writeln!(out, "{} Elemente", els.len());

        let frames: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Frame).collect();
        let frame_of = |e: &Element| -> Option<usize> {
            let (cx, cy) = e.center();
            frames
                .iter()
                .enumerate()
                .filter(|(_, f)| f.id != e.id && cx >= f.x && cx <= f.x + f.w && cy >= f.y && cy <= f.y + f.h)
                .min_by(|a, b| (a.1.w * a.1.h).total_cmp(&(b.1.w * b.1.h)))
                .map(|(i, _)| i)
        };
        let mut groups: Vec<Vec<&Element>> = vec![Vec::new(); frames.len() + 1];
        let mut drawings: HashMap<&'static str, usize> = HashMap::new();
        for e in &els {
            if e.kind == Kind::Frame || e.kind == Kind::Arrow {
                continue;
            }
            if e.text.trim().is_empty() {
                *drawings.entry(e.kind.label()).or_default() += 1;
                continue;
            }
            let g = frame_of(e).unwrap_or(frames.len());
            if let Some(list) = groups.get_mut(g) {
                list.push(e);
            }
        }
        let one_line = |s: &str| {
            let mut t = s.split_whitespace().collect::<Vec<_>>().join(" ");
            truncate_chars(&mut t, 500);
            t
        };
        for (i, list) in groups.iter_mut().enumerate() {
            // Reading order: rows of ~120 units, left to right.
            list.sort_by(|a, b| (a.y / 120.0).floor().total_cmp(&(b.y / 120.0).floor()).then(a.x.total_cmp(&b.x)));
            let heading = match frames.get(i) {
                Some(f) if !f.text.trim().is_empty() => format!("Rahmen „{}“", one_line(&f.text)),
                Some(_) => format!("Rahmen {}", i + 1),
                None if frames.is_empty() => "Inhalt".to_string(),
                None => "Außerhalb von Rahmen".to_string(),
            };
            if list.is_empty() && frames.get(i).is_none() {
                continue;
            }
            let _ = writeln!(out, "\n## {heading}");
            if list.is_empty() {
                let _ = writeln!(out, "(leer)");
            }
            for e in list.iter() {
                let _ = writeln!(out, "- {}: {}", e.kind.label(), one_line(&e.text));
                if out.len() > MAX_SUMMARY {
                    break;
                }
            }
        }
        let label = |id: Option<&str>| id.and_then(|i| by_id.get(i)).map(|e| one_line(&e.text)).filter(|t| !t.is_empty());
        let links: Vec<String> = els
            .iter()
            .filter(|e| e.kind == Kind::Arrow)
            .filter_map(|e| {
                let (a, b) = (label(e.from.as_deref()), label(e.to.as_deref()));
                match (a, b) {
                    (Some(a), Some(b)) => Some(format!("- {a} → {b}")),
                    (Some(a), None) => Some(format!("- {a} → …")),
                    (None, Some(b)) => Some(format!("- … → {b}")),
                    (None, None) => None,
                }
            })
            .collect();
        if !links.is_empty() {
            let _ = writeln!(out, "\n## Verbindungen");
            for l in links {
                if out.len() > MAX_SUMMARY {
                    break;
                }
                let _ = writeln!(out, "{l}");
            }
        }
        if !drawings.is_empty() {
            let mut d: Vec<String> = drawings.into_iter().map(|(k, n)| format!("{n}× {k}")).collect();
            d.sort();
            let _ = writeln!(out, "\nOhne Text: {}", d.join(", "));
        }
        if out.len() > MAX_SUMMARY {
            let mut cut = MAX_SUMMARY;
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push_str("\n…");
        }
        out
    }
}
