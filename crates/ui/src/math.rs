//! Formulas (`$…$`, `$$…$$`, `\(…\)`, `\[…\]`): LaTeX math read by craft-math into one equation
//! tree, laid out here as boxes (TeX's display, text and script styles, simplified) and drawn
//! with egui's painter: glyphs as text, fraction bars, radicals, grown delimiters, braces and
//! accents as lines. No browser, no images, no fonts of its own.
//!
//! Hostile input: the source length, the nesting depth and the number of drawn pieces are capped;
//! what does not fit is cut, never a panic. Glyphs the loaded fonts lack fall back to ASCII where
//! one exists (`−` → `-`).

use std::sync::Arc;

use craft_math::math::{self as m, ColJc, FracKind, LimLoc, MClass, MScr, MSty, ScriptKind};
use craft_math::{MNode, MRun};
use egui::{Color32, Galley, Painter, Pos2, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

/// Longer formulas are shown as source.
pub const MAX_TEX: usize = 4000;
const MAX_ITEMS: usize = 3000;
const MAX_NEST: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Display,
    Text,
    Script,
    ScriptScript,
}

impl Level {
    fn scale(self) -> f32 {
        match self {
            Level::Display | Level::Text => 1.0,
            Level::Script => 0.72,
            Level::ScriptScript => 0.55,
        }
    }
    fn sup(self) -> Level {
        match self {
            Level::Display | Level::Text => Level::Script,
            _ => Level::ScriptScript,
        }
    }
    fn frac(self) -> Level {
        match self {
            Level::Display => Level::Text,
            Level::Text => Level::Script,
            _ => Level::ScriptScript,
        }
    }
    fn spaced(self) -> bool {
        matches!(self, Level::Display | Level::Text)
    }
}

/// A drawn piece, relative to the box's left edge and baseline (y grows downwards).
#[derive(Clone)]
enum Item {
    Text { at: Vec2, galley: Arc<Galley>, baseline: f32 },
    Path { points: Vec<Vec2>, width: f32 },
    Dot { at: Vec2, r: f32 },
}

#[derive(Clone, Default)]
struct MBox {
    w: f32,
    asc: f32,
    desc: f32,
    items: Vec<Item>,
}

impl MBox {
    fn space(w: f32) -> MBox {
        MBox { w, ..Default::default() }
    }
    fn h(&self) -> f32 {
        self.asc + self.desc
    }
    /// Put `other` into this box with its left edge at `dx` and its baseline at `dy`.
    fn put(&mut self, other: MBox, dx: f32, dy: f32) {
        let d = vec2(dx, dy);
        self.asc = self.asc.max(other.asc - dy);
        self.desc = self.desc.max(other.desc + dy);
        self.w = self.w.max(dx + other.w);
        for it in other.items {
            self.items.push(match it {
                Item::Text { at, galley, baseline } => Item::Text { at: at + d, galley, baseline },
                Item::Path { points, width } => Item::Path { points: points.into_iter().map(|p| p + d).collect(), width },
                Item::Dot { at, r } => Item::Dot { at: at + d, r },
            });
        }
    }
    /// Append `other` on the same baseline.
    fn append(&mut self, other: MBox) {
        let x = self.w;
        self.put(other, x, 0.0);
    }
    fn path(&mut self, points: Vec<Vec2>, width: f32) {
        self.items.push(Item::Path { points, width });
    }
}

struct Lx<'a> {
    painter: &'a Painter,
    color: Color32,
    base: f32,
    budget: usize,
    /// Something was cut (depth or size).
    cut: bool,
}

/// A laid-out atom with its spacing class.
struct Atom {
    b: MBox,
    class: MClass,
}

