//! The whiteboard output format `aiscraft-board` v1 (#56): what a model delivers when it is
//! asked for a whiteboard. Content only, no coordinates; the layout is ours (mind map through
//! [`crate::outline`], clusters, flows and timelines here).
//!
//! ```json
//! {"title": "Wasserkreislauf", "layout": "mindmap",
//!  "nodes": [{"id": "a", "text": "Wasserkreislauf"}, {"id": "b", "text": "Verdunstung", "color": "blue", "group": "g1"}],
//!  "links": [{"from": "a", "to": "b", "label": "beginnt mit"}],
//!  "groups": [{"id": "g1", "title": "Vorgänge"}]}
//! ```
//!
//! The parser is lenient (unknown fields ignored, broken parts reported, never a panic) and
//! caps everything: nodes, links, groups, text length, depth.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::Value;

use crate::model::{Board, Element, Kind, truncate_chars};
use crate::outline::note_height;

/// The fence language of the format.
pub const FENCE: &str = "aiscraft-board";
/// Largest block accepted.
pub const MAX_SPEC_BYTES: usize = 256 * 1024;
pub const MAX_NODES: usize = 120;
pub const MAX_LINKS: usize = 240;
pub const MAX_GROUPS: usize = 12;
pub const MAX_NODE_TEXT: usize = 160;
/// Levels below the root in a mind map.
pub const MAX_DEPTH: usize = 5;
const MAX_IDLEN: usize = 64;
const MAX_TITLE: usize = 120;

const NODE_W: f64 = 200.0;
const GAP: f64 = 40.0;
const FRAME_PAD: f64 = 24.0;
const FRAME_HEAD: f64 = 48.0;
const PER_ROW: usize = 4;
const GROUP_COLORS: [&str; 6] = ["yellow", "blue", "green", "orange", "violet", "pink"];

/// How the cards are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpecLayout {
    /// A root in the middle, branches around it (the links form the tree).
    #[default]
    Mindmap,
    /// Each group in a frame, cards inside.
    Cluster,
    /// Steps left to right in the order of the links.
    Flow,
    /// Cards in their order along a time line.
    Timeline,
}

