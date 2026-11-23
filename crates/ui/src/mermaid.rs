//! Mermaid diagrams from ```` ```mermaid ```` blocks, drawn natively (no browser, no JavaScript):
//! flowcharts (`graph`/`flowchart` with directions TD, TB, BT, LR, RL, the common node shapes,
//! edges with labels, chains and `&`) and mind maps (`mindmap`, a tree by indentation). Other
//! diagram types are named and shown as source.
//!
//! Everything is hostile input: the source, the node and edge counts, label lengths and the tree
//! depth are capped; a statement that does not parse is skipped, never fatal.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Galley, Pos2, Rect, Shape as EShape, Stroke, Ui, Vec2, pos2, vec2};

/// The fence language of Mermaid blocks.
pub const MERMAID_LANGUAGE: &str = "mermaid";

const MAX_SOURCE: usize = 20_000;
const MAX_NODES: usize = 150;
const MAX_EDGES: usize = 300;
const MAX_LABEL: usize = 200;
const MAX_DEPTH: usize = 12;

/// A node shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeShape {
    Rect,
    Round,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    Asymmetric,
    Rhombus,
    Hexagon,
    Parallelogram,
    Trapezoid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: NodeShape,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub arrow: bool,
    pub dotted: bool,
    pub thick: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    Right,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flowchart {
    pub dir: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Something was left out (caps, unknown statements).
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MindNode {
    pub label: String,
    pub shape: NodeShape,
    pub children: Vec<MindNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Diagram {
    Flowchart(Flowchart),
    Mindmap(MindNode),
    /// A diagram type this module does not draw (its keyword).
    Unsupported(String),
}

/// Read a Mermaid source. `Err` is a German message for the person.
pub fn parse(src: &str) -> Result<Diagram, String> {
    if src.len() > MAX_SOURCE {
        return Err(format!("Das Diagramm ist zu groß (über {} Zeichen).", MAX_SOURCE));
    }
    let mut lines = src.lines().map(strip_comment).filter(|l| !l.trim().is_empty());
    // Front matter and init directives (`---` … `---`, `%%{init: …}%%`) are skipped by
    // `strip_comment` and here.
    let mut header = lines.next().unwrap_or("").trim().to_string();
    if header == "---" {
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
        }
        header = lines.next().unwrap_or("").trim().to_string();
    }
    let keyword = header.split(|c: char| c.is_whitespace() || c == ';').next().unwrap_or("").to_string();
    match keyword.as_str() {
        "graph" | "flowchart" => {
            let rest_of_header = header.get(keyword.len()..).unwrap_or("").trim();
            let (dir_word, after) = rest_of_header.split_once(|c: char| c.is_whitespace() || c == ';').unwrap_or((rest_of_header, ""));
            let dir = match dir_word.trim() {
                "LR" => Direction::Right,
                "RL" => Direction::Left,
                "BT" => Direction::Up,
                _ => Direction::Down,
            };
            let body: Vec<&str> = std::iter::once(after).chain(lines).collect();
            let f = parse_flowchart(dir, &body);
            if f.nodes.is_empty() {
                return Err("Das Flussdiagramm enthält keine Knoten.".into());
            }
            Ok(Diagram::Flowchart(f))
        }
        "mindmap" => {
            let body: Vec<&str> = lines.collect();
            parse_mindmap(&body).map(Diagram::Mindmap).ok_or_else(|| "Die Mindmap enthält keine Einträge.".to_string())
        }
        "" => Err("Das Diagramm ist leer.".into()),
        k => Ok(Diagram::Unsupported(k.chars().take(40).collect())),
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find("%%") {
        Some(i) => line.get(..i).unwrap_or(""),
        None => line,
    }
}

// ---- flowchart parsing -------------------------------------------------------------------

struct FlowBuilder {
    nodes: Vec<Node>,
    index: HashMap<String, usize>,
    edges: Vec<Edge>,
    truncated: bool,
}

impl FlowBuilder {
    /// The node with this id, created or updated (a later `A[Text]` sets the label).
    fn node(&mut self, id: &str, label: Option<(String, NodeShape)>) -> Option<usize> {
        if let Some(&i) = self.index.get(id) {
            if let (Some((l, s)), Some(n)) = (label, self.nodes.get_mut(i)) {
                n.label = l;
                n.shape = s;
            }
            return Some(i);
        }
        if self.nodes.len() >= MAX_NODES {
            self.truncated = true;
            return None;
        }
        let (label, shape) = label.unwrap_or_else(|| (id.to_string(), NodeShape::Rect));
        self.nodes.push(Node { id: id.to_string(), label, shape });
        self.index.insert(id.to_string(), self.nodes.len() - 1);
        Some(self.nodes.len() - 1)
    }
}

fn parse_flowchart(dir: Direction, lines: &[&str]) -> Flowchart {
    let mut b = FlowBuilder { nodes: Vec::new(), index: HashMap::new(), edges: Vec::new(), truncated: false };
    for line in lines {
        for stmt in split_statements(line) {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            let first = stmt.split_whitespace().next().unwrap_or("");
            if matches!(first, "classDef" | "class" | "style" | "linkStyle" | "click" | "direction" | "accTitle" | "accDescr" | "end")
                || first.starts_with("accTitle:")
                || first.starts_with("accDescr")
            {
                continue;
            }
            if first == "subgraph" {
                continue;
            }
            if !parse_statement(stmt, &mut b) {
                b.truncated = true;
            }
        }
    }
    Flowchart { dir, nodes: b.nodes, edges: b.edges, truncated: b.truncated }
}

/// Split at `;` outside quotes and brackets.
fn split_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(line.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(line.get(start..).unwrap_or(""));
    out
}

/// One statement: node groups joined by edges. `false` when it does not parse.
fn parse_statement(stmt: &str, b: &mut FlowBuilder) -> bool {
    let mut rest = stmt;
    let Some((mut prev, r)) = parse_group(rest, b) else { return false };
    rest = r;
    loop {
        let t = rest.trim_start();
        if t.is_empty() {
            return true;
        }
        let Some((edge, r)) = parse_edge(t) else { return false };
        let Some((next, r)) = parse_group(r, b) else { return false };
        for &f in &prev {
            for &n in &next {
                if b.edges.len() >= MAX_EDGES {
                    b.truncated = true;
                    break;
                }
                b.edges.push(Edge { from: f, to: n, ..edge.clone() });
            }
        }
        prev = next;
        rest = r;
    }
}

/// `A`, `A[Text]`, `A & B`.
fn parse_group<'a>(s: &'a str, b: &mut FlowBuilder) -> Option<(Vec<usize>, &'a str)> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        let (id, label, r) = parse_node(rest)?;
        if let Some(i) = b.node(&id, label) {
            out.push(i);
        }
        rest = r;
        let t = rest.trim_start();
        if let Some(after) = t.strip_prefix('&') {
            rest = after;
        } else {
            return Some((out, rest));
        }
    }
}