impl Lx<'_> {
    fn size(&self, level: Level) -> f32 {
        self.base * level.scale()
    }

    fn take(&mut self) -> bool {
        if self.budget == 0 {
            self.cut = true;
            return false;
        }
        self.budget -= 1;
        true
    }

    /// The character, or an ASCII stand-in when no loaded font has it.
    fn glyph_text(&self, s: &str, font: &egui::FontId) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_ascii() || self.painter.fonts_mut(|f| f.has_glyph(font, c)) {
                out.push(c);
                continue;
            }
            out.push_str(match c {
                '−' => "-",
                '⋅' | '∙' => "·",
                '∗' => "*",
                '…' | '⋯' => "...",
                '′' => "'",
                '″' => "''",
                '\u{a0}' | '\u{2009}' | '\u{2002}' | '\u{2003}' => " ",
                _ => {
                    out.push(c);
                    continue;
                }
            });
        }
        out
    }

    fn text(&mut self, s: &str, size: f32, italic: bool) -> MBox {
        if s.is_empty() || !self.take() {
            return MBox::default();
        }
        let font = egui::FontId::proportional(size);
        let s = self.glyph_text(s, &font);
        let fmt = egui::TextFormat { font_id: font, color: self.color, italics: italic, ..Default::default() };
        let job = egui::text::LayoutJob::single_section(s.clone(), fmt);
        let galley = self.painter.layout_job(job);
        let baseline = galley.rows.first().and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(size * 0.8);
        let tall = s.chars().any(|c| "()[]{}|/\\∫∑∏".contains(c) || !c.is_ascii());
        let desc = if tall || s.chars().any(|c| "gjpqy,;".contains(c)) { 0.24 * size } else { 0.06 * size };
        let asc = if tall { 0.78 * size } else { 0.72 * size };
        let w = galley.size().x + if italic { 0.04 * size } else { 0.0 };
        MBox { w, asc, desc, items: vec![Item::Text { at: vec2(0.0, 0.0), galley, baseline }] }
    }

    fn list(&mut self, nodes: &[MNode], level: Level, depth: usize) -> MBox {
        self.list_after(nodes, level, depth, None)
    }

    /// A row of nodes with TeX's spacing between atoms; `prev` is the class of what stands before
    /// (aligned columns).
    fn list_after(&mut self, nodes: &[MNode], level: Level, depth: usize, prev: Option<MClass>) -> MBox {
        let mut atoms: Vec<Atom> = Vec::new();
        for n in nodes {
            if self.budget == 0 {
                self.cut = true;
                break;
            }
            self.node(n, level, depth + 1, &mut atoms);
        }
        let s = self.size(level);
        let mut out = MBox { asc: 0.72 * s, desc: 0.06 * s, ..Default::default() };
        let mut last = prev;
        for a in atoms {
            let mut class = a.class;
            // A binary operator with nothing to its left is a sign (−x).
            if class == MClass::Bin && matches!(last, None | Some(MClass::Bin | MClass::Rel | MClass::Open | MClass::Punct | MClass::Op)) {
                class = MClass::Ord;
            }
            if level.spaced() {
                out.w += s * gap(last, class);
            } else if last == Some(MClass::Op) && class == MClass::Ord {
                out.w += s * 0.1;
            }
            out.append(a.b);
            last = Some(class);
        }
        out
    }

    fn node(&mut self, n: &MNode, level: Level, depth: usize, out: &mut Vec<Atom>) {
        let s = self.size(level);
        if depth > MAX_NEST {
            self.cut = true;
            out.push(Atom { b: self.text("…", s, false), class: MClass::Ord });
            return;
        }
        let ord = |b| Atom { b, class: MClass::Ord };
        match n {
            MNode::Run(r) => self.run(r, level, out),
            MNode::Frac { kind, num, den } => {
                let b = self.frac(*kind, num, den, level, depth);
                out.push(Atom { b, class: MClass::Inner });
            }
            MNode::Script { kind, base, sub, sup } => {
                let b = self.script(*kind, base, sub, sup, level, depth);
                // The script keeps the class of a lone operator base (x_i = …, ∑ as a run).
                let class = match base.as_slice() {
                    [MNode::Run(r)] if r.text.chars().count() == 1 => r.text.chars().next().map(m::math_class).unwrap_or(MClass::Ord),
                    _ => MClass::Ord,
                };
                out.push(Atom { b, class });
            }
            MNode::Rad { deg, deg_hide, e } => {
                let b = self.rad(if *deg_hide { &[] } else { deg }, e, level, depth);
                out.push(ord(b));
            }
            MNode::Nary { chr, lim_loc, sub_hide, sup_hide, sub, sup, e, .. } => {
                let b = self.nary(*chr, *lim_loc, if *sub_hide { &[] } else { sub }, if *sup_hide { &[] } else { sup }, level, depth);
                out.push(Atom { b, class: MClass::Op });
                if !e.is_empty() {
                    let eb = self.list_after(e, level, depth, Some(MClass::Op));
                    out.push(Atom { b: eb, class: MClass::Inner });
                }
            }
            MNode::Delim { beg, end, sep, grow, elems, .. } => {
                let b = self.delim(*beg, *end, *sep, *grow, elems, level, depth);
                out.push(Atom { b, class: MClass::Inner });
            }
            MNode::Func { name, e } => {
                let mut b = self.upright(name, level, depth);
                b.w += s * 0.17;
                let eb = self.list(e, level, depth);
                b.append(eb);
                out.push(Atom { b, class: MClass::Op });
            }
            MNode::Lim { upper, e, lim } => {
                let eb = self.upright(e, level, depth);
                let lb = self.list(lim, level.sup(), depth);
                let b = self.stack(eb, if *upper { Some(lb.clone()) } else { None }, if *upper { None } else { Some(lb) }, s);
                out.push(Atom { b, class: MClass::Op });
            }
            MNode::Acc { chr, e } => {
                let b = self.accent(*chr, e, level, depth);
                out.push(ord(b));
            }
            MNode::Bar { top, e } => {
                let mut b = self.list(e, level, depth);
                let t = (0.055 * s).max(1.0);
                let y = if *top { -b.asc - 0.12 * s } else { b.desc + 0.12 * s };
                b.path(vec![vec2(0.0, y), vec2(b.w, y)], t);
                if *top {
                    b.asc += 0.15 * s;
                } else {
                    b.desc += 0.15 * s;
                }
                out.push(ord(b));
            }
            MNode::BorderBox { hide, e, .. } => {
                let inner = self.list(e, level, depth);
                let pad = 0.18 * s;
                let mut b = MBox::default();
                b.put(inner, pad, 0.0);
                b.w += pad;
                b.asc += pad;
                b.desc += pad;
                if !hide.iter().all(|h| *h) {
                    let (w, a, d) = (b.w, b.asc, b.desc);
                    b.path(vec![vec2(0.0, -a), vec2(w, -a), vec2(w, d), vec2(0.0, d), vec2(0.0, -a)], (0.05 * s).max(1.0));
                }
                out.push(ord(b));
            }
            MNode::Boxed { e } => out.push(ord(self.list(e, level, depth))),
            MNode::GroupChr { chr, top, e } => {
                let b = self.group_chr(*chr, *top, e, level, depth);
                out.push(Atom { b, class: MClass::Op });
            }
            MNode::EqArr { rows } => {
                let grid: Vec<Vec<Vec<MNode>>> = rows.iter().map(|r| split_columns(r)).collect();
                let b = self.grid(&grid, &[], level, depth, true);
                out.push(ord(b));
            }
            MNode::Matrix { rows, col_jc } => {
                let b = self.grid(rows, col_jc, level, depth, false);
                out.push(ord(b));
            }
            MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => {
                let mut b = self.list(e, level, depth);
                if !*show {
                    b.items.clear();
                }
                if *zero_wid {
                    b.w = 0.0;
                }
                if *zero_asc {
                    b.asc = 0.0;
                }
                if *zero_desc {
                    b.desc = 0.0;
                }
                out.push(ord(b));
            }
        }
    }

    fn upright(&mut self, nodes: &[MNode], level: Level, depth: usize) -> MBox {
        let plain: Vec<MNode> = nodes
            .iter()
            .map(|n| match n {
                MNode::Run(r) if r.sty.is_none() => MNode::Run(MRun { sty: Some(MSty::Plain), ..r.clone() }),
                other => other.clone(),
            })
            .collect();
        self.list(&plain, level, depth)
    }

    /// A run: letters italic (unless styled), operators as their own atoms for spacing.
    fn run(&mut self, r: &MRun, level: Level, out: &mut Vec<Atom>) {
        let s = self.size(level) * r.size.map(|p| (p / 11.0).clamp(0.5, 2.0)).unwrap_or(1.0);
        if r.nor {
            let b = self.text(&r.text, s, false);
            out.push(Atom { b, class: MClass::Ord });
            return;
        }
        let italic_style = matches!(r.sty, Some(MSty::Italic | MSty::BoldItalic));
        let plain_style = matches!(r.sty, Some(MSty::Plain | MSty::Bold));
        let mut word = String::new();
        let mut word_italic = false;
        let flush = |lx: &mut Self, word: &mut String, italic: bool, out: &mut Vec<Atom>| {
            if !word.is_empty() {
                let b = lx.text(word, s, italic);
                out.push(Atom { b, class: MClass::Ord });
                word.clear();
            }
        };
        for c in r.text.chars().take(2000) {
            let c = if r.scr != MScr::Roman { mapped(self, c, r.scr, s) } else { c };
            let class = m::math_class(c);
            // Spacing commands come as space characters: \, \: \; \quad …
            let space = match c {
                ' ' => Some(0.0),
                '\u{2009}' | '\u{2006}' | '\u{202f}' => Some(0.17),
                '\u{2005}' | '\u{205f}' => Some(0.22),
                '\u{2004}' | '\u{a0}' => Some(0.28),
                '\u{2002}' => Some(0.5),
                '\u{2003}' => Some(1.0),
                '\u{2001}' => Some(2.0),
                _ => None,
            };
            if let Some(em) = space {
                if em > 0.0 {
                    flush(self, &mut word, word_italic, out);
                    out.push(Atom { b: MBox::space(em * s), class: MClass::Ord });
                }
                continue;
            }
            if class != MClass::Ord {
                flush(self, &mut word, word_italic, out);
                let shown = match c {
                    '-' => '−',
                    '*' => '∗',
                    c => c,
                };
                let b = if m::is_nary_char(c) { self.text(&shown.to_string(), s * 1.1, false) } else { self.text(&shown.to_string(), s, false) };
                out.push(Atom { b, class });
                continue;
            }
            let italic = if italic_style {
                true
            } else if plain_style || r.scr != MScr::Roman {
                false
            } else {
                c.is_ascii_alphabetic() || ('α'..='ω').contains(&c) || c == 'ϕ' || c == 'ϵ' || c == 'ϑ'
            };
            if italic != word_italic && !word.is_empty() {
                flush(self, &mut word, word_italic, out);
            }
            word_italic = italic;
            word.push(c);
        }
        flush(self, &mut word, word_italic, out);
    }

    fn frac(&mut self, kind: FracKind, num: &[MNode], den: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        if matches!(kind, FracKind::Linear | FracKind::Skewed) {
            let mut b = self.list(num, level, depth);
            let slash = self.text("/", s, false);
            b.append(slash);
            let d = self.list(den, level, depth);
            b.append(d);
            return b;
        }
        let inner = level.frac();
        let n = self.list(num, inner, depth);
        let d = self.list(den, inner, depth);
        let axis = 0.27 * s;
        let t = (0.055 * s).max(1.0);
        let gap = if level == Level::Display { 0.2 * s } else { 0.12 * s };
        let pad = 0.12 * s;
        let w = n.w.max(d.w) + pad * 2.0;
        let mut b = MBox { w, ..Default::default() };
        let nw = n.w;
        let ny = -(axis + t / 2.0 + gap + n.desc);
        b.put(n, (w - nw) / 2.0, ny);
        let dw = d.w;
        let dy = -axis + t / 2.0 + gap + d.asc;
        b.put(d, (w - dw) / 2.0, dy);
        if kind == FracKind::Bar {
            b.path(vec![vec2(pad * 0.5, -axis), vec2(w - pad * 0.5, -axis)], t);
        }
        b
    }

    fn script(&mut self, kind: ScriptKind, base: &[MNode], sub: &[MNode], sup: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let base_b = self.list(base, level, depth);
        let small = level.sup();
        let sub_b = (!sub.is_empty() && matches!(kind, ScriptKind::Sub | ScriptKind::SubSup | ScriptKind::Pre)).then(|| self.list(sub, small, depth));
        let sup_b = (!sup.is_empty() && matches!(kind, ScriptKind::Sup | ScriptKind::SubSup | ScriptKind::Pre)).then(|| self.list(sup, small, depth));
        let mut u = (base_b.asc - 0.3 * s).max(0.38 * s);
        let mut v = (base_b.desc + 0.02 * s).max(0.2 * s);
        if let (Some(a), Some(b)) = (&sup_b, &sub_b) {
            let clash = (-u + a.desc) - (v - b.asc) + 0.12 * s;
            if clash > 0.0 {
                v += clash / 2.0;
                u += clash / 2.0;
            }
        }
        let scripts_w = sub_b.as_ref().map(|b| b.w).unwrap_or(0.0).max(sup_b.as_ref().map(|b| b.w).unwrap_or(0.0));
        let mut out = MBox::default();
        let pre = kind == ScriptKind::Pre;
        let bw = base_b.w;
        let script_x = if pre { 0.0 } else { bw + 0.03 * s };
        out.put(base_b, if pre { scripts_w + 0.05 * s } else { 0.0 }, 0.0);
        if let Some(b) = sup_b {
            let x = if pre { scripts_w - b.w } else { script_x + 0.04 * s };
            out.put(b, x, -u);
        }
        if let Some(b) = sub_b {
            let x = if pre { scripts_w - b.w } else { script_x };
            out.put(b, x, v);
        }
        out.w += 0.04 * s;
        out
    }

    fn rad(&mut self, deg: &[MNode], e: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let inner = self.list(e, level, depth);
        let t = (0.055 * s).max(1.0);
        let gap = 0.12 * s;
        let top = inner.asc + gap + t;
        let bottom = inner.desc;
        let h = top + bottom;
        let sw = 0.6 * s;
        let deg_b = (!deg.is_empty()).then(|| self.list(deg, Level::ScriptScript, depth));
        let shift = deg_b.as_ref().map(|d| (d.w - sw * 0.45).max(0.0)).unwrap_or(0.0);
        let mut b = MBox::default();
        let iw = inner.w;
        b.put(inner, shift + sw + 0.06 * s, 0.0);
        let x = shift;
        b.path(
            vec![
                vec2(x, -top + h * 0.58),
                vec2(x + sw * 0.22, -top + h * 0.5),
                vec2(x + sw * 0.55, bottom),
                vec2(x + sw, -top + t / 2.0),
                vec2(x + sw + iw + 0.14 * s, -top + t / 2.0),
            ],
            t,
        );
        if let Some(d) = deg_b {
            let dy = -top + h * 0.45;
            b.put(d, 0.0, dy);
        }
        b.asc = b.asc.max(top + t * 0.5);
        b.w += 0.08 * s;
        b
    }

    fn nary(&mut self, chr: char, lim_loc: Option<LimLoc>, sub: &[MNode], sup: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let integral = m::is_integral(chr);
        let big = if level == Level::Display { if integral { 2.1 } else { 1.9 } } else { 1.3 };
        let op_size = s * big;
        let op = self.text(&chr.to_string(), op_size, false);
        let axis = 0.27 * s;
        // Centre the glyph's ink on the math axis.
        let (top, bottom) = op
            .items
            .first()
            .and_then(|it| match it {
                Item::Text { galley, baseline, .. } => {
                    let r = galley.mesh_bounds;
                    (r.is_finite() && r.height() > 0.0).then(|| (r.top() - baseline, r.bottom() - baseline))
                }
                _ => None,
            })
            .unwrap_or((-0.72 * op_size, 0.0));
        let dy = -axis - (top + bottom) / 2.0;
        let ow = op.w;
        let mut sym = MBox::default();
        sym.put(op, 0.0, dy);
        sym.w = ow;
        sym.asc = -(top + dy) + 0.02 * s;
        sym.desc = (bottom + dy).max(0.0) + 0.02 * s;
        let under_over = match lim_loc {
            Some(LimLoc::UndOvr) => true,
            Some(LimLoc::SubSup) => false,
            None => !integral && level == Level::Display,
        };
        let sub_b = (!sub.is_empty()).then(|| self.list(sub, level.sup(), depth));
        let sup_b = (!sup.is_empty()).then(|| self.list(sup, level.sup(), depth));
        if under_over {
            return self.stack(sym, sup_b, sub_b, s);
        }
        let mut b = MBox::default();
        let sa = sym.asc;
        let sd = sym.desc;
        b.append(sym);
        let x = b.w + if integral { 0.0 } else { 0.05 * s };
        if let Some(u) = sup_b {
            let y = -sa + u.asc * 0.7;
            b.put(u, x + if integral { 0.15 * s } else { 0.0 }, y);
        }
        if let Some(d) = sub_b {
            let y = sd + d.asc * 0.3;
            b.put(d, x - if integral { 0.15 * s } else { 0.0 }, y);
        }
        b.w += 0.06 * s;
        b
    }

    /// `base` with `over` above and `under` below, centred.
    fn stack(&mut self, base: MBox, over: Option<MBox>, under: Option<MBox>, s: f32) -> MBox {
        let gap = 0.12 * s;
        let w = base.w.max(over.as_ref().map(|b| b.w).unwrap_or(0.0)).max(under.as_ref().map(|b| b.w).unwrap_or(0.0));
        let (ba, bd, bw) = (base.asc, base.desc, base.w);
        let mut out = MBox::default();
        out.put(base, (w - bw) / 2.0, 0.0);
        if let Some(o) = over {
            let y = -(ba + gap + o.desc);
            let ow = o.w;
            out.put(o, (w - ow) / 2.0, y);
        }
        if let Some(u) = under {
            let y = bd + gap + u.asc;
            let uw = u.w;
            out.put(u, (w - uw) / 2.0, y);
        }
        out.w = w;
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn delim(&mut self, beg: Option<char>, end: Option<char>, sep: Option<char>, grow: bool, elems: &[Vec<MNode>], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let mut content = MBox { asc: 0.72 * s, desc: 0.06 * s, ..Default::default() };
        for (i, e) in elems.iter().enumerate() {
            if i > 0 {
                let c = sep.unwrap_or('|');
                let sb = self.text(&c.to_string(), s, false);
                content.append(sb);
                content.w += 0.15 * s;
            }
            let b = self.list(e, level, depth);
            content.append(b);
        }
        let axis = 0.27 * s;
        let tall = grow && (content.asc > 0.9 * s || content.desc > 0.32 * s);
        let extent = (content.asc - axis).max(content.desc + axis) + 0.08 * s;
        let mut out = MBox::default();
        if let Some(c) = beg {
            let d = self.delimiter(c, tall, -axis - extent, -axis + extent, s);
            out.append(d);
        }
        out.w += 0.03 * s;
        out.append(content);
        out.w += 0.03 * s;
        if let Some(c) = end {
            let d = self.delimiter(c, tall, -axis - extent, -axis + extent, s);
            out.append(d);
        }
        out
    }

    /// One delimiter, drawn as lines when it has to grow beyond a glyph.
    fn delimiter(&mut self, c: char, tall: bool, top: f32, bottom: f32, s: f32) -> MBox {
        if !tall || !self.take() {
            return self.text(&c.to_string(), s, false);
        }
        let h = bottom - top;
        let mid = (top + bottom) / 2.0;
        let dw = (0.32 * s + h * 0.04).min(0.7 * s);
        let t = (0.06 * s).max(1.0);
        let curve = |x0: f32, xm: f32| -> Vec<Vec2> {
            (0..=16)
                .map(|i| {
                    let u = i as f32 / 16.0;
                    let x = (1.0 - u) * (1.0 - u) * x0 + 2.0 * u * (1.0 - u) * xm + u * u * x0;
                    vec2(x, top + h * u)
                })
                .collect()
        };
        let pts: Vec<Vec<Vec2>> = match c {
            '(' => vec![curve(dw * 0.85, -dw * 0.25)],
            ')' => vec![curve(dw * 0.15, dw * 1.25)],
            '[' => vec![vec![vec2(dw * 0.8, top), vec2(dw * 0.3, top), vec2(dw * 0.3, bottom), vec2(dw * 0.8, bottom)]],
            ']' => vec![vec![vec2(dw * 0.2, top), vec2(dw * 0.7, top), vec2(dw * 0.7, bottom), vec2(dw * 0.2, bottom)]],
            '{' => vec![brace(top, bottom, mid, dw, false)],
            '}' => vec![brace(top, bottom, mid, dw, true)],
            '|' | '∣' => vec![vec![vec2(dw * 0.5, top), vec2(dw * 0.5, bottom)]],
            '‖' | '∥' => vec![vec![vec2(dw * 0.35, top), vec2(dw * 0.35, bottom)], vec![vec2(dw * 0.7, top), vec2(dw * 0.7, bottom)]],
            '⟨' | '〈' => vec![vec![vec2(dw * 0.8, top), vec2(dw * 0.2, mid), vec2(dw * 0.8, bottom)]],
            '⟩' | '〉' => vec![vec![vec2(dw * 0.2, top), vec2(dw * 0.8, mid), vec2(dw * 0.2, bottom)]],
            '⌈' => vec![vec![vec2(dw * 0.8, top), vec2(dw * 0.3, top), vec2(dw * 0.3, bottom)]],
            '⌉' => vec![vec![vec2(dw * 0.2, top), vec2(dw * 0.7, top), vec2(dw * 0.7, bottom)]],
            '⌊' => vec![vec![vec2(dw * 0.3, top), vec2(dw * 0.3, bottom), vec2(dw * 0.8, bottom)]],
            '⌋' => vec![vec![vec2(dw * 0.7, top), vec2(dw * 0.7, bottom), vec2(dw * 0.2, bottom)]],
            _ => return self.text(&c.to_string(), s, false),
        };
        let mut b = MBox { w: dw, asc: -top, desc: bottom, items: Vec::new() };
        for p in pts {
            b.path(p, t);
        }
        b
    }

    fn accent(&mut self, chr: char, e: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let mut b = self.list(e, level, depth);
        let t = (0.05 * s).max(1.0);
        let y = -b.asc - 0.14 * s;
        let w = b.w;
        let cx = w / 2.0 + 0.04 * s;
        let half = (w * 0.5).clamp(0.18 * s, 0.6 * s);
        match chr {
            '\u{20d7}' | '\u{20d1}' | '→' => {
                let half = (w * 0.5).max(0.25 * s);
                b.path(vec![vec2(cx - half, y), vec2(cx + half, y)], t);
                b.path(vec![vec2(cx + half - 0.14 * s, y - 0.1 * s), vec2(cx + half, y), vec2(cx + half - 0.14 * s, y + 0.1 * s)], t);
            }
            '\u{302}' | '^' | 'ˆ' => {
                let hw = half.min(0.3 * s);
                b.path(vec![vec2(cx - hw, y + 0.05 * s), vec2(cx, y - 0.12 * s), vec2(cx + hw, y + 0.05 * s)], t);
            }
            '\u{30c}' | 'ˇ' => {
                let hw = half.min(0.3 * s);
                b.path(vec![vec2(cx - hw, y - 0.12 * s), vec2(cx, y + 0.05 * s), vec2(cx + hw, y - 0.12 * s)], t);
            }
            '\u{304}' | '\u{305}' | '¯' | '‾' => b.path(vec![vec2(0.05 * s, y), vec2(w, y)], t),
            '\u{303}' | '~' | '˜' => {
                let hw = half.min(0.35 * s);
                let pts = (0..=12)
                    .map(|i| {
                        let u = i as f32 / 12.0;
                        vec2(cx - hw + 2.0 * hw * u, y - (u * std::f32::consts::TAU).sin() * 0.06 * s)
                    })
                    .collect();
                b.path(pts, t);
            }
            '\u{307}' | '˙' => b.items.push(Item::Dot { at: vec2(cx, y), r: 0.06 * s }),
            '\u{308}' | '¨' => {
                b.items.push(Item::Dot { at: vec2(cx - 0.12 * s, y), r: 0.06 * s });
                b.items.push(Item::Dot { at: vec2(cx + 0.12 * s, y), r: 0.06 * s });
            }
            c => {
                let a = self.text(&spacing_accent(c).to_string(), s * 0.8, false);
                let aw = a.w;
                b.put(a, cx - aw / 2.0, y + 0.3 * s);
            }
        }
        b.asc = b.asc.max(-y + 0.15 * s);
        b
    }

    fn group_chr(&mut self, chr: char, top: bool, e: &[MNode], level: Level, depth: usize) -> MBox {
        let s = self.size(level);
        let mut b = self.list(e, level, depth);
        let t = (0.05 * s).max(1.0);
        let w = b.w;
        let y = if top { -b.asc - 0.12 * s } else { b.desc + 0.12 * s };
        let h = 0.22 * s * if top { -1.0 } else { 1.0 };
        match chr {
            '⏟' | '⏞' | '︸' | '︷' => {
                let m = w / 2.0;
                b.path(vec![vec2(0.0, y), vec2(0.06 * s, y + h * 0.6), vec2(m - 0.08 * s, y + h * 0.6), vec2(m, y + h)], t);
                b.path(vec![vec2(w, y), vec2(w - 0.06 * s, y + h * 0.6), vec2(m + 0.08 * s, y + h * 0.6), vec2(m, y + h)], t);
            }
            '→' | '←' | '↔' => {
                b.path(vec![vec2(0.0, y), vec2(w, y)], t);
                if chr != '←' {
                    b.path(vec![vec2(w - 0.14 * s, y - 0.1 * s), vec2(w, y), vec2(w - 0.14 * s, y + 0.1 * s)], t);
                }
                if chr != '→' {
                    b.path(vec![vec2(0.14 * s, y - 0.1 * s), vec2(0.0, y), vec2(0.14 * s, y + 0.1 * s)], t);
                }
            }
            c => {
                let g = self.text(&c.to_string(), s, false);
                let gw = g.w;
                b.put(g, (w - gw) / 2.0, y + h);
            }
        }
        if top {
            b.asc = b.asc.max(-(y + h) + 0.05 * s);
        } else {
            b.desc = b.desc.max(y + h + 0.05 * s);
        }
        b
    }

    /// Rows of cells: matrices (centred columns) and aligned equations (`&` alternates right and
    /// left alignment).
    fn grid(&mut self, rows: &[Vec<Vec<MNode>>], col_jc: &[ColJc], level: Level, depth: usize, aligned: bool) -> MBox {
        let s = self.size(level);
        let cell_level = if level == Level::Display { Level::Text } else { level };
        let mut cells: Vec<Vec<MBox>> = Vec::new();
        for r in rows.iter().take(m::MAX_MATRIX_ROWS.min(100)) {
            let mut row = Vec::new();
            for (ci, c) in r.iter().take(m::MAX_MATRIX_COLS.min(20)).enumerate() {
                let prev = (aligned && ci % 2 == 1).then_some(MClass::Ord);
                row.push(self.list_after(c, cell_level, depth, prev));
            }
            cells.push(row);
        }
        let ncols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![0.0f32; ncols];
        for r in &cells {
            for (ci, c) in r.iter().enumerate() {
                if let Some(w) = widths.get_mut(ci) {
                    *w = w.max(c.w);
                }
            }
        }
        let col_gap = |ci: usize| -> f32 {
            if !aligned {
                0.9 * s
            } else if ci % 2 == 1 {
                1.2 * s
            } else {
                0.0
            }
        };
        let row_gap = 0.28 * s;
        let mut out = MBox::default();
        let mut y = 0.0;
        let mut first = true;
        let mut total_top = 0.0;
        for r in cells {
            let asc = r.iter().map(|c| c.asc).fold(0.72 * s, f32::max);
            let desc = r.iter().map(|c| c.desc).fold(0.06 * s, f32::max);
            if first {
                total_top = asc;
                first = false;
            } else {
                y += asc + row_gap;
            }
            let mut x = 0.0;
            for (ci, c) in r.into_iter().enumerate() {
                let cw = widths.get(ci).copied().unwrap_or(0.0);
                let jc = if aligned { if ci % 2 == 0 { ColJc::Right } else { ColJc::Left } } else { col_jc.get(ci).copied().unwrap_or(ColJc::Center) };
                let dx = match jc {
                    ColJc::Left => 0.0,
                    ColJc::Right => cw - c.w,
                    ColJc::Center => (cw - c.w) / 2.0,
                };
                out.put(c, x + dx, y);
                x += cw + col_gap(ci);
            }
            y += desc;
        }
        // Centre the block on the math axis.
        let axis = 0.27 * s;
        let height = total_top + y;
        let shift = -axis - height / 2.0 + total_top;
        let mut centred = MBox::default();
        let w = out.w;
        centred.put(out, 0.1 * s, shift);
        centred.w = w + 0.2 * s;
        centred
    }
}

