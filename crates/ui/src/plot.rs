//! Diagrams AIS.chat draws with its `create_plot` tool. The model puts a JSXGraph spec into its
//! answer as a fenced block (```` ```jsxgraph-json ````); the web app renders it with JSXGraph.
//! AIS.ChatCraft has no browser, so it draws the 2D part itself: function graphs, curves,
//! points, segments, polygons, circles, texts and charts. What it cannot draw (3D, other
//! element types) is named below the drawing, and the data is always one click away.
//!
//! Everything here is hostile input: expressions are parsed by a small evaluator (no code runs),
//! sizes are capped, and anything odd is skipped instead of failing.

use std::collections::HashMap;

use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::Value;

/// The fence language AIS.chat uses for plots (`PLOT_LANGUAGE`).
pub const PLOT_LANGUAGE: &str = "jsxgraph-json";

/// A piece of an answer: Markdown, or a plot spec (raw JSON text).
#[derive(Debug, Clone, PartialEq)]
pub enum Segment<'a> {
    Markdown(&'a str),
    Plot(&'a str),
}

/// Split an answer at ```` ```jsxgraph-json ```` fences. An unclosed fence (still streaming) is
/// a plot that is not complete yet; it is left in the Markdown.
pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = find_fence(rest) {
        let after_lang = start + 3 + PLOT_LANGUAGE.len();
        let Some(line_end) = rest.get(after_lang..).and_then(|r| r.find('\n')).map(|i| after_lang + i + 1) else { break };
        let Some(body) = rest.get(line_end..) else { break };
        let Some(close) = closing_fence(body) else { break };
        if let Some(md) = rest.get(..start).filter(|m| !m.trim().is_empty()) {
            out.push(Segment::Markdown(md));
        }
        out.push(Segment::Plot(body.get(..close).unwrap_or("")));
        let after = (close + 3).min(body.len());
        rest = body.get(after..).unwrap_or("");
    }
    if !rest.trim().is_empty() || out.is_empty() {
        out.push(Segment::Markdown(rest));
    }
    out
}

fn find_fence(s: &str) -> Option<usize> {
    let pat = format!("```{PLOT_LANGUAGE}");
    let mut from = 0;
    while let Some(i) = s.get(from..)?.find(&pat) {
        let at = from + i;
        let at_line_start = at == 0 || s.get(..at).is_some_and(|b| b.ends_with('\n'));
        let next = s.get(at + pat.len()..).and_then(|r| r.chars().next());
        if at_line_start && next.is_none_or(|c| c == '\n' || c == ' ' || c == '\r') {
            return Some(at);
        }
        from = at + pat.len();
    }
    None
}

fn closing_fence(body: &str) -> Option<usize> {
    if body.starts_with("```") {
        return Some(0);
    }
    body.find("\n```").map(|i| i + 1)
}