const SHAPES: [(&str, &str, NodeShape); 14] = [
    ("(((", ")))", NodeShape::Circle),
    ("((", "))", NodeShape::Circle),
    ("([", "])", NodeShape::Stadium),
    ("[[", "]]", NodeShape::Subroutine),
    ("[(", ")]", NodeShape::Cylinder),
    ("[/", "/]", NodeShape::Parallelogram),
    ("[\\", "\\]", NodeShape::Parallelogram),
    ("[/", "\\]", NodeShape::Trapezoid),
    ("[\\", "/]", NodeShape::Trapezoid),
    ("{{", "}}", NodeShape::Hexagon),
    ("(", ")", NodeShape::Round),
    ("[", "]", NodeShape::Rect),
    ("{", "}", NodeShape::Rhombus),
    (">", "]", NodeShape::Asymmetric),
];

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A node: its id, its label and shape when given, and the rest of the statement.
type ParsedNode<'a> = (String, Option<(String, NodeShape)>, &'a str);

fn parse_node(s: &str) -> Option<ParsedNode<'_>> {
    let s = s.trim_start();
    let end = s.char_indices().find(|(_, c)| !is_id_char(*c)).map(|(i, _)| i).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let id: String = s.get(..end)?.chars().take(60).collect();
    let mut rest = s.get(end..)?;
    let mut label = None;
    for (open, close, shape) in SHAPES {
        let Some(after) = rest.strip_prefix(open) else { continue };
        // A quoted label may hold the closing characters.
        let (text, after_close) = if let Some(q) = after.strip_prefix('"') {
            let Some(qe) = q.find('"') else { continue };
            let Some(tail) = q.get(qe + 1..).and_then(|t| t.strip_prefix(close)) else { continue };
            (q.get(..qe).unwrap_or(""), tail)
        } else {
            let Some(ce) = after.find(close) else { continue };
            (after.get(..ce).unwrap_or(""), after.get(ce + close.len()..).unwrap_or(""))
        };
        label = Some((clean_label(text), shape));
        rest = after_close;
        break;
    }
    // `:::className` after a node.
    if let Some(r) = rest.strip_prefix(":::") {
        let e = r.char_indices().find(|(_, c)| !(is_id_char(*c) || *c == '-')).map(|(i, _)| i).unwrap_or(r.len());
        rest = r.get(e..).unwrap_or("");
    }
    Some((id, label, rest))
}

/// Label text: `<br>` becomes a line break, quotes, Markdown stars and entities go.
fn clean_label(s: &str) -> String {
    let mut t = s.trim().trim_matches('"').trim_matches('`').to_string();
    for br in ["<br>", "<br/>", "<br />", "\\n"] {
        t = t.replace(br, "\n");
    }
    t = t.replace("**", "").replace("#quot;", "\"").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
    t.chars().take(MAX_LABEL).collect()
}

/// An edge operator at the start of `s` (after white space), with its label.
fn parse_edge(s: &str) -> Option<(Edge, &str)> {
    let mut rest = s.strip_prefix('<').unwrap_or(s);
    let line_len = rest.chars().take_while(|c| matches!(c, '-' | '=' | '.')).count();
    if line_len < 2 {
        return None;
    }
    let line = rest.get(..line_len)?;
    let mut dotted = line.contains('.');
    let mut thick = line.contains('=');
    rest = rest.get(line_len..)?;
    let mut label = String::new();
    let mut arrow;
    // `-- Text -->`, `== Text ==>`, `-. Text .->`
    if rest.starts_with(char::is_whitespace) && matches!(line, "--" | "==" | "-.") {
        let closers: &[&str] = match line {
            "--" => &["-->", "---", "--o", "--x"],
            "==" => &["==>", "===", "==o", "==x"],
            _ => &[".->", ".-"],
        };
        let found = closers.iter().filter_map(|c| rest.find(c).map(|i| (i, *c))).min_by_key(|(i, _)| *i);
        let (i, c) = found?;
        label = clean_label(rest.get(..i)?);
        rest = rest.get(i..)?;
        let n = rest.chars().take_while(|c| matches!(c, '-' | '=' | '.')).count();
        dotted |= c.starts_with('.');
        thick |= c.starts_with('=');
        rest = rest.get(n..)?;
        arrow = c.ends_with('>');
    } else {
        arrow = false;
    }
    if let Some(r) = rest.strip_prefix('>') {
        arrow = true;
        rest = r;
    } else if let Some(r) = rest.strip_prefix(['o', 'x'])
        && !r.starts_with(is_id_char)
    {
        arrow = true;
        rest = r;
    }
    let t = rest.trim_start();
    if let Some(r) = t.strip_prefix('|') {
        let e = r.find('|')?;
        label = clean_label(r.get(..e)?);
        rest = r.get(e + 1..)?;
    }
    Some((Edge { from: 0, to: 0, label, arrow, dotted, thick }, rest))
}