impl SpecLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            SpecLayout::Mindmap => "mindmap",
            SpecLayout::Cluster => "cluster",
            SpecLayout::Flow => "flow",
            SpecLayout::Timeline => "timeline",
        }
    }

    pub fn parse(s: &str) -> Option<SpecLayout> {
        match s.trim().to_lowercase().as_str() {
            "mindmap" | "mind-map" | "mind_map" | "mindmappe" => Some(SpecLayout::Mindmap),
            "cluster" | "clusters" | "gruppen" => Some(SpecLayout::Cluster),
            "flow" | "ablauf" | "flowchart" | "process" => Some(SpecLayout::Flow),
            "timeline" | "zeitleiste" | "zeitstrahl" => Some(SpecLayout::Timeline),
            _ => None,
        }
    }

    /// German name for cards and notices.
    pub fn label(self) -> &'static str {
        match self {
            SpecLayout::Mindmap => "Mindmap",
            SpecLayout::Cluster => "Cluster",
            SpecLayout::Flow => "Ablauf",
            SpecLayout::Timeline => "Zeitleiste",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecNode {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecLink {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecGroup {
    pub id: String,
    pub title: String,
}

/// A whiteboard as content: cards, links, groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct BoardSpec {
    pub title: String,
    pub layout: SpecLayout,
    pub nodes: Vec<SpecNode>,
    pub links: Vec<SpecLink>,
    pub groups: Vec<SpecGroup>,
}

/// What a model is told about the format.
pub fn prompt_schema() -> &'static str {
    "Format aiscraft-board (JSON, nur Inhalt, keine Koordinaten): {\"title\": \"…\", \"layout\": \"mindmap\" | \"cluster\" | \"flow\" | \"timeline\", \"nodes\": [{\"id\": \"a\", \"text\": \"kurzer Text\", \"color\"?: \"yellow|orange|red|pink|violet|blue|green|gray\", \"group\"?: \"g1\"}], \"links\": [{\"from\": \"a\", \"to\": \"b\", \"label\"?: \"…\"}], \"groups\": [{\"id\": \"g1\", \"title\": \"…\"}]}\n- mindmap: der erste Knoten ist die Mitte, links bilden den Baum (höchstens 5 Ebenen); cluster: jede Gruppe ein Rahmen; flow: Schritte in der Reihenfolge der links; timeline: Knoten in zeitlicher Reihenfolge, Datum am Anfang des Texts\n- höchstens 120 Knoten, Texte höchstens 160 Zeichen, Deutsch\nBeispiel: {\"title\":\"Wasserkreislauf\",\"layout\":\"mindmap\",\"nodes\":[{\"id\":\"a\",\"text\":\"Wasserkreislauf\"},{\"id\":\"b\",\"text\":\"Verdunstung\"},{\"id\":\"c\",\"text\":\"Niederschlag\"}],\"links\":[{\"from\":\"a\",\"to\":\"b\"},{\"from\":\"a\",\"to\":\"c\"}],\"groups\":[]}"
}

fn clean_text(s: &str, max: usize) -> String {
    let mut t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    t.retain(|c| !c.is_control());
    truncate_chars(&mut t, max);
    t
}

fn clean_id(v: Option<&Value>) -> Option<String> {
    let s = match v? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let mut s: String = s.chars().filter(|c| !c.is_control()).collect();
    truncate_chars(&mut s, MAX_IDLEN);
    (!s.is_empty()).then_some(s)
}

/// German and English colour names to the board palette.
fn color(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim().to_lowercase();
    let c = match s.as_str() {
        "yellow" | "gelb" => "yellow",
        "orange" => "orange",
        "red" | "rot" => "red",
        "pink" | "rosa" => "pink",
        "violet" | "purple" | "lila" | "violett" => "violet",
        "blue" | "blau" => "blue",
        "green" | "grün" | "gruen" => "green",
        "gray" | "grey" | "grau" => "gray",
        _ => return None,
    };
    Some(c.to_string())
}

/// The JSON object in a block: the block itself, or the part from the first `{` to the last `}`.
fn object_of(src: &str) -> Result<Value, String> {
    if src.len() > MAX_SPEC_BYTES {
        return Err(format!("zu groß (höchstens {} KB)", MAX_SPEC_BYTES / 1024));
    }
    let t = src.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) else { return Err("kein JSON-Objekt gefunden".into()) };
    let inner = t.get(a..=b).ok_or("kein JSON-Objekt gefunden")?;
    serde_json::from_str::<Value>(inner).map_err(|e| format!("kein gültiges JSON: {e}"))
}

/// Parses an `aiscraft-board` block: the board and the problems found on the way (skipped
/// nodes, unknown references, caps). Unusable input (no JSON, no nodes) is an error.
pub fn parse_lenient(src: &str) -> Result<(BoardSpec, Vec<String>), String> {
    let v = object_of(src)?;
    from_value(&v)
}