// ---- expressions -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    Le,
    Ge,
    Eq,
    Ne,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > 2000 {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while chars.get(i).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                i += 1;
            }
            if chars.get(i).is_some_and(|c| *c == 'e' || *c == 'E') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit() || *c == '-' || *c == '+') {
                i += 2;
                while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    i += 1;
                }
            }
            let text: String = chars.get(start..i)?.iter().collect();
            out.push(Tok::Num(text.parse().ok()?));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while chars.get(i).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars.get(start..i)?.iter().collect()));
        } else {
            let next = chars.get(i + 1).copied();
            let two = match (c, next) {
                ('<', Some('=')) => Some(Tok::Le),
                ('>', Some('=')) => Some(Tok::Ge),
                ('=', Some('=')) => Some(Tok::Eq),
                ('!', Some('=')) => Some(Tok::Ne),
                ('*', Some('*')) => Some(Tok::Op('^')),
                _ => None,
            };
            if let Some(t) = two {
                out.push(t);
                i += 2;
            } else if "+-*/^%(),<>?:".contains(c) {
                out.push(Tok::Op(c));
                i += 1;
            } else {
                return None;
            }
        }
    }
    Some(out)
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn deeper(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth < 100).then_some(())
    }
    fn ternary(&mut self) -> Option<Expr> {
        self.deeper()?;
        let cond = self.comparison()?;
        let out = if self.eat(&Tok::Op('?')) {
            let a = self.ternary()?;
            if !self.eat(&Tok::Op(':')) {
                return None;
            }
            let b = self.ternary()?;
            Expr::If(Box::new(cond), Box::new(a), Box::new(b))
        } else {
            cond
        };
        self.depth -= 1;
        Some(out)
    }
    fn comparison(&mut self) -> Option<Expr> {
        let left = self.sum()?;
        let op = match self.peek() {
            Some(Tok::Op('<')) => "<",
            Some(Tok::Op('>')) => ">",
            Some(Tok::Le) => "<=",
            Some(Tok::Ge) => ">=",
            Some(Tok::Eq) => "==",
            Some(Tok::Ne) => "!=",
            _ => return Some(left),
        };
        self.pos += 1;
        let right = self.sum()?;
        Some(Expr::Cmp(op, Box::new(left), Box::new(right)))
    }
    fn sum(&mut self) -> Option<Expr> {
        let mut left = self.product()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.product()?;
            left = Expr::Bin(c, Box::new(left), Box::new(right));
        }
        Some(left)
    }
    fn product(&mut self) -> Option<Expr> {
        let mut left = self.unary()?;
        while let Some(Tok::Op(c @ ('*' | '/' | '%'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.unary()?;
            left = Expr::Bin(c, Box::new(left), Box::new(right));
        }
        Some(left)
    }
    fn unary(&mut self) -> Option<Expr> {
        self.deeper()?;
        let out = if self.eat(&Tok::Op('-')) {
            Expr::Neg(Box::new(self.unary()?))
        } else if self.eat(&Tok::Op('+')) {
            self.unary()?
        } else {
            self.power()?
        };
        self.depth -= 1;
        Some(out)
    }
    fn power(&mut self) -> Option<Expr> {
        let base = self.atom()?;
        if self.eat(&Tok::Op('^')) {
            // Right-associative, and -x^2 = -(x^2): the exponent may carry its own sign.
            let exp = self.unary()?;
            return Some(Expr::Bin('^', Box::new(base), Box::new(exp)));
        }
        Some(base)
    }
    fn atom(&mut self) -> Option<Expr> {
        match self.peek().cloned()? {
            Tok::Num(n) => {
                self.pos += 1;
                Some(Expr::Num(n))
            }
            Tok::Ident(name) => {
                self.pos += 1;
                if self.eat(&Tok::Op('(')) {
                    let mut args = Vec::new();
                    if !self.eat(&Tok::Op(')')) {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(&Tok::Op(')')) {
                                break;
                            }
                            if !self.eat(&Tok::Op(',')) || args.len() > 8 {
                                return None;
                            }
                        }
                    }
                    Some(Expr::Call(name, args))
                } else {
                    Some(Expr::Var(name))
                }
            }
            Tok::Op('(') => {
                self.pos += 1;
                let e = self.ternary()?;
                self.eat(&Tok::Op(')')).then_some(e)
            }
            _ => None,
        }
    }
}

/// Parse an expression in JessieCode's arithmetic subset; `None` for anything else.
pub fn parse_expr(s: &str) -> Option<Expr> {
    let toks = tokenize(s)?;
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.ternary()?;
    (p.pos == p.toks.len()).then_some(e)
}

/// Evaluate with variables (`x`, sliders). Unknown names and functions give NaN.
pub fn eval(e: &Expr, vars: &HashMap<String, f64>) -> f64 {
    eval_d(e, vars, 0)
}

fn eval_d(e: &Expr, vars: &HashMap<String, f64>, d: usize) -> f64 {
    if d > 200 {
        return f64::NAN;
    }
    let ev = |x: &Expr| eval_d(x, vars, d + 1);
    match e {
        Expr::Num(n) => *n,
        Expr::Var(v) => match v.as_str() {
            "PI" | "pi" => std::f64::consts::PI,
            "E" => std::f64::consts::E,
            other => vars.get(other).copied().unwrap_or(f64::NAN),
        },
        Expr::Neg(a) => -ev(a),
        Expr::Bin(op, a, b) => {
            let (a, b) = (ev(a), ev(b));
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                '%' => a % b,
                '^' => a.powf(b),
                _ => f64::NAN,
            }
        }
        Expr::Cmp(op, a, b) => {
            let (a, b) = (ev(a), ev(b));
            let r = match *op {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                ">=" => a >= b,
                "==" => a == b,
                _ => a != b,
            };
            if r { 1.0 } else { 0.0 }
        }
        Expr::If(c, a, b) => {
            if ev(c) != 0.0 {
                ev(a)
            } else {
                ev(b)
            }
        }
        Expr::Call(name, args) => {
            let a: Vec<f64> = args.iter().map(ev).collect();
            let x = a.first().copied().unwrap_or(f64::NAN);
            let y = a.get(1).copied().unwrap_or(f64::NAN);
            match name.as_str() {
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "asin" | "arcsin" => x.asin(),
                "acos" | "arccos" => x.acos(),
                "atan" | "arctan" => x.atan(),
                "atan2" => x.atan2(y),
                "sinh" => x.sinh(),
                "cosh" => x.cosh(),
                "tanh" => x.tanh(),
                "sqrt" => x.sqrt(),
                "cbrt" => x.cbrt(),
                "abs" => x.abs(),
                "exp" => x.exp(),
                "log" | "ln" => {
                    if a.len() == 2 {
                        x.ln() / y.ln()
                    } else {
                        x.ln()
                    }
                }
                "log10" | "lg" => x.log10(),
                "log2" | "ld" => x.log2(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "round" => x.round(),
                "trunc" => x.trunc(),
                "sign" => x.signum(),
                "pow" => x.powf(y),
                "min" => a.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => a.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                "nthroot" | "root" => x.powf(1.0 / y),
                "factorial" => {
                    if (0.0..=170.0).contains(&x) {
                        (1..=x.round() as u64).map(|k| k as f64).product()
                    } else {
                        f64::NAN
                    }
                }
                _ => f64::NAN,
            }
        }
    }
}