// ---- mind map parsing --------------------------------------------------------------------

fn parse_mindmap(lines: &[&str]) -> Option<MindNode> {
    // (indent, node path) — build the tree with a stack of open nodes.
    let mut items: Vec<(usize, String, NodeShape)> = Vec::new();
    for l in lines.iter().take(MAX_NODES * 2) {
        let t = l.trim();
        if t.starts_with("::icon(") || t.starts_with(":::") {
            continue;
        }
        let indent = l.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
        let (label, shape) = mind_label(t);
        if label.is_empty() {
            continue;
        }
        items.push((indent, label, shape));
        if items.len() >= MAX_NODES {
            break;
        }
    }
    let mut iter = items.into_iter();
    let (root_indent, label, shape) = iter.next()?;
    let mut root = MindNode { label, shape, children: Vec::new() };
    // Stack of indents from the root down to the last placed node.
    let mut stack: Vec<usize> = vec![root_indent];
    for (indent, label, shape) in iter {
        while stack.len() > 1 && stack.last().is_some_and(|&i| indent <= i) {
            stack.pop();
        }
        if stack.len() > MAX_DEPTH {
            continue;
        }
        insert(&mut root, stack.len() - 1, MindNode { label, shape, children: Vec::new() });
        stack.push(indent);
    }
    Some(root)
}

/// Add `child` below the last node `depth` levels down.
fn insert(node: &mut MindNode, depth: usize, child: MindNode) {
    match node.children.last_mut() {
        Some(last) if depth > 0 => insert(last, depth - 1, child),
        _ => node.children.push(child),
    }
}

fn mind_label(t: &str) -> (String, NodeShape) {
    if let Some((_, Some((label, shape)), rest)) = parse_node(t)
        && rest.trim().is_empty()
    {
        return (label, shape);
    }
    // A bare shape without id: `((Text))`, `[Text]`.
    for (open, close, shape) in SHAPES {
        if let Some(inner) = t.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            return (clean_label(inner), shape);
        }
    }
    (clean_label(t), NodeShape::Round)
}

/// A mind map as an indented outline (`- Text`), the form the whiteboard reads.
pub fn mindmap_outline(root: &MindNode) -> String {
    fn walk(n: &MindNode, depth: usize, out: &mut String) {
        out.push_str(&"  ".repeat(depth));
        out.push_str("- ");
        out.push_str(&n.label.replace('\n', " "));
        out.push('\n');
        for c in &n.children {
            walk(c, depth + 1, out);
        }
    }
    let mut out = String::new();
    walk(root, 0, &mut out);
    out
}

// ---- drawing -----------------------------------------------------------------------------

/// Colours for a diagram.
#[derive(Clone, Copy)]
pub struct Palette {
    pub text: Color32,
    pub muted: Color32,
    pub fill: Color32,
    pub stroke: Color32,
    pub accent: Color32,
    pub bg: Color32,
}

const BRANCHES: [Color32; 6] = [
    Color32::from_rgb(0x25, 0x63, 0xeb),
    Color32::from_rgb(0x16, 0xa3, 0x4a),
    Color32::from_rgb(0xd9, 0x77, 0x06),
    Color32::from_rgb(0xdb, 0x27, 0x77),
    Color32::from_rgb(0x7c, 0x3a, 0xed),
    Color32::from_rgb(0x08, 0x91, 0xb2),
];

struct Placed {
    rect: Rect,
    shape: NodeShape,
    galley: Arc<Galley>,
    fill: Color32,
    stroke: Color32,
}

fn label_galley(ui: &Ui, text: &str, size: f32, color: Color32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::proportional(size), color, 180.0 * size / 14.0);
    job.halign = egui::Align::Center;
    ui.painter().layout_job(job)
}

/// The outer size of a node around its text.
fn node_size(shape: NodeShape, text: Vec2, size: f32) -> Vec2 {
    let pad = vec2(size * 0.9, size * 0.55);
    let base = text + pad * 2.0;
    match shape {
        NodeShape::Circle => {
            let d = (text.x.max(text.y) + size * 1.4).max(size * 3.0);
            vec2(d, d)
        }
        NodeShape::Rhombus => vec2(text.x + text.y * 2.0 + size * 2.0, text.y * 2.0 + size * 1.6),
        NodeShape::Hexagon | NodeShape::Parallelogram | NodeShape::Trapezoid | NodeShape::Asymmetric => vec2(base.x + size * 1.2, base.y),
        NodeShape::Cylinder => vec2(base.x, base.y + size * 0.6),
        _ => base,
    }
}