/// [`parse_lenient`] for JSON already parsed (a tool's arguments).
pub fn from_value(v: &Value) -> Result<(BoardSpec, Vec<String>), String> {
    let obj = v.as_object().ok_or("das JSON ist kein Objekt")?;
    let mut problems = Vec::new();
    let mut spec = BoardSpec { title: clean_text(obj.get("title").and_then(Value::as_str).unwrap_or(""), MAX_TITLE), ..BoardSpec::default() };
    if let Some(l) = obj.get("layout").and_then(Value::as_str) {
        match SpecLayout::parse(l) {
            Some(l) => spec.layout = l,
            None => problems.push(format!("Anordnung „{}“ unbekannt, als Mindmap gezeichnet", clean_text(l, 30))),
        }
    }
    let empty = Vec::new();
    let groups = obj.get("groups").and_then(Value::as_array).unwrap_or(&empty);
    for g in groups {
        if spec.groups.len() >= MAX_GROUPS {
            problems.push(format!("nur die ersten {MAX_GROUPS} Gruppen übernommen"));
            break;
        }
        let Some(id) = clean_id(g.get("id")) else {
            problems.push("eine Gruppe ohne id übersprungen".into());
            continue;
        };
        if spec.groups.iter().any(|x| x.id == id) {
            problems.push(format!("Gruppe {id} doppelt"));
            continue;
        }
        let title = clean_text(g.get("title").or_else(|| g.get("text")).and_then(Value::as_str).unwrap_or(&id), MAX_NODE_TEXT);
        spec.groups.push(SpecGroup { id, title });
    }
    let nodes = obj.get("nodes").and_then(Value::as_array).ok_or("nodes fehlt (eine Liste von Karten)")?;
    let mut ids = HashSet::new();
    for (i, n) in nodes.iter().enumerate() {
        if spec.nodes.len() >= MAX_NODES {
            problems.push(format!("nur die ersten {MAX_NODES} Karten übernommen"));
            break;
        }
        // A bare string is a card too.
        let (id, text, col, group) = match n {
            Value::String(s) => (None, s.as_str(), None, None),
            Value::Object(o) => (
                clean_id(o.get("id")),
                o.get("text").or_else(|| o.get("label")).or_else(|| o.get("title")).and_then(Value::as_str).unwrap_or(""),
                color(o.get("color")),
                clean_id(o.get("group")),
            ),
            _ => {
                problems.push(format!("Karte {} ist weder Text noch Objekt", i + 1));
                continue;
            }
        };
        let text = clean_text(text, MAX_NODE_TEXT);
        if text.is_empty() {
            problems.push(format!("Karte {} ohne Text übersprungen", i + 1));
            continue;
        }
        let id = id.unwrap_or_else(|| format!("n{}", i + 1));
        if !ids.insert(id.clone()) {
            problems.push(format!("id {id} doppelt, zweite Karte übersprungen"));
            continue;
        }
        let group = match group {
            Some(g) if spec.groups.iter().any(|x| x.id == g) => Some(g),
            Some(g) => {
                problems.push(format!("Gruppe {g} gibt es nicht"));
                None
            }
            None => None,
        };
        spec.nodes.push(SpecNode { id, text, color: col, group });
    }
    if spec.nodes.is_empty() {
        return Err("keine Karten (nodes ist leer)".into());
    }
    let links = obj.get("links").or_else(|| obj.get("edges")).and_then(Value::as_array).unwrap_or(&empty);
    let mut seen = HashSet::new();
    for l in links {
        if spec.links.len() >= MAX_LINKS {
            problems.push(format!("nur die ersten {MAX_LINKS} Verbindungen übernommen"));
            break;
        }
        let (Some(from), Some(to)) = (clean_id(l.get("from").or_else(|| l.get("source"))), clean_id(l.get("to").or_else(|| l.get("target")))) else {
            problems.push("eine Verbindung ohne from/to übersprungen".into());
            continue;
        };
        if !ids.contains(&from) || !ids.contains(&to) {
            problems.push(format!("Verbindung {from} → {to}: Karte fehlt"));
            continue;
        }
        if from == to || !seen.insert((from.clone(), to.clone())) {
            continue;
        }
        let label = l.get("label").and_then(Value::as_str).map(|s| clean_text(s, 60)).filter(|s| !s.is_empty());
        spec.links.push(SpecLink { from, to, label });
    }
    if spec.title.is_empty() {
        spec.title = spec.nodes.first().map(|n| n.text.clone()).unwrap_or_default();
        truncate_chars(&mut spec.title, MAX_TITLE);
    }
    Ok((spec, problems))
}