// ---- spec --------------------------------------------------------------------------------

/// What to draw, after reading the spec.
#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    /// x min, y max, x max, y min (JSXGraph's order).
    pub bbox: [f64; 4],
    pub axis: bool,
    pub grid: bool,
    pub shapes: Vec<Shape>,
    /// Slider names and the value they are drawn with.
    pub sliders: Vec<(String, f64)>,
    /// Element types left out.
    pub skipped: Vec<String>,
    pub x_label: Option<String>,
    pub y_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Sampled points of a graph or curve; NaN breaks the line.
    Path {
        points: Vec<[f64; 2]>,
        color: Option<Color32>,
        dashed: bool,
        fill: bool,
    },
    Point {
        at: [f64; 2],
        name: Option<String>,
        color: Option<Color32>,
    },
    Circle {
        center: [f64; 2],
        radius: f64,
        color: Option<Color32>,
    },
    Text {
        at: [f64; 2],
        text: String,
    },
    Bar {
        x: f64,
        width: f64,
        height: f64,
        color: Option<Color32>,
    },
}

const MAX_ELEMENTS: usize = 200;
const SAMPLES: usize = 400;

fn num(v: &Value, vars: &HashMap<String, f64>) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => Some(eval(&parse_expr(s)?, vars)).filter(|x| x.is_finite()),
        _ => None,
    }
}

fn color(attrs: Option<&Value>) -> Option<Color32> {
    let s = attrs?.get("strokeColor").or_else(|| attrs?.get("color")).or_else(|| attrs?.get("fillColor"))?.as_str()?;
    let hex = s.strip_prefix('#')?;
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match hex.len() {
        6 | 8 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        3 => {
            let nib = |i: usize| hex.get(i..i + 1).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|n| n * 17);
            Some(Color32::from_rgb(nib(0)?, nib(1)?, nib(2)?))
        }
        _ => None,
    }
}