fn draw_node(painter: &egui::Painter, p: &Placed, offset: Vec2, scale: f32) {
    let r = p.rect.translate(offset);
    let stroke = Stroke::new(1.2 * scale.max(0.6), p.stroke);
    let (l, rt, t, b) = (r.left(), r.right(), r.top(), r.bottom());
    let c = r.center();
    let slant = (r.height() * 0.35).min(r.width() * 0.2);
    let poly = |pts: Vec<Pos2>| EShape::convex_polygon(pts, p.fill, stroke);
    match p.shape {
        NodeShape::Rect => {
            painter.rect(r, CornerRadius::same(2), p.fill, stroke, egui::StrokeKind::Inside);
        }
        NodeShape::Round => {
            painter.rect(r, CornerRadius::same((r.height() * 0.25).min(12.0) as u8), p.fill, stroke, egui::StrokeKind::Inside);
        }
        NodeShape::Stadium => {
            painter.rect(r, CornerRadius::same((r.height() * 0.5).min(120.0) as u8), p.fill, stroke, egui::StrokeKind::Inside);
        }
        NodeShape::Subroutine => {
            painter.rect(r, CornerRadius::ZERO, p.fill, stroke, egui::StrokeKind::Inside);
            let inset = (r.width() * 0.06).clamp(4.0, 10.0);
            painter.line_segment([pos2(l + inset, t), pos2(l + inset, b)], stroke);
            painter.line_segment([pos2(rt - inset, t), pos2(rt - inset, b)], stroke);
        }
        NodeShape::Cylinder => {
            let ry = (r.height() * 0.12).max(3.0);
            painter.rect(Rect::from_min_max(pos2(l, t + ry), pos2(rt, b - ry)), CornerRadius::ZERO, p.fill, Stroke::NONE, egui::StrokeKind::Inside);
            painter.add(ellipse(pos2(c.x, b - ry), r.width() / 2.0, ry, p.fill, stroke));
            painter.line_segment([pos2(l, t + ry), pos2(l, b - ry)], stroke);
            painter.line_segment([pos2(rt, t + ry), pos2(rt, b - ry)], stroke);
            painter.add(ellipse(pos2(c.x, t + ry), r.width() / 2.0, ry, p.fill, stroke));
        }
        NodeShape::Circle => {
            painter.circle(c, r.width().min(r.height()) / 2.0, p.fill, stroke);
        }
        NodeShape::Rhombus => {
            painter.add(poly(vec![pos2(c.x, t), pos2(rt, c.y), pos2(c.x, b), pos2(l, c.y)]));
        }
        NodeShape::Hexagon => {
            painter.add(poly(vec![pos2(l + slant, t), pos2(rt - slant, t), pos2(rt, c.y), pos2(rt - slant, b), pos2(l + slant, b), pos2(l, c.y)]));
        }
        NodeShape::Parallelogram => {
            painter.add(poly(vec![pos2(l + slant, t), pos2(rt, t), pos2(rt - slant, b), pos2(l, b)]));
        }
        NodeShape::Trapezoid => {
            painter.add(poly(vec![pos2(l + slant, t), pos2(rt - slant, t), pos2(rt, b), pos2(l, b)]));
        }
        NodeShape::Asymmetric => {
            painter.rect(Rect::from_min_max(pos2(l + slant, t), pos2(rt, b)), CornerRadius::ZERO, p.fill, Stroke::NONE, egui::StrokeKind::Inside);
            painter.add(EShape::convex_polygon(
                vec![pos2(l, t), pos2(l + slant, t), pos2(l + slant, b), pos2(l, b), pos2(l + slant, c.y)],
                Color32::TRANSPARENT,
                Stroke::NONE,
            ));
            painter.add(EShape::closed_line(vec![pos2(l, t), pos2(rt, t), pos2(rt, b), pos2(l, b), pos2(l + slant, c.y)], stroke));
        }
    }
    let g = &p.galley;
    painter.galley(c - g.rect.center().to_vec2(), g.clone(), p.stroke);
}

fn ellipse(center: Pos2, rx: f32, ry: f32, fill: Color32, stroke: Stroke) -> EShape {
    let pts: Vec<Pos2> = (0..32)
        .map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            center + vec2(rx * a.cos(), ry * a.sin())
        })
        .collect();
    EShape::convex_polygon(pts, fill, stroke)
}

/// Where the line from the node centre towards `towards` leaves the node.
fn boundary(p: &Placed, towards: Pos2) -> Pos2 {
    let c = p.rect.center();
    let d = towards - c;
    if d.length() < 0.01 {
        return c;
    }
    let hw = p.rect.width() / 2.0;
    let hh = p.rect.height() / 2.0;
    let t = match p.shape {
        NodeShape::Circle => hw.min(hh) / d.length(),
        NodeShape::Rhombus => 1.0 / (d.x.abs() / hw.max(0.01) + d.y.abs() / hh.max(0.01)),
        _ => (hw / d.x.abs().max(0.001)).min(hh / d.y.abs().max(0.001)),
    };
    c + d * t.min(1.0)
}

fn draw_edge(painter: &egui::Painter, points: &[Pos2], e: &Edge, color: Color32, scale: f32) {
    let (Some(&b), Some(&before)) = (points.last(), points.len().checked_sub(2).and_then(|i| points.get(i))) else { return };
    let w = if e.thick { 2.6 } else { 1.3 } * scale.max(0.6);
    let stroke = Stroke::new(w, color);
    let dir = (b - before).normalized();
    let head = 8.0 * scale.max(0.6);
    let mut line = points.to_vec();
    if e.arrow
        && let Some(last) = line.last_mut()
    {
        *last = b - dir * head * 0.8;
    }
    if e.dotted {
        painter.extend(EShape::dashed_line(&line, stroke, 4.0 * scale.max(0.6), 3.0 * scale.max(0.6)));
    } else {
        painter.add(EShape::line(line, stroke));
    }
    if e.arrow && dir.length() > 0.5 {
        let n = vec2(-dir.y, dir.x);
        painter.add(EShape::convex_polygon(vec![b, b - dir * head + n * head * 0.45, b - dir * head - n * head * 0.45], color, Stroke::NONE));
    }
}

/// A cubic Bézier as a polyline.
fn bezier(p: [Pos2; 4]) -> Vec<Pos2> {
    (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let u = 1.0 - t;
            let v = p[0].to_vec2() * (u * u * u) + p[1].to_vec2() * (3.0 * u * u * t) + p[2].to_vec2() * (3.0 * u * t * t) + p[3].to_vec2() * (t * t * t);
            v.to_pos2()
        })
        .collect()
}