fn brace(top: f32, bottom: f32, mid: f32, dw: f32, right: bool) -> Vec<Vec2> {
    let h = bottom - top;
    let f = |x: f32| if right { dw - x } else { x };
    vec![
        vec2(f(dw * 0.9), top),
        vec2(f(dw * 0.55), top + h * 0.04),
        vec2(f(dw * 0.5), top + h * 0.1),
        vec2(f(dw * 0.5), mid - h * 0.08),
        vec2(f(dw * 0.15), mid),
        vec2(f(dw * 0.5), mid + h * 0.08),
        vec2(f(dw * 0.5), bottom - h * 0.1),
        vec2(f(dw * 0.55), bottom - h * 0.04),
        vec2(f(dw * 0.9), bottom),
    ]
}

fn spacing_accent(c: char) -> char {
    match c {
        '\u{301}' => '´',
        '\u{300}' => '`',
        '\u{306}' => '˘',
        '\u{30a}' => '˚',
        c => c,
    }
}

fn mapped(lx: &Lx, c: char, scr: MScr, size: f32) -> char {
    let m = m::math_alnum(c, false, false, scr);
    if m == c || lx.painter.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(size), m)) { m } else { c }
}

/// TeX's spacing between atoms, in em.
fn gap(prev: Option<MClass>, cur: MClass) -> f32 {
    use MClass::*;
    let Some(p) = prev else { return 0.0 };
    match (p, cur) {
        (Bin, _) | (_, Bin) => 0.22,
        (Rel, Rel) => 0.0,
        (Rel, Close | Punct) => 0.0,
        (Open, Rel) => 0.0,
        (Rel, _) | (_, Rel) => 0.28,
        (Punct, _) => 0.17,
        (Op, Ord | Op | Inner) | (Ord | Close, Op) => 0.17,
        (Inner, Ord | Inner) | (Ord, Inner) => 0.0,
        _ => 0.0,
    }
}