/// Read a JSXGraph spec as AIS.chat's `parsePlotSpec` allows it.
pub fn parse_plot(src: &str) -> Result<Plot, String> {
    if src.len() > 512 * 1024 {
        return Err("Die Diagrammdaten sind zu groß".into());
    }
    let v: Value = serde_json::from_str(src.trim()).map_err(|e| format!("Die Diagrammdaten sind kein gültiges JSON ({e})"))?;
    let board = v.get("board").ok_or("Diagramm ohne „board“")?;
    let bb: Vec<f64> = board.get("boundingBox").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    let bbox = match bb.as_slice() {
        [a, b, c, d] if a < c && d < b && [*a, *b, *c, *d].iter().all(|x| x.is_finite()) => [*a, *b, *c, *d],
        _ => return Err("Diagramm ohne gültigen Bildausschnitt (boundingBox)".into()),
    };
    let axis = board.get("axis").and_then(Value::as_bool).unwrap_or(true);
    let grid = board.get("grid").and_then(Value::as_bool).unwrap_or(false);
    let axis_name = |k: &str| board.pointer(&format!("/defaultAxes/{k}/name")).and_then(Value::as_str).map(str::to_string);
    let elements = v.get("elements").and_then(Value::as_array).ok_or("Diagramm ohne Elemente")?;
    let mut plot = Plot { bbox, axis, grid, shapes: Vec::new(), sliders: Vec::new(), skipped: Vec::new(), x_label: axis_name("x"), y_label: axis_name("y") };
    let mut vars: HashMap<String, f64> = HashMap::new();
    let mut named: HashMap<String, [f64; 2]> = HashMap::new();
    let mut functions: HashMap<String, (Expr, f64, f64)> = HashMap::new();
    for el in elements.iter().take(MAX_ELEMENTS) {
        let Some(kind) = el.get(0).and_then(Value::as_str) else { continue };
        let parents: Vec<Value> = el.get(1).and_then(Value::as_array).cloned().unwrap_or_default();
        let attrs = el.get(2);
        let name = attrs.and_then(|a| a.get("name")).and_then(Value::as_str).map(str::to_string);
        let c = color(attrs);
        let dashed = attrs.and_then(|a| a.get("dash")).and_then(Value::as_f64).is_some_and(|d| d > 0.0);
        let visible = attrs.and_then(|a| a.get("visible")).and_then(Value::as_bool).unwrap_or(true);
        let coord = |p: &Value, vars: &HashMap<String, f64>, named: &HashMap<String, [f64; 2]>| -> Option<[f64; 2]> {
            match p {
                Value::String(n) => named.get(n).copied(),
                Value::Array(a) => Some([num(a.first()?, vars)?, num(a.get(1)?, vars)?]),
                _ => None,
            }
        };
        let mut drawn = true;
        match kind {
            "slider" => {
                // AIS.chat's simple form `[min, start, max]`, or JSXGraph's `[[x, y], [x, y], [min, start, max]]`.
                let vals: Vec<f64> = if parents.first().is_some_and(Value::is_number) {
                    parents.iter().filter_map(|x| num(x, &vars)).collect()
                } else {
                    parents.last().and_then(Value::as_array).map(|a| a.iter().filter_map(|x| num(x, &vars)).collect()).unwrap_or_default()
                };
                if let (Some(n), Some(start)) = (name.clone(), vals.get(1).copied()) {
                    vars.insert(n.clone(), start);
                    plot.sliders.push((n, start));
                }
            }
            "point" | "glider" => {
                let at = if parents.len() >= 2 {
                    (|| Some([num(parents.first()?, &vars)?, num(parents.get(1)?, &vars)?]))()
                } else {
                    parents.first().and_then(|p| coord(p, &vars, &named))
                };
                match at {
                    Some(at) => {
                        if let Some(n) = &name {
                            named.insert(n.clone(), at);
                        }
                        if visible {
                            plot.shapes.push(Shape::Point { at, name: name.clone(), color: c });
                        }
                    }
                    None => drawn = false,
                }
            }
            "midpoint" => {
                let pts: Vec<[f64; 2]> = parents.iter().filter_map(|p| coord(p, &vars, &named)).collect();
                match pts.as_slice() {
                    [a, b] => {
                        let at = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                        if let Some(n) = &name {
                            named.insert(n.clone(), at);
                        }
                        plot.shapes.push(Shape::Point { at, name: name.clone(), color: c });
                    }
                    _ => drawn = false,
                }
            }
            "segment" | "line" | "arrow" => {
                let pts: Vec<[f64; 2]> = parents.iter().take(2).filter_map(|p| coord(p, &vars, &named)).collect();
                match pts.as_slice() {
                    [a, b] if kind == "line" => {
                        // Extend across the view.
                        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                        let span = (bbox[2] - bbox[0]).abs() + (bbox[1] - bbox[3]).abs();
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 0.0 {
                            let k = span / len;
                            plot.shapes.push(Shape::Path {
                                points: vec![[a[0] - dx * k, a[1] - dy * k], [a[0] + dx * k, a[1] + dy * k]],
                                color: c,
                                dashed,
                                fill: false,
                            });
                        }
                    }
                    [a, b] => plot.shapes.push(Shape::Path { points: vec![*a, *b], color: c, dashed, fill: false }),
                    _ => drawn = false,
                }
            }
            "polygon" => {
                let pts: Vec<[f64; 2]> = parents.iter().filter_map(|p| coord(p, &vars, &named)).take(200).collect();
                if pts.len() >= 3 {
                    let mut closed = pts.clone();
                    closed.extend(pts.first().copied());
                    plot.shapes.push(Shape::Path { points: closed, color: c, dashed, fill: true });
                } else {
                    drawn = false;
                }
            }
            "circle" => {
                let center = parents.first().and_then(|p| coord(p, &vars, &named));
                let radius = parents.get(1).and_then(|p| match coord(p, &vars, &named) {
                    Some(q) if p.is_string() || p.is_array() => center.map(|c0| ((q[0] - c0[0]).powi(2) + (q[1] - c0[1]).powi(2)).sqrt()),
                    _ => num(p, &vars),
                });
                match (center, radius) {
                    (Some(center), Some(radius)) if radius.is_finite() && radius > 0.0 => plot.shapes.push(Shape::Circle { center, radius, color: c }),
                    _ => drawn = false,
                }
            }
            "functiongraph" => {
                let expr = parents.first().and_then(Value::as_str).and_then(parse_expr);
                let from = parents.get(1).and_then(|p| num(p, &vars)).unwrap_or(bbox[0]);
                let to = parents.get(2).and_then(|p| num(p, &vars)).unwrap_or(bbox[2]);
                match expr {
                    Some(e) if from < to => {
                        let points = sample(&e, from, to, &vars);
                        if let Some(n) = &name {
                            functions.insert(n.clone(), (e, from, to));
                        }
                        if visible {
                            plot.shapes.push(Shape::Path { points, color: c, dashed, fill: false });
                        }
                    }
                    _ => drawn = false,
                }
            }
            "integral" => {
                let range: Vec<f64> = parents.first().and_then(Value::as_array).map(|a| a.iter().filter_map(|x| num(x, &vars)).collect()).unwrap_or_default();
                let f = parents.get(1).and_then(Value::as_str).and_then(|n| functions.get(n)).cloned();
                match (range.as_slice(), f) {
                    ([a, b], Some((e, _, _))) if a < b => {
                        let mut points = vec![[*a, 0.0]];
                        points.extend(sample(&e, *a, *b, &vars).into_iter().filter(|p| p[1].is_finite()));
                        points.push([*b, 0.0]);
                        points.push([*a, 0.0]);
                        plot.shapes.push(Shape::Path { points, color: c.or(Some(Color32::from_rgb(0x3b, 0x82, 0xf6))), dashed: false, fill: true });
                    }
                    _ => drawn = false,
                }
            }
            "curve" => {
                let fx = parents.first().and_then(Value::as_str).and_then(parse_expr);
                let fy = parents.get(1).and_then(Value::as_str).and_then(parse_expr);
                let t0 = parents.get(2).and_then(|p| num(p, &vars));
                let t1 = parents.get(3).and_then(|p| num(p, &vars));
                match (fx, fy, t0, t1) {
                    (Some(fx), Some(fy), Some(t0), Some(t1)) if t0 < t1 => {
                        let mut v = vars.clone();
                        let points = (0..=SAMPLES)
                            .map(|i| {
                                let t = t0 + (t1 - t0) * i as f64 / SAMPLES as f64;
                                v.insert("x".into(), t);
                                v.insert("t".into(), t);
                                [eval(&fx, &v), eval(&fy, &v)]
                            })
                            .collect();
                        plot.shapes.push(Shape::Path { points, color: c, dashed, fill: false });
                    }
                    _ => drawn = false,
                }
            }
            "text" => {
                let at = (|| Some([num(parents.first()?, &vars)?, num(parents.get(1)?, &vars)?]))();
                let text = parents.get(2).and_then(Value::as_str).map(|s| s.chars().take(200).collect::<String>());
                match (at, text) {
                    (Some(at), Some(text)) => plot.shapes.push(Shape::Text { at, text }),
                    _ => drawn = false,
                }
            }
            "chart" => {
                let rows: Vec<Vec<f64>> =
                    parents.iter().filter_map(Value::as_array).map(|a| a.iter().filter_map(|x| num(x, &vars)).take(500).collect()).collect();
                let (xs, ys): (Vec<f64>, Vec<f64>) = match rows.as_slice() {
                    [ys] => ((1..=ys.len()).map(|i| i as f64).collect(), ys.clone()),
                    [xs, ys, ..] => (xs.clone(), ys.clone()),
                    _ => (vec![], vec![]),
                };
                let style = attrs.and_then(|a| a.get("chartStyle")).and_then(Value::as_str).unwrap_or("line");
                let width = attrs.and_then(|a| a.get("width")).and_then(Value::as_f64).unwrap_or(0.6);
                if xs.is_empty() || ys.is_empty() {
                    drawn = false;
                } else if style.contains("bar") {
                    for (x, y) in xs.iter().zip(&ys) {
                        plot.shapes.push(Shape::Bar { x: *x, width, height: *y, color: c });
                    }
                } else if style.contains("pie") {
                    drawn = false;
                } else {
                    plot.shapes.push(Shape::Path { points: xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect(), color: c, dashed, fill: false });
                }
            }
            // Drawn by the frame itself, or only hints for JSXGraph.
            "axis" | "grid" | "legend" | "measurement" | "ticks" => {}
            _ => drawn = false,
        }
        if !drawn && !plot.skipped.iter().any(|s| s == kind) {
            plot.skipped.push(kind.chars().take(40).collect());
        }
    }
    Ok(plot)
}