/// Layered layout of a flowchart: rank by longest path (cycles broken), order inside a rank by
/// the parents' positions, then rows (TD/BT) or columns (LR/RL).
fn flow_ranks(f: &Flowchart) -> Vec<usize> {
    let n = f.nodes.len();
    // Break cycles: an edge is "back" when it closes a loop in a DFS from the nodes in order.
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, e) in f.edges.iter().enumerate() {
        if let Some(v) = out_edges.get_mut(e.from) {
            v.push(k);
        }
    }
    let mut state = vec![0u8; n]; // 0 new, 1 on stack, 2 done
    let mut back = vec![false; f.edges.len()];
    for s in 0..n {
        if state.get(s) != Some(&0) {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        if let Some(x) = state.get_mut(s) {
            *x = 1;
        }
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            let next = out_edges.get(v).and_then(|es| es.get(*i)).copied();
            *i += 1;
            match next {
                Some(k) => {
                    let to = f.edges.get(k).map(|e| e.to).unwrap_or(v);
                    match state.get(to) {
                        Some(0) => {
                            if let Some(x) = state.get_mut(to) {
                                *x = 1;
                            }
                            stack.push((to, 0));
                        }
                        Some(1) => {
                            if let Some(b) = back.get_mut(k) {
                                *b = true;
                            }
                        }
                        _ => {}
                    }
                }
                None => {
                    if let Some(x) = state.get_mut(v) {
                        *x = 2;
                    }
                    stack.pop();
                }
            }
        }
    }
    // Longest path over the forward edges (Kahn order).
    let mut indeg = vec![0usize; n];
    for (k, e) in f.edges.iter().enumerate() {
        if !back.get(k).copied().unwrap_or(true)
            && e.from != e.to
            && let Some(d) = indeg.get_mut(e.to)
        {
            *d += 1;
        }
    }
    let mut rank = vec![0usize; n];
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&v| indeg.get(v) == Some(&0)).collect();
    while let Some(v) = queue.pop_front() {
        let rv = rank.get(v).copied().unwrap_or(0);
        for &k in out_edges.get(v).map(|v| v.as_slice()).unwrap_or(&[]) {
            if back.get(k).copied().unwrap_or(true) {
                continue;
            }
            let Some(e) = f.edges.get(k) else { continue };
            if e.from == e.to {
                continue;
            }
            if let Some(r) = rank.get_mut(e.to) {
                *r = (*r).max(rv + 1);
            }
            if let Some(d) = indeg.get_mut(e.to) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    queue.push_back(e.to);
                }
            }
        }
    }
    rank
}

struct FlowLayout {
    placed: Vec<Placed>,
    ranks: Vec<usize>,
    size: Vec2,
    /// Room for edges that run back against the direction (drawn as arcs beside the layers).
    bulge: f32,
}