impl BoardSpec {
    /// The tree a mind map shows: (root index, children per node index). The root is the first
    /// card without an incoming link that has outgoing ones (else the first card); cards the
    /// links don't reach hang under their group's title, or under the root.
    fn tree(&self) -> (usize, Vec<Vec<TreeItem>>) {
        let index: HashMap<&str, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
        let incoming: HashSet<&str> = self.links.iter().map(|l| l.to.as_str()).collect();
        let outgoing: HashSet<&str> = self.links.iter().map(|l| l.from.as_str()).collect();
        let root = self.nodes.iter().position(|n| !incoming.contains(n.id.as_str()) && outgoing.contains(n.id.as_str())).unwrap_or(0);
        let mut children: Vec<Vec<TreeItem>> = vec![Vec::new(); self.nodes.len()];
        let mut placed = vec![false; self.nodes.len()];
        let mut depth = vec![0usize; self.nodes.len()];
        if let Some(p) = placed.get_mut(root) {
            *p = true;
        }
        // Breadth first along the links, so each card sits at its shortest distance.
        let mut queue = std::collections::VecDeque::from([root]);
        while let Some(i) = queue.pop_front() {
            let Some(id) = self.nodes.get(i).map(|n| n.id.as_str()) else { continue };
            let d = depth.get(i).copied().unwrap_or(0);
            if d >= MAX_DEPTH {
                continue;
            }
            for l in self.links.iter().filter(|l| l.from == id) {
                let Some(&j) = index.get(l.to.as_str()) else { continue };
                if placed.get(j).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(p) = placed.get_mut(j) {
                    *p = true;
                }
                if let Some(x) = depth.get_mut(j) {
                    *x = d + 1;
                }
                if let Some(c) = children.get_mut(i) {
                    c.push(TreeItem::Node(j));
                }
                queue.push_back(j);
            }
        }
        // The rest: under their group (a heading of its own), else under the root.
        let mut by_group: Vec<(String, Vec<TreeItem>)> = Vec::new();
        for (j, n) in self.nodes.iter().enumerate() {
            if placed.get(j).copied().unwrap_or(true) {
                continue;
            }
            match n.group.as_ref().and_then(|g| self.groups.iter().find(|x| &x.id == g)) {
                Some(g) => match by_group.iter_mut().find(|(t, _)| t == &g.title) {
                    Some((_, items)) => items.push(TreeItem::Node(j)),
                    None => by_group.push((g.title.clone(), vec![TreeItem::Node(j)])),
                },
                None => {
                    if let Some(c) = children.get_mut(root) {
                        c.push(TreeItem::Node(j));
                    }
                }
            }
        }
        if let Some(c) = children.get_mut(root) {
            c.extend(by_group.into_iter().map(|(t, items)| TreeItem::Heading(t, items)));
        }
        (root, children)
    }

    /// The board as an indented Markdown list (the mind map's input, and what is added to an
    /// open board).
    pub fn to_outline(&self) -> String {
        let (root, children) = self.tree();
        let kids: HashMap<usize, &Vec<TreeItem>> = children.iter().enumerate().collect();
        let mut out = String::new();
        let _ = writeln!(out, "# {}", self.nodes.get(root).map(|n| n.text.clone()).unwrap_or_else(|| self.title.clone()));
        fn walk(out: &mut String, items: &[TreeItem], depth: usize, nodes: &[SpecNode], kids: &HashMap<usize, &Vec<TreeItem>>) {
            for item in items {
                let pad = "  ".repeat(depth);
                match item {
                    TreeItem::Node(j) => {
                        let Some(n) = nodes.get(*j) else { continue };
                        let _ = writeln!(out, "{pad}- {}", n.text);
                        if depth < MAX_DEPTH
                            && let Some(c) = kids.get(j)
                        {
                            walk(out, c, depth + 1, nodes, kids);
                        }
                    }
                    TreeItem::Heading(t, c) => {
                        let _ = writeln!(out, "{pad}- {t}");
                        walk(out, c, depth + 1, nodes, kids);
                    }
                }
            }
        }
        if let Some(c) = kids.get(&root) {
            walk(&mut out, c, 0, &self.nodes, &kids);
        }
        out
    }

    /// The board as `aiscraft-board` JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
enum TreeItem {
    Node(usize),
    Heading(String, Vec<TreeItem>),
}

fn note(id: String, text: &str, x: f64, y: f64, color: &str) -> Element {
    let mut el = Element::new(Kind::Note, x, y).with_size(NODE_W, note_height(text, NODE_W)).with_text(text).with_color(color);
    el.id = id;
    el
}

/// Arrows for the links between placed cards.
fn connectors(spec: &BoardSpec, placed: &HashMap<String, Element>) -> Vec<Element> {
    spec.links
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let (a, b) = (placed.get(&l.from)?, placed.get(&l.to)?);
            let mut e = Element::connector(a, b).with_color("gray");
            e.id = format!("sl{i}");
            if let Some(t) = &l.label {
                e.text = t.clone();
            }
            Some(e)
        })
        .collect()
}