/// Split an aligned row at its `&` signs.
fn split_columns(row: &[MNode]) -> Vec<Vec<MNode>> {
    let mut cols: Vec<Vec<MNode>> = vec![Vec::new()];
    for n in row {
        match n {
            MNode::Run(r) if r.text.contains('&') && !r.nor => {
                for (i, piece) in r.text.split('&').enumerate() {
                    if i > 0 {
                        cols.push(Vec::new());
                    }
                    if !piece.is_empty()
                        && let Some(c) = cols.last_mut()
                    {
                        c.push(MNode::Run(MRun { text: piece.to_string(), ..r.clone() }));
                    }
                }
            }
            other => {
                if let Some(c) = cols.last_mut() {
                    c.push(other.clone());
                }
            }
        }
    }
    cols
}

/// A laid-out formula, ready to paint.
pub struct Formula {
    b: MBox,
    /// Pieces were left out (too deep or too large).
    pub cut: bool,
}

impl Formula {
    pub fn size(&self) -> Vec2 {
        vec2(self.b.w, self.b.h())
    }
    pub fn ascent(&self) -> f32 {
        self.b.asc
    }

    /// Paint with the baseline's left end at `origin`.
    pub fn paint(&self, painter: &Painter, origin: Pos2, color: Color32) {
        let o = origin.to_vec2();
        for it in &self.b.items {
            match it {
                Item::Text { at, galley, baseline } => {
                    painter.galley(pos2(o.x + at.x, o.y + at.y - baseline), galley.clone(), color);
                }
                Item::Path { points, width } => {
                    let pts: Vec<Pos2> = points.iter().map(|p| (*p + o).to_pos2()).collect();
                    painter.add(egui::Shape::line(pts, Stroke::new(*width, color)));
                }
                Item::Dot { at, r } => {
                    painter.circle_filled((*at + o).to_pos2(), *r, color);
                }
            }
        }
    }
}