fn layout_flow(ui: &Ui, f: &Flowchart, pal: &Palette, font: f32) -> FlowLayout {
    let ranks = flow_ranks(f);
    let max_rank = ranks.iter().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (v, &r) in ranks.iter().enumerate() {
        if let Some(l) = layers.get_mut(r) {
            l.push(v);
        }
    }
    // Order by the mean position of the neighbours in the layer before (a few sweeps).
    let mut pos: Vec<f32> = vec![0.0; f.nodes.len()];
    for l in &layers {
        for (i, &v) in l.iter().enumerate() {
            if let Some(p) = pos.get_mut(v) {
                *p = i as f32;
            }
        }
    }
    for sweep in 0..4 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down { (1..layers.len()).collect() } else { (0..layers.len().saturating_sub(1)).rev().collect() };
        for li in order {
            let Some(layer) = layers.get(li) else { continue };
            let mut keyed: Vec<(f32, usize)> = layer
                .iter()
                .map(|&v| {
                    let neigh: Vec<f32> = f
                        .edges
                        .iter()
                        .filter_map(|e| {
                            let other = if e.to == v {
                                e.from
                            } else if e.from == v {
                                e.to
                            } else {
                                return None;
                            };
                            let want = if down { li.checked_sub(1) } else { Some(li + 1) };
                            (ranks.get(other).copied() == want).then(|| pos.get(other).copied().unwrap_or(0.0))
                        })
                        .collect();
                    let k = if neigh.is_empty() { pos.get(v).copied().unwrap_or(0.0) } else { neigh.iter().sum::<f32>() / neigh.len() as f32 };
                    (k, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            let new: Vec<usize> = keyed.into_iter().map(|(_, v)| v).collect();
            for (i, &v) in new.iter().enumerate() {
                if let Some(p) = pos.get_mut(v) {
                    *p = i as f32;
                }
            }
            if let Some(l) = layers.get_mut(li) {
                *l = new;
            }
        }
    }
    let galleys: Vec<Arc<Galley>> = f.nodes.iter().map(|n| label_galley(ui, &n.label, font, pal.text)).collect();
    let sizes: Vec<Vec2> = f.nodes.iter().zip(&galleys).map(|(n, g)| node_size(n.shape, g.size(), font)).collect();
    let horizontal = matches!(f.dir, Direction::Right | Direction::Left);
    let gap_in = font * 1.6; // between nodes of one layer
    let gap_across = font * if horizontal { 4.0 } else { 3.2 }; // between layers
    let mut placed_rects: Vec<Rect> = vec![Rect::NOTHING; f.nodes.len()];
    // Layer extents: thickness across, length along.
    let thick: Vec<f32> =
        layers.iter().map(|l| l.iter().map(|&v| sizes.get(v).map(|s| if horizontal { s.x } else { s.y }).unwrap_or(0.0)).fold(0.0, f32::max)).collect();
    let length: Vec<f32> = layers
        .iter()
        .map(|l| l.iter().map(|&v| sizes.get(v).map(|s| if horizontal { s.y } else { s.x }).unwrap_or(0.0) + gap_in).sum::<f32>() - gap_in)
        .collect();
    let longest = length.iter().copied().fold(0.0, f32::max);
    let mut across = 0.0;
    for (li, l) in layers.iter().enumerate() {
        let th = thick.get(li).copied().unwrap_or(0.0);
        let mut along = (longest - length.get(li).copied().unwrap_or(0.0)) / 2.0;
        for &v in l {
            let s = sizes.get(v).copied().unwrap_or(Vec2::ZERO);
            let (sa, sc) = if horizontal { (s.y, s.x) } else { (s.x, s.y) };
            let c_across = across + th / 2.0;
            let min = if horizontal { pos2(c_across - sc / 2.0, along) } else { pos2(along, c_across - sc / 2.0) };
            if let Some(r) = placed_rects.get_mut(v) {
                *r = Rect::from_min_size(min, s);
            }
            along += sa + gap_in;
        }
        across += th + gap_across;
    }
    let total_across = (across - gap_across).max(0.0);
    // Flip for BT and RL.
    for r in &mut placed_rects {
        match f.dir {
            Direction::Up => *r = r.translate(vec2(0.0, total_across - r.top() - r.bottom())),
            Direction::Left => *r = r.translate(vec2(total_across - r.left() - r.right(), 0.0)),
            _ => {}
        }
    }
    let back = f.edges.iter().any(|e| e.from != e.to && ranks.get(e.to) <= ranks.get(e.from));
    let bulge = if back { font * 2.6 } else { 0.0 };
    let size = if horizontal { vec2(total_across, longest + bulge) } else { vec2(longest + bulge, total_across) };
    let placed = f
        .nodes
        .iter()
        .zip(galleys)
        .zip(placed_rects)
        .map(|((n, g), rect)| Placed { rect, shape: n.shape, galley: g, fill: pal.fill, stroke: pal.stroke })
        .collect();
    FlowLayout { placed, ranks, size, bulge }
}

/// Draw a flowchart, scaled down to fit the width.
pub fn show_flowchart(ui: &mut Ui, f: &Flowchart, pal: &Palette) {
    let base = 13.0;
    let avail = ui.available_width().max(100.0);
    let mut lay = layout_flow(ui, f, pal, base);
    let mut scale = 1.0;
    if lay.size.x > avail {
        scale = (avail / lay.size.x).max(0.45);
        lay = layout_flow(ui, f, pal, base * scale);
    }
    let pad = 6.0;
    let want = lay.size + Vec2::splat(pad * 2.0);
    egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).auto_shrink([false, true]).show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(want.x.max(ui.available_width()), want.y), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let offset = rect.min.to_vec2() + vec2(((rect.width() - want.x) / 2.0).max(0.0) + pad, pad);
        let mut labels = Vec::new();
        for e in &f.edges {
            let (Some(a), Some(b)) = (lay.placed.get(e.from), lay.placed.get(e.to)) else { continue };
            if e.from == e.to {
                // A loop: a small arc over the node.
                let r = a.rect.translate(offset);
                let top = r.right_top() + vec2(-8.0, 0.0);
                painter.add(EShape::line(vec![top, top + vec2(10.0, -10.0), top + vec2(14.0, 6.0), r.right_center()], Stroke::new(1.2, pal.muted)));
                continue;
            }
            let backwards = lay.ranks.get(e.to) <= lay.ranks.get(e.from);
            let points = if backwards {
                // Against the direction: an arc beside the layers (below for LR/RL, right for TD/BT).
                let (ra, rb) = (a.rect.translate(offset), b.rect.translate(offset));
                if matches!(f.dir, Direction::Right | Direction::Left) {
                    let y = ra.bottom().max(rb.bottom()) + lay.bulge * 0.9;
                    bezier([ra.center_bottom(), pos2(ra.center().x, y), pos2(rb.center().x, y), rb.center_bottom()])
                } else {
                    let x = ra.right().max(rb.right()) + lay.bulge * 0.9;
                    bezier([ra.right_center(), pos2(x, ra.center().y), pos2(x, rb.center().y), rb.right_center()])
                }
            } else {
                vec![boundary(a, b.rect.center()) + offset, boundary(b, a.rect.center()) + offset]
            };
            draw_edge(&painter, &points, e, pal.muted, scale);
            if !e.label.is_empty() {
                let mid = if backwards { points.get(points.len() / 2).copied() } else { points.first().zip(points.last()).map(|(p, q)| *p + (*q - *p) / 2.0) };
                if let Some(at) = mid {
                    labels.push((at, e.label.clone()));
                }
            }
        }
        for p in &lay.placed {
            draw_node(&painter, p, offset, scale);
        }
        for (at, text) in labels {
            let g = label_galley(ui, &text, base * scale * 0.9, pal.text);
            let r = Rect::from_center_size(at, g.size() + vec2(6.0, 2.0));
            painter.rect_filled(r, CornerRadius::same(3), pal.bg);
            painter.galley(r.center() - g.rect.center().to_vec2(), g, pal.text);
        }
    });
}

// ---- mind map layout ---------------------------------------------------------------------

struct MindPlaced {
    placed: Placed,
    parent: Option<usize>,
    color: Color32,
}