/// Sample y = f(x), breaking the line where it jumps (poles, gaps).
fn sample(e: &Expr, from: f64, to: f64, vars: &HashMap<String, f64>) -> Vec<[f64; 2]> {
    let mut v = vars.clone();
    (0..=SAMPLES)
        .map(|i| {
            let x = from + (to - from) * i as f64 / SAMPLES as f64;
            v.insert("x".into(), x);
            [x, eval(e, &v)]
        })
        .collect()
}

// ---- drawing -----------------------------------------------------------------------------

const PALETTE: [Color32; 5] = [
    Color32::from_rgb(0x25, 0x63, 0xeb),
    Color32::from_rgb(0xdc, 0x26, 0x26),
    Color32::from_rgb(0x16, 0xa3, 0x4a),
    Color32::from_rgb(0x93, 0x33, 0xea),
    Color32::from_rgb(0xea, 0x58, 0x0c),
];

/// Draw a plot into the available width.
pub fn show(ui: &mut Ui, plot: &Plot, fg: Color32, muted: Color32, bg: Color32) {
    let [x0, y1, x1, y0] = plot.bbox;
    let w = ui.available_width().clamp(160.0, 560.0);
    let aspect = ((y1 - y0) / (x1 - x0)).clamp(0.4, 1.2) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(w, (w * aspect).clamp(140.0, 360.0)), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, bg);
    let to_screen = |p: [f64; 2]| -> Pos2 {
        let fx = ((p[0] - x0) / (x1 - x0)) as f32;
        let fy = ((y1 - p[1]) / (y1 - y0)) as f32;
        pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height())
    };
    let font = egui::FontId::proportional(11.0);
    if plot.grid || plot.axis {
        let step = |span: f64| {
            let raw = span / 8.0;
            let mag = 10f64.powf(raw.log10().floor());
            [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(raw)
        };
        let (sx, sy) = (step(x1 - x0), step(y1 - y0));
        let grid_stroke = Stroke::new(0.5, muted.gamma_multiply(0.35));
        let mut gx = (x0 / sx).ceil() * sx;
        let mut n = 0;
        while gx <= x1 && n < 100 {
            if plot.grid {
                painter.line_segment([to_screen([gx, y0]), to_screen([gx, y1])], grid_stroke);
            }
            if plot.axis && gx.abs() > sx / 2.0 && gx > x0 + sx / 4.0 && gx < x1 - sx / 4.0 && (y0..=y1).contains(&0.0) {
                let p = to_screen([gx, 0.0]);
                painter.line_segment([p - vec2(0.0, 3.0), p + vec2(0.0, 3.0)], Stroke::new(1.0, muted));
                painter.text(p + vec2(0.0, 4.0), egui::Align2::CENTER_TOP, fmt_tick(gx), font.clone(), muted);
            }
            gx += sx;
            n += 1;
        }
        let mut gy = (y0 / sy).ceil() * sy;
        n = 0;
        while gy <= y1 && n < 100 {
            if plot.grid {
                painter.line_segment([to_screen([x0, gy]), to_screen([x1, gy])], grid_stroke);
            }
            if plot.axis && gy.abs() > sy / 2.0 && gy > y0 + sy / 4.0 && gy < y1 - sy / 4.0 && (x0..=x1).contains(&0.0) {
                let p = to_screen([0.0, gy]);
                painter.line_segment([p - vec2(3.0, 0.0), p + vec2(3.0, 0.0)], Stroke::new(1.0, muted));
                painter.text(p + vec2(-5.0, 0.0), egui::Align2::RIGHT_CENTER, fmt_tick(gy), font.clone(), muted);
            }
            gy += sy;
            n += 1;
        }
        if plot.axis {
            let axis = Stroke::new(1.2, muted);
            if (y0..=y1).contains(&0.0) {
                painter.line_segment([to_screen([x0, 0.0]), to_screen([x1, 0.0])], axis);
                painter.text(to_screen([x1, 0.0]) + vec2(-4.0, -4.0), egui::Align2::RIGHT_BOTTOM, plot.x_label.as_deref().unwrap_or("x"), font.clone(), muted);
            }
            if (x0..=x1).contains(&0.0) {
                painter.line_segment([to_screen([0.0, y0]), to_screen([0.0, y1])], axis);
                painter.text(to_screen([0.0, y1]) + vec2(6.0, 4.0), egui::Align2::LEFT_TOP, plot.y_label.as_deref().unwrap_or("y"), font.clone(), muted);
            }
        }
    }
    let mut next = 0usize;
    let mut pick = |c: Option<Color32>| {
        c.unwrap_or_else(|| {
            let col = PALETTE.get(next % PALETTE.len()).copied().unwrap_or(fg);
            next += 1;
            col
        })
    };
    let limit = ((x1 - x0).abs() + (y1 - y0).abs()) * 50.0;
    for s in &plot.shapes {
        match s {
            Shape::Path { points, color, dashed, fill } => {
                let col = pick(*color);
                if *fill {
                    let pts: Vec<Pos2> = points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| to_screen(*p)).collect();
                    if pts.len() >= 3 {
                        painter.add(egui::Shape::convex_polygon(pts.clone(), col.gamma_multiply(0.25), Stroke::NONE));
                        painter.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
                    }
                    continue;
                }
                // Split at NaN and at jumps far outside the view.
                let mut run: Vec<Pos2> = Vec::new();
                let flush = |run: &mut Vec<Pos2>| {
                    if run.len() >= 2 {
                        let line = std::mem::take(run);
                        if *dashed {
                            painter.extend(egui::Shape::dashed_line(&line, Stroke::new(2.0, col), 6.0, 4.0));
                        } else {
                            painter.add(egui::Shape::line(line, Stroke::new(2.0, col)));
                        }
                    }
                    run.clear();
                };
                for p in points {
                    if !(p[0].is_finite() && p[1].is_finite()) || p[1].abs() > limit {
                        flush(&mut run);
                        continue;
                    }
                    let q = to_screen(*p);
                    if run.last().is_some_and(|l| (l.y - q.y).abs() > rect.height() * 3.0) {
                        flush(&mut run);
                    }
                    run.push(q);
                }
                flush(&mut run);
            }
            Shape::Point { at, name, color } => {
                let col = color.unwrap_or(Color32::from_rgb(0xdc, 0x26, 0x26));
                let p = to_screen(*at);
                painter.circle_filled(p, 3.5, col);
                if let Some(n) = name {
                    painter.text(p + vec2(5.0, -5.0), egui::Align2::LEFT_BOTTOM, n, font.clone(), fg);
                }
            }
            Shape::Circle { center, radius, color } => {
                let col = pick(*color);
                let c = to_screen(*center);
                let r = (*radius * rect.width() as f64 / (x1 - x0)) as f32;
                if r.is_finite() && r < 10_000.0 {
                    painter.circle_stroke(c, r, Stroke::new(2.0, col));
                }
            }
            Shape::Text { at, text } => {
                painter.text(to_screen(*at), egui::Align2::LEFT_CENTER, text, font.clone(), fg);
            }
            Shape::Bar { x, width, height, color } => {
                let col = color.unwrap_or(PALETTE[0]);
                let r = Rect::from_two_pos(to_screen([x - width / 2.0, 0.0]), to_screen([x + width / 2.0, *height]));
                painter.rect_filled(r, 0.0, col.gamma_multiply(0.85));
            }
        }
    }
}