fn el_id(n: &SpecNode) -> String {
    format!("s-{}", n.id)
}

/// Cards in rows of [`PER_ROW`], in the given order; returns the elements by spec id.
fn grid(nodes: &[&SpecNode], x0: f64, y0: f64, cols: usize, color_of: &dyn Fn(&SpecNode) -> String) -> (Vec<Element>, f64, f64) {
    let cols = cols.max(1);
    let mut els = Vec::new();
    let (mut y, mut row_h, mut w) = (y0, 0.0_f64, 0.0_f64);
    for (k, n) in nodes.iter().enumerate() {
        let col = k % cols;
        if col == 0 && k > 0 {
            y += row_h + GAP / 2.0;
            row_h = 0.0;
        }
        let x = x0 + col as f64 * (NODE_W + GAP / 2.0);
        let e = note(el_id(n), &n.text, x, y, &color_of(n));
        row_h = row_h.max(e.h);
        w = w.max(x + NODE_W - x0);
        els.push(e);
    }
    (els, w, y + row_h - y0)
}

impl Board {
    /// A new board from an `aiscraft-board` spec, laid out by its `layout`.
    pub fn from_spec(spec: &BoardSpec) -> Board {
        let mut b = match spec.layout {
            SpecLayout::Mindmap => Board::from_outline(&spec.to_outline()),
            _ => {
                let mut b = Board::new();
                b.add_many(layout_elements(spec));
                b.clear_history();
                b
            }
        };
        b.set_title(&spec.title);
        b
    }
}

fn node_color(n: &SpecNode, fallback: &str) -> String {
    n.color.clone().unwrap_or_else(|| fallback.to_string())
}

/// Elements of a cluster, flow or timeline board.
fn layout_elements(spec: &BoardSpec) -> Vec<Element> {
    let mut els = Vec::new();
    let mut placed: HashMap<String, Element> = HashMap::new();
    match spec.layout {
        SpecLayout::Cluster => {
            // One frame per group (in the groups' order), the rest in a frame „Weitere“.
            let mut sets: Vec<(String, Vec<&SpecNode>)> = spec.groups.iter().map(|g| (g.title.clone(), Vec::new())).collect();
            let mut rest = Vec::new();
            for n in &spec.nodes {
                match n.group.as_ref().and_then(|g| spec.groups.iter().position(|x| &x.id == g)).and_then(|i| sets.get_mut(i)) {
                    Some((_, v)) => v.push(n),
                    None => rest.push(n),
                }
            }
            if !rest.is_empty() {
                sets.push((if spec.groups.is_empty() { spec.title.clone() } else { "Weitere".into() }, rest));
            }
            sets.retain(|(_, v)| !v.is_empty());
            let (mut x, mut y, mut row_h) = (0.0_f64, 0.0_f64, 0.0_f64);
            for (gi, (title, nodes)) in sets.iter().enumerate() {
                let fallback = GROUP_COLORS.get(gi % GROUP_COLORS.len()).copied().unwrap_or("yellow");
                let cols = if nodes.len() > 4 { 2 } else { 1 };
                let (cards, w, h) = grid(nodes, x + FRAME_PAD, y + FRAME_HEAD, cols, &|n| node_color(n, fallback));
                let mut frame = Element::new(Kind::Frame, x, y).with_size(w + 2.0 * FRAME_PAD, h + FRAME_HEAD + FRAME_PAD).with_text(title.clone());
                frame.id = format!("sg{gi}");
                let fw = frame.w;
                row_h = row_h.max(frame.h);
                els.push(frame);
                for c in cards {
                    placed.insert(c.id.trim_start_matches("s-").to_string(), c.clone());
                    els.push(c);
                }
                x += fw + GAP;
                if (gi + 1) % 3 == 0 {
                    x = 0.0;
                    y += row_h + GAP;
                    row_h = 0.0;
                }
            }
        }
        SpecLayout::Flow | SpecLayout::Timeline => {
            let order = if spec.layout == SpecLayout::Flow { flow_order(spec) } else { (0..spec.nodes.len()).collect() };
            let nodes: Vec<&SpecNode> = order.iter().filter_map(|&i| spec.nodes.get(i)).collect();
            if spec.layout == SpecLayout::Flow {
                let (cards, _, _) = grid(&nodes, 0.0, 0.0, PER_ROW, &|n| node_color(n, "blue"));
                for c in cards {
                    placed.insert(c.id.trim_start_matches("s-").to_string(), c.clone());
                    els.push(c);
                }
            } else {
                // Alternately above and below a line.
                let step = NODE_W * 0.6 + GAP / 2.0;
                let axis_y = 0.0;
                let mut axis = Element::segment(Kind::Arrow, (-GAP, axis_y), (nodes.len() as f64 * step + NODE_W * 0.4 + GAP, axis_y)).with_color("ink");
                axis.id = "s-axis".into();
                els.push(axis);
                for (k, n) in nodes.iter().enumerate() {
                    let h = note_height(&n.text, NODE_W);
                    let x = k as f64 * step;
                    let y = if k % 2 == 0 { axis_y - GAP - h } else { axis_y + GAP };
                    let c = note(el_id(n), &n.text, x, y, &node_color(n, "orange"));
                    let mut tick =
                        Element::segment(Kind::Line, (x + NODE_W / 2.0, axis_y), (x + NODE_W / 2.0, if k % 2 == 0 { y + h } else { y })).with_color("gray");
                    tick.id = format!("st{k}");
                    els.push(tick);
                    placed.insert(n.id.clone(), c.clone());
                    els.push(c);
                }
            }
            // A flow without links still reads left to right: connect the steps in order.
            if spec.layout == SpecLayout::Flow && spec.links.is_empty() {
                for (k, w) in nodes.windows(2).enumerate() {
                    if let [a, b] = w
                        && let (Some(ea), Some(eb)) = (placed.get(&a.id), placed.get(&b.id))
                    {
                        let mut e = Element::connector(ea, eb).with_color("gray");
                        e.id = format!("sf{k}");
                        els.push(e);
                    }
                }
            }
        }
        SpecLayout::Mindmap => {}
    }
    els.extend(connectors(spec, &placed));
    els
}