fn mind_layout(ui: &Ui, root: &MindNode, pal: &Palette, font: f32) -> (Vec<MindPlaced>, Rect) {
    // Subtree heights on one side: leaves stack vertically, parents sit centred beside them.
    let gap_v = font * 0.6;
    let gap_h = font * 2.6;
    struct Ctx<'a> {
        ui: &'a Ui,
        pal: &'a Palette,
        font: f32,
        gap_v: f32,
        gap_h: f32,
        out: Vec<MindPlaced>,
    }
    fn measure(cx: &Ctx, n: &MindNode, depth: usize) -> f32 {
        let size = cx.font * if depth == 1 { 1.0 } else { 0.92 };
        let g = label_galley(cx.ui, &n.label, size, cx.pal.text);
        let own = node_size(if depth == 1 { NodeShape::Round } else { n.shape }, g.size(), size).y;
        if n.children.is_empty() || depth >= MAX_DEPTH {
            return own;
        }
        let kids: f32 = n.children.iter().map(|c| measure(cx, c, depth + 1)).sum::<f32>() + cx.gap_v * (n.children.len().saturating_sub(1)) as f32;
        own.max(kids)
    }
    #[allow(clippy::too_many_arguments)]
    fn place(cx: &mut Ctx, n: &MindNode, depth: usize, x: f32, top: f32, side: f32, parent: usize, color: Color32) {
        let size = cx.font * 0.92;
        let g = label_galley(cx.ui, &n.label, size, cx.pal.text);
        let s = node_size(n.shape, g.size(), size);
        let h = measure(cx, n, depth);
        let cy = top + h / 2.0;
        let rect = if side > 0.0 { Rect::from_min_size(pos2(x, cy - s.y / 2.0), s) } else { Rect::from_min_size(pos2(x - s.x, cy - s.y / 2.0), s) };
        let fill = cx.pal.fill.lerp_to_gamma(color, if cx.pal.bg.r() < 100 { 0.28 } else { 0.14 });
        cx.out.push(MindPlaced { placed: Placed { rect, shape: n.shape, galley: g, fill, stroke: color }, parent: Some(parent), color });
        let me = cx.out.len() - 1;
        if depth >= MAX_DEPTH || n.children.is_empty() {
            return;
        }
        let kids_h: f32 = n.children.iter().map(|c| measure(cx, c, depth + 1)).sum::<f32>() + cx.gap_v * (n.children.len().saturating_sub(1)) as f32;
        let mut t = cy - kids_h / 2.0;
        let nx = if side > 0.0 { rect.right() + cx.gap_h } else { rect.left() - cx.gap_h };
        for c in &n.children {
            let ch = measure(cx, c, depth + 1);
            place(cx, c, depth + 1, nx, t, side, me, color);
            t += ch + cx.gap_v;
        }
    }
    let mut cx = Ctx { ui, pal, font, gap_v, gap_h, out: Vec::new() };
    let rg = label_galley(ui, &root.label, font * 1.15, pal.text);
    // A circle only for short titles; long ones get a pill.
    let root_shape = if root.shape == NodeShape::Circle && root.label.chars().count() <= 12 { NodeShape::Circle } else { NodeShape::Stadium };
    let rs = node_size(root_shape, rg.size(), font * 1.15);
    let root_rect = Rect::from_center_size(Pos2::ZERO, rs);
    cx.out.push(MindPlaced {
        placed: Placed { rect: root_rect, shape: root_shape, galley: rg, fill: pal.accent, stroke: pal.accent },
        parent: None,
        color: pal.accent,
    });
    // Split the root's children between the right and the left side by height.
    let heights: Vec<f32> = root.children.iter().map(|c| cx_measure(&cx, c)).collect();
    fn cx_measure(cx: &Ctx, c: &MindNode) -> f32 {
        measure(cx, c, 1)
    }
    let total: f32 = heights.iter().sum();
    let mut right = Vec::new();
    let mut left = Vec::new();
    let mut acc = 0.0;
    for (i, h) in heights.iter().enumerate() {
        if acc + h / 2.0 <= total / 2.0 || right.is_empty() {
            right.push(i);
            acc += h;
        } else {
            left.push(i);
        }
    }
    for (side, list) in [(1.0f32, &right), (-1.0, &left)] {
        let hs: f32 = list.iter().filter_map(|&i| heights.get(i)).sum::<f32>() + gap_v * list.len().saturating_sub(1) as f32;
        let mut t = -hs / 2.0;
        let x = if side > 0.0 { root_rect.right() + gap_h } else { root_rect.left() - gap_h };
        for &i in list.iter() {
            let (Some(c), Some(h)) = (root.children.get(i), heights.get(i)) else { continue };
            let color = BRANCHES.get(i % BRANCHES.len()).copied().unwrap_or(pal.accent);
            place(&mut cx, c, 1, x, t, side, 0, color);
            t += h + gap_v;
        }
    }
    let mut bounds = Rect::NOTHING;
    for p in &cx.out {
        bounds = bounds.union(p.placed.rect);
    }
    (cx.out, bounds)
}