/// Lay out LaTeX math at `size` points. `None` when the source is empty or too long.
pub fn layout(painter: &Painter, tex: &str, display: bool, size: f32, color: Color32) -> Option<Formula> {
    let tex = tex.trim();
    if tex.is_empty() || tex.len() > MAX_TEX {
        return None;
    }
    let nodes = craft_math::math_latex::parse_latex(tex);
    if nodes.is_empty() {
        return None;
    }
    let mut lx = Lx { painter, color, base: size, budget: MAX_ITEMS, cut: false };
    let b = lx.list(&nodes, if display { Level::Display } else { Level::Text }, 0);
    Some(Formula { b, cut: lx.cut })
}

/// The body text size of this ui.
fn body_size(ui: &Ui) -> f32 {
    ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size).unwrap_or(14.0)
}

fn source_label(ui: &mut Ui, tex: &str) -> Response {
    let shown: String = tex.chars().take(300).collect();
    ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).wrap())
}

/// A formula inside running text, on the text's baseline. Hover shows the source; the context
/// menu copies it.
pub fn show_inline(ui: &mut Ui, tex: &str, color: Color32) -> Response {
    let size = body_size(ui);
    let Some(f) = layout(ui.painter(), tex, false, size * 1.02, color) else { return source_label(ui, tex) };
    // Line up the formula's baseline with that of a label in the same (centred) row.
    let probe = ui.painter().layout_no_wrap("x".into(), egui::FontId::proportional(size), color);
    let row_h = probe.size().y;
    let label_baseline = probe.rows.first().and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(row_h * 0.8);
    let d0 = label_baseline - row_h / 2.0;
    let half = (f.b.asc - d0).max(f.b.desc + d0).max(row_h / 2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(f.b.w + 2.0, half * 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        f.paint(ui.painter(), pos2(rect.left() + 1.0, rect.center().y + d0), color);
    }
    finish(resp, tex)
}

/// A formula on its own line, centred (scrolls sideways when wider than the column).
pub fn show_display(ui: &mut Ui, tex: &str, color: Color32) -> Response {
    let size = body_size(ui) * 1.1;
    let Some(f) = layout(ui.painter(), tex, true, size, color) else { return source_label(ui, tex) };
    let pad = 0.35 * size;
    let want = vec2(f.b.w, f.b.h() + pad * 2.0);
    let avail = ui.available_width();
    let inner = if want.x > avail {
        egui::ScrollArea::horizontal()
            .id_salt(("formel", tex.len(), tex.chars().take(40).collect::<String>()))
            .auto_shrink([false, true])
            .show(ui, |ui| ui.allocate_exact_size(want, Sense::click()))
            .inner
    } else {
        ui.allocate_exact_size(vec2(avail, want.y), Sense::click())
    };
    let (rect, resp) = inner;
    if ui.is_rect_visible(rect) {
        let x = rect.left() + ((rect.width() - f.b.w) / 2.0).max(0.0);
        f.paint(ui.painter(), pos2(x, rect.top() + pad + f.b.asc), color);
    }
    finish(resp, tex)
}

fn finish(resp: Response, tex: &str) -> Response {
    let resp = resp.on_hover_text(egui::RichText::new(tex.chars().take(500).collect::<String>()).monospace());
    resp.context_menu(|ui| {
        if ui.button("Formel kopieren (LaTeX)").clicked() {
            ui.ctx().copy_text(tex.to_string());
            ui.close();
        }
    });
    resp
}

/// The formula as plain Unicode text (UnicodeMath linear format), for places that only take
/// text, such as tables.
pub fn to_text(tex: &str) -> String {
    if tex.len() > MAX_TEX {
        return tex.to_string();
    }
    craft_math::math::to_linear(&craft_math::math_latex::parse_latex(tex))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_painter<R>(f: impl FnOnce(&Painter) -> R) -> R {
        let ctx = egui::Context::default();
        let mut f = Some(f);
        let mut out = None;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                out = Some(f(ui.painter()));
            }
        });
        frame.textures_delta.clear();
        out.unwrap()
    }

    #[test]
    fn lays_out_school_formulas() {
        with_painter(|p| {
            let c = Color32::WHITE;
            let x = layout(p, "x", false, 14.0, c).unwrap();
            let q = layout(p, r"x = \frac{-b \pm \sqrt{b^2-4ac}}{2a}", true, 14.0, c).unwrap();
            assert!(q.size().x > x.size().x * 5.0);
            assert!(q.size().y > x.size().y * 1.8, "a fraction is taller than a letter");
            assert!(!q.cut);
            let sum = layout(p, r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}", true, 14.0, c).unwrap();
            assert!(sum.ascent() > 14.0);
            for tex in [
                r"\int_0^1 x^2\,dx",
                r"\lim_{x\to 0} \frac{\sin x}{x} = 1",
                r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}",
                r"f(x) = \begin{cases} x & x \ge 0 \\ -x & \text{sonst} \end{cases}",
                r"\begin{aligned} a &= b + c \\ &= d \end{aligned}",
                r"\vec{v} + \overline{AB} + \hat{x} + \tilde y + \dot x",
                r"\left( \frac{a}{b} \right)^2 \cdot \sqrt[3]{8} \neq \mathbb{R}",
                r"\underbrace{a + b}_{n} \quad \boxed{E = mc^2}",
                r"\binom{n}{k} \le \lVert x \rVert",
            ] {
                let f = layout(p, tex, true, 14.0, c).unwrap();
                assert!(f.size().x > 10.0 && f.size().y.is_finite(), "{tex}");
            }
        });
    }

    #[test]
    fn hostile_formulas_are_cut_not_fatal() {
        with_painter(|p| {
            let c = Color32::WHITE;
            assert!(layout(p, "", false, 14.0, c).is_none());
            assert!(layout(p, &"x".repeat(MAX_TEX + 1), false, 14.0, c).is_none());
            let deep = format!("{}x{}", r"\frac{".repeat(200), "}{2}".repeat(200));
            let f = layout(p, deep.get(..MAX_TEX.min(deep.len())).unwrap(), true, 14.0, c);
            assert!(f.is_some());
            let wide = "x+".repeat(1900);
            let f = layout(p, &wide, false, 14.0, c).unwrap();
            assert!(f.b.items.len() <= MAX_ITEMS);
            for tex in [r"\frac", r"^^^", r"\sqrt", r"\left(", r"\begin{matrix}", "}}}{{{", r"\\\\&&&", r"\unknown{x}", "$", r"\text{"] {
                let _ = layout(p, tex, true, 14.0, c);
            }
        });
    }
}