/// Cards in the order of the links (sources first); cards outside every link keep their place
/// at the end.
fn flow_order(spec: &BoardSpec) -> Vec<usize> {
    let index: HashMap<&str, usize> = spec.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut indeg = vec![0usize; spec.nodes.len()];
    for l in &spec.links {
        if let Some(&j) = index.get(l.to.as_str())
            && let Some(d) = indeg.get_mut(j)
        {
            *d += 1;
        }
    }
    let mut order = Vec::with_capacity(spec.nodes.len());
    let mut done = vec![false; spec.nodes.len()];
    let mut ready: std::collections::VecDeque<usize> = (0..spec.nodes.len()).filter(|&i| indeg.get(i) == Some(&0)).collect();
    while let Some(i) = ready.pop_front() {
        if done.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some(d) = done.get_mut(i) {
            *d = true;
        }
        order.push(i);
        let Some(id) = spec.nodes.get(i).map(|n| n.id.as_str()) else { continue };
        for l in spec.links.iter().filter(|l| l.from == id) {
            if let Some(&j) = index.get(l.to.as_str())
                && let Some(d) = indeg.get_mut(j)
            {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    ready.push_back(j);
                }
            }
        }
    }
    // Cycles: the rest in the given order.
    order.extend((0..spec.nodes.len()).filter(|&i| !done.get(i).copied().unwrap_or(true)));
    order
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const SAMPLE: &str = r#"{"title":"Wasserkreislauf","layout":"mindmap","nodes":[{"id":"a","text":"Wasserkreislauf"},{"id":"b","text":"Verdunstung","color":"blau"},{"id":"c","text":"Niederschlag"},{"id":"d","text":"Regen"},{"id":"e","text":"Grundwasser","group":"g"}],"links":[{"from":"a","to":"b"},{"from":"a","to":"c"},{"from":"c","to":"d","label":"z. B."}],"groups":[{"id":"g","title":"Speicher"}],"extra":1}"#;

    #[test]
    fn round_trip_and_outline() {
        let (spec, problems) = parse_lenient(SAMPLE).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(spec.nodes.len(), 5);
        assert_eq!(spec.nodes[1].color.as_deref(), Some("blue"));
        let again = parse_lenient(&spec.to_json()).unwrap().0;
        assert_eq!(again, spec);
        let outline = spec.to_outline();
        assert_eq!(outline, "# Wasserkreislauf\n- Verdunstung\n- Niederschlag\n  - Regen\n- Speicher\n  - Grundwasser\n");
        let b = Board::from_spec(&spec);
        assert_eq!(b.title(), "Wasserkreislauf");
        assert!(b.summary_text().contains("Grundwasser"));
    }

    #[test]
    fn every_layout_places_every_card() {
        for layout in ["cluster", "flow", "timeline", "mindmap"] {
            let src = SAMPLE.replace("\"mindmap\"", &format!("\"{layout}\""));
            let (spec, _) = parse_lenient(&src).unwrap();
            let b = Board::from_spec(&spec);
            let notes: Vec<_> = b.elements().into_iter().filter(|e| e.kind == Kind::Note).collect();
            assert_eq!(notes.len(), if layout == "mindmap" { 6 } else { 5 }, "{layout}");
            // No two cards on top of each other.
            for (i, a) in notes.iter().enumerate() {
                for c in notes.iter().skip(i + 1) {
                    let overlap = a.x < c.x + c.w && c.x < a.x + a.w && a.y < c.y + c.h && c.y < a.y + a.h;
                    assert!(!overlap, "{layout}: {} / {}", a.text, c.text);
                }
            }
        }
    }

    #[test]
    fn hostile_input_is_reported_not_fatal() {
        assert!(parse_lenient("").is_err());
        assert!(parse_lenient("{kaputt").is_err());
        assert!(parse_lenient("[]").is_err());
        assert!(parse_lenient("{\"nodes\":[]}").is_err());
        assert!(parse_lenient(&"x".repeat(MAX_SPEC_BYTES + 1)).is_err());
        // Text before and after the object, bare strings, broken parts.
        let (spec, problems) = parse_lenient(
            "Hier: {\"layout\":\"spirale\",\"nodes\":[\"Eins\",{\"id\":\"x\"},{\"id\":1,\"text\":\"Zwei\",\"group\":\"nix\"},{\"id\":1,\"text\":\"doppelt\"},7],\"links\":[{\"from\":\"n1\",\"to\":\"zz\"},{\"from\":\"n1\",\"to\":\"1\"},{\"from\":\"n1\",\"to\":\"n1\"}]} danke",
        )
        .unwrap();
        assert_eq!(spec.nodes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), vec!["Eins", "Zwei"]);
        assert_eq!(spec.layout, SpecLayout::Mindmap);
        assert_eq!(spec.links.len(), 1);
        assert_eq!(spec.title, "Eins");
        assert!(problems.len() >= 5, "{problems:?}");
        // Caps.
        let many: Vec<String> = (0..200).map(|i| format!("{{\"id\":\"{i}\",\"text\":\"{}\"}}", "ä".repeat(200))).collect();
        let (spec, problems) = parse_lenient(&format!("{{\"nodes\":[{}]}}", many.join(","))).unwrap();
        assert_eq!(spec.nodes.len(), MAX_NODES);
        assert!(spec.nodes.iter().all(|n| n.text.chars().count() <= MAX_NODE_TEXT));
        assert!(!problems.is_empty());
        // A chain deeper than the cap still lays out.
        let chain: Vec<String> = (0..50).map(|i| format!("{{\"from\":\"{i}\",\"to\":\"{}\"}}", i + 1)).collect();
        let nodes: Vec<String> = (0..51).map(|i| format!("{{\"id\":\"{i}\",\"text\":\"K{i}\"}}")).collect();
        let (spec, _) = parse_lenient(&format!("{{\"nodes\":[{}],\"links\":[{}]}}", nodes.join(","), chain.join(","))).unwrap();
        let _ = Board::from_spec(&spec);
        for l in ["flow", "timeline", "cluster"] {
            let mut s = spec.clone();
            s.layout = SpecLayout::parse(l).unwrap();
            let _ = Board::from_spec(&s);
        }
    }
}