/// Draw a mind map: the root in the middle, branches to both sides.
pub fn show_mindmap(ui: &mut Ui, root: &MindNode, pal: &Palette) {
    let base = 13.0;
    let avail = ui.available_width().max(100.0);
    let (mut nodes, mut bounds) = mind_layout(ui, root, pal, base);
    let mut scale = 1.0;
    if bounds.width() > avail {
        scale = (avail / bounds.width()).max(0.45);
        (nodes, bounds) = mind_layout(ui, root, pal, base * scale);
    }
    let pad = 6.0;
    let want = bounds.size() + Vec2::splat(pad * 2.0);
    egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).auto_shrink([false, true]).show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(want.x.max(ui.available_width()), want.y), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let offset = rect.min.to_vec2() - bounds.min.to_vec2() + vec2(((rect.width() - want.x) / 2.0).max(0.0) + pad, pad);
        for n in &nodes {
            let Some(p) = n.parent.and_then(|i| nodes.get(i)) else { continue };
            let (a, b) = (p.placed.rect.translate(offset), n.placed.rect.translate(offset));
            let right = b.center().x > a.center().x;
            let start = if right { a.right_center() } else { a.left_center() };
            let end = if right { b.left_center() } else { b.right_center() };
            let start = if p.parent.is_none() { boundary(&p.placed, n.placed.rect.center()) + offset } else { start };
            let mid = (end.x - start.x) / 2.0;
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                [start, start + vec2(mid, 0.0), end - vec2(mid, 0.0), end],
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0 * scale.max(0.6), n.color),
            );
            painter.add(curve);
        }
        for n in &nodes {
            if n.parent.is_none() {
                // White text on the accent.
                let galley = label_galley(ui, &root.label, base * scale * 1.15, Color32::WHITE);
                let p = Placed { rect: n.placed.rect, shape: n.placed.shape, galley, fill: n.placed.fill, stroke: n.placed.stroke };
                draw_node(&painter, &p, offset, scale);
            } else {
                draw_node(&painter, &n.placed, offset, scale);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(src: &str) -> Flowchart {
        match parse(src) {
            Ok(Diagram::Flowchart(f)) => f,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_flowcharts() {
        let f = flow("graph TD\n  A[Start] --> B{Ist es warm?}\n  B -->|Ja| C(Draußen spielen)\n  B -- Nein --> D([Drinnen lesen])\n  C & D --> E((Ende))");
        assert_eq!(f.dir, Direction::Down);
        assert_eq!(f.nodes.len(), 5);
        assert_eq!(f.nodes[1].shape, NodeShape::Rhombus);
        assert_eq!(f.nodes[1].label, "Ist es warm?");
        assert_eq!(f.nodes[3].shape, NodeShape::Stadium);
        assert_eq!(f.nodes[4].shape, NodeShape::Circle);
        assert_eq!(f.edges.len(), 5);
        assert_eq!(f.edges[1].label, "Ja");
        assert_eq!(f.edges[2].label, "Nein");
        assert!(f.edges.iter().all(|e| e.arrow));
        assert!(!f.truncated);

        let f = flow("flowchart LR; a-->b; b-.->c; c==>d; d---e; e--oa\nclassDef x fill:#f00\nclass a x\n%% Kommentar");
        assert_eq!(f.dir, Direction::Right);
        assert_eq!(f.edges.len(), 5);
        assert!(f.edges[1].dotted && f.edges[1].arrow);
        assert!(f.edges[2].thick);
        assert!(!f.edges[3].arrow);
        assert!(!f.truncated, "classDef, class and comments are skipped");

        let f =
            flow("graph TD\nA[\"Text mit (Klammern) und ]\"] --> B[/Eingabe/]\nB --> C[(Datenbank)]\nC --> D{{Sechseck}}\nsubgraph S [Gruppe]\nD --> A\nend");
        assert_eq!(f.nodes[0].label, "Text mit (Klammern) und ]");
        assert_eq!(f.nodes[1].shape, NodeShape::Parallelogram);
        assert_eq!(f.nodes[2].shape, NodeShape::Cylinder);
        assert_eq!(f.nodes[3].shape, NodeShape::Hexagon);
        assert_eq!(f.edges.len(), 4);
        // The cycle D → A is broken for ranking, all nodes get a rank.
        let r = flow_ranks(&f);
        assert_eq!(r, vec![0, 1, 2, 3]);
    }

    #[test]
    fn reads_mindmaps() {
        let Ok(Diagram::Mindmap(m)) =
            parse("mindmap\n  root((Wald))\n    Bäume\n      Eiche\n      Buche\n    Tiere\n      ::icon(fa fa-paw)\n      Reh\n    Boden[Waldboden]")
        else {
            panic!()
        };
        assert_eq!(m.label, "Wald");
        assert_eq!(m.shape, NodeShape::Circle);
        assert_eq!(m.children.len(), 3);
        assert_eq!(m.children[0].children.len(), 2);
        assert_eq!(m.children[1].children[0].label, "Reh");
        assert_eq!(m.children[2].label, "Waldboden");
        assert_eq!(mindmap_outline(&m), "- Wald\n  - Bäume\n    - Eiche\n    - Buche\n  - Tiere\n    - Reh\n  - Waldboden\n");
    }

    #[test]
    fn other_types_and_hostile_sources() {
        assert_eq!(parse("sequenceDiagram\nA->>B: Hallo"), Ok(Diagram::Unsupported("sequenceDiagram".into())));
        assert!(parse("").is_err());
        assert!(parse("graph TD").is_err());
        assert!(parse(&"x".repeat(MAX_SOURCE + 1)).is_err());
        // Garbage never panics.
        for src in [
            "graph TD\n-->",
            "graph TD\nA-->",
            "graph TD\nA[unclosed --> B",
            "graph\n||||",
            "flowchart TD\nA -- ohne Ende",
            "mindmap",
            "mindmap\n\t\t\tx\ny\n    z",
        ] {
            let _ = parse(src);
        }
        // Caps hold.
        let many: String = (0..1000).map(|i| format!("n{i} --> n{}\n", i + 1)).collect();
        let f = flow(&format!("graph TD\n{many}"));
        assert_eq!(f.nodes.len(), MAX_NODES);
        assert!(f.truncated);
        let dense: String =
            (0..40).map(|i| format!("a{i} & ")).collect::<String>() + "x --> " + &(0..40).map(|i| format!("b{i}")).collect::<Vec<_>>().join(" & ");
        let f = flow(&format!("graph TD\n{dense}"));
        assert!(f.edges.len() <= MAX_EDGES);
        let deep: String = (0..100).map(|i| format!("{}n{i}\n", " ".repeat(i + 1))).collect();
        let Ok(Diagram::Mindmap(m)) = parse(&format!("mindmap\n{deep}")) else { panic!() };
        fn depth(n: &MindNode) -> usize {
            1 + n.children.iter().map(depth).max().unwrap_or(0)
        }
        assert!(depth(&m) <= MAX_DEPTH + 1);
    }
}