fn fmt_tick(v: f64) -> String {
    let s =
        if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string() };
    s.replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str, x: f64) -> f64 {
        eval(&parse_expr(s).unwrap(), &HashMap::from([("x".to_string(), x), ("a".to_string(), 2.0)]))
    }

    #[test]
    fn expressions() {
        assert_eq!(ev("2*x^2 - 3*x + 1", 2.0), 3.0);
        assert_eq!(ev("-x^2", 3.0), -9.0);
        assert_eq!(ev("2^3^2", 0.0), 512.0);
        assert!((ev("a*sin(PI/2)", 0.0) - 2.0).abs() < 1e-12);
        assert_eq!(ev("x < 0 ? -1 : 1", -5.0), -1.0);
        assert_eq!(ev("max(1, x, 3)", 7.0), 7.0);
        assert!(ev("unknown(x)", 1.0).is_nan());
        assert!(ev("y + 1", 1.0).is_nan());
        assert!(parse_expr("2x").is_none(), "JessieCode needs *");
        assert!(parse_expr("alert(1); x").is_none());
        assert!(parse_expr("((((((").is_none());
        assert!(parse_expr(&"(".repeat(5000)).is_none(), "deep nesting is refused, not a stack overflow");
        assert!(parse_expr(&"-".repeat(500)).is_none());
    }

    #[test]
    fn splits_answers_at_plot_fences() {
        let text = "Hier ist der Graph:\n\n```jsxgraph-json\n{\"board\":{}}\n```\n\nFertig.";
        assert_eq!(segments(text), vec![Segment::Markdown("Hier ist der Graph:\n\n"), Segment::Plot("{\"board\":{}}\n"), Segment::Markdown("\n\nFertig.")]);
        // Still streaming: no closing fence yet, nothing to draw.
        let partial = "Graph:\n```jsxgraph-json\n{\"board\":";
        assert_eq!(segments(partial), vec![Segment::Markdown(partial)]);
        assert_eq!(segments("```jsxgraph-jsonx\n1\n```"), vec![Segment::Markdown("```jsxgraph-jsonx\n1\n```")]);
        assert_eq!(segments(""), vec![Segment::Markdown("")]);
    }

    #[test]
    fn reads_the_example_specs_from_ais_chat() {
        // The examples in AIS.chat's JSXGRAPH_SYSTEM_PROMPT.
        let p = parse_plot(r##"{"board": {"boundingBox": [-7, 4, 7, -4]}, "elements": [["slider", [0, 1, 3], {"name": "a"}], ["functiongraph", ["a*sin(x)"], {"name": "f"}], ["integral", [[0, 3], "f"], {"fillColor": "#3b82f6", "fillOpacity": 0.4}]]}"##).unwrap();
        assert_eq!(p.sliders, vec![("a".to_string(), 1.0)]);
        assert_eq!(p.shapes.len(), 2);
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        let Shape::Path { points, .. } = &p.shapes[0] else { panic!() };
        assert_eq!(points.len(), SAMPLES + 1);
        assert!((points[SAMPLES / 2][1]).abs() < 1e-9, "sin(0) = 0");

        let p = parse_plot(r##"{"board": {"boundingBox": [-2, 8, 8, -2]}, "elements": [["point", [0, 0], {"name": "A"}], ["point", [6, 0], {"name": "B"}], ["point", [6, 5], {"name": "C"}], ["point", [1, 5], {"name": "D"}], ["polygon", ["A", "B", "C", "D"], {"name": "Q"}], ["measurement", [-1, 7, ["Area", "Q"]], {"prefix": "Area: ", "digits": 2}]]}"##).unwrap();
        assert_eq!(p.shapes.len(), 5);
        assert!(matches!(&p.shapes[4], Shape::Path { points, fill: true, .. } if points.len() == 5));

        let p = parse_plot(r##"{"board": {"boundingBox": [-0.5, 6, 4.5, -1.5]}, "elements": [["chart", [[3, 5, 2]], {"chartStyle": "bar", "width": 0.6, "labels": ["3", "5", "2"]}], ["text", [1, -0.6, "Mon"]], ["text", [2, -0.6, "Tue"]], ["text", [3, -0.6, "Wed"]]]}"##).unwrap();
        assert_eq!(p.shapes.iter().filter(|s| matches!(s, Shape::Bar { .. })).count(), 3);
        assert_eq!(p.shapes.iter().filter(|s| matches!(s, Shape::Text { .. })).count(), 3);

        let p = parse_plot(r##"{"board": {"boundingBox": [-8, 8, 8, -8], "axis": false}, "elements": [["view3d", [[-4, -3], [8, 8], [[-5, 5], [-5, 5], [-5, 5]]]], ["functiongraph3d", ["x^2-y^2", [-3, 3], [-3, 3]]]]}"##).unwrap();
        assert_eq!(p.skipped, vec!["view3d".to_string(), "functiongraph3d".to_string()], "3D is named, not drawn");
    }

    #[test]
    fn hostile_specs_fail_cleanly() {
        assert!(parse_plot("nicht json").is_err());
        assert!(parse_plot(r##"{"board": {"boundingBox": [1, 0, 0, 1]}, "elements": []}"##).is_err(), "inverted box");
        assert!(parse_plot(r##"{"board": {"boundingBox": [0, 1, 1]}, "elements": []}"##).is_err());
        assert!(parse_plot(r##"{"elements": []}"##).is_err());
        let p = parse_plot(r##"{"board": {"boundingBox": [-1, 1, 1, -1]}, "elements": [[], [5], ["point"], ["circle", ["nope", -1]], ["functiongraph", [{"x": 1}]], ["curve", ["x", "x", 5, 1]]]}"##).unwrap();
        assert!(p.shapes.is_empty());
        let many = format!(r#"{{"board": {{"boundingBox": [-1, 1, 1, -1]}}, "elements": [{}]}}"#, vec![r#"["point", [0, 0]]"#; 5000].join(","));
        assert_eq!(parse_plot(&many).unwrap().shapes.len(), MAX_ELEMENTS);
    }
}
