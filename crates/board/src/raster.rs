//! The board as a picture (#60, „Bild (PNG)“), without a window or a GPU.
//!
//! The canvas paints the elements exactly as on screen (`paint_elements`) into a headless egui
//! context; its meshes and font atlas are then filled in by a small software rasteriser. White
//! background, everything inside with a margin, never larger than `max_edge` on either side
//! (large boards are scaled down).

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId, textures::TexturesDelta};
use egui::{Color32, Pos2, RawInput, Rect, pos2, vec2};

use crate::model::Board;
use crate::view::{BoardStyle, Camera, ViewState, drawn_bounds, paint_elements, refresh};

/// Longest side of an exported picture (pixels).
pub const MAX_IMAGE_EDGE: u32 = 4096;
/// Space around the content (pixels).
const MARGIN: f32 = 32.0;
/// Small boards are drawn at twice their size, so text stays sharp on a projector.
const MAX_SCALE: f32 = 2.0;

/// An RGB picture, row by row from the top, 3 bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Board {
    /// The board as a picture no larger than `max_edge` (clamped to 64..=[`MAX_IMAGE_EDGE`]) on
    /// either side; `None` when there is nothing on it.
    pub fn render_image(&self, max_edge: u32) -> Option<Raster> {
        let max_edge = max_edge.clamp(64, MAX_IMAGE_EDGE) as f32;
        let mut st = ViewState::default();
        refresh(&mut st, self);
        let b = drawn_bounds(&st)?;
        let room = max_edge - 2.0 * MARGIN;
        let (bw, bh) = (b.width().max(1.0), b.height().max(1.0));
        let scale = (room / bw).min(room / bh).min(MAX_SCALE);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let w = (bw * scale + 2.0 * MARGIN).ceil().clamp(1.0, max_edge);
        let h = (bh * scale + 2.0 * MARGIN).ceil().clamp(1.0, max_edge);
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        st.camera = Camera { center: b.center(), zoom: scale };
        let style = BoardStyle { bg: Color32::WHITE, ..BoardStyle::light() };

        let ctx = egui::Context::default();
        let input = RawInput { screen_rect: Some(canvas), max_texture_side: Some(8192), ..RawInput::default() };
        let out = ctx.run_ui(input, |ui| {
            let painter = ui.painter_at(canvas);
            paint_elements(&painter, ui, canvas, &st, &style);
        });
        let mut textures = Textures::default();
        textures.apply(out.textures_delta);
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let (wu, hu) = (w as usize, h as usize);
        let mut px = Canvas { w: wu, h: hu, rgb: vec![[255.0; 3]; wu * hu] };
        for p in &prims {
            px.mesh(p, &textures);
        }
        let rgb = px.rgb.iter().flat_map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
        Some(Raster { width: w as u32, height: h as u32, rgb })
    }
}

#[derive(Default)]
struct Textures {
    map: HashMap<TextureId, Texture>,
}

struct Texture {
    w: usize,
    h: usize,
    px: Vec<Color32>,
}

impl Textures {
    fn apply(&mut self, mut delta: TexturesDelta) {
        delta.free.clear();
        for (id, deltas) in delta.set.drain() {
            for d in deltas {
                let ImageData::Color(img) = &d.image;
                let [iw, ih] = img.size;
                match d.pos {
                    None => {
                        self.map.insert(id, Texture { w: iw, h: ih, px: img.pixels.clone() });
                    }
                    Some([x0, y0]) => {
                        let Some(t) = self.map.get_mut(&id) else { continue };
                        for y in 0..ih {
                            for x in 0..iw {
                                let (tx, ty) = (x0 + x, y0 + y);
                                if tx >= t.w || ty >= t.h {
                                    continue;
                                }
                                if let (Some(dst), Some(src)) = (t.px.get_mut(ty * t.w + tx), img.pixels.get(y * iw + x)) {
                                    *dst = *src;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Nearest texel at normalised `uv`; opaque white for an unknown texture.
    fn sample(&self, id: TextureId, uv: Pos2) -> [f32; 4] {
        let Some(t) = self.map.get(&id) else { return [255.0; 4] };
        if t.w == 0 || t.h == 0 {
            return [255.0; 4];
        }
        let x = ((uv.x * t.w as f32) as isize).clamp(0, t.w as isize - 1) as usize;
        let y = ((uv.y * t.h as f32) as isize).clamp(0, t.h as isize - 1) as usize;
        t.px.get(y * t.w + x).map_or([255.0; 4], |c| [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32])
    }
}

struct Canvas {
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
}

/// Which triangle owns a pixel centre exactly on a shared edge: exactly one of an edge and its
/// reverse, so nothing is blended twice.
fn owns(a: Pos2, b: Pos2) -> bool {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    dy > 0.0 || (dy == 0.0 && dx < 0.0)
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

impl Canvas {
    fn mesh(&mut self, prim: &ClippedPrimitive, tex: &Textures) {
        let Primitive::Mesh(mesh) = &prim.primitive else { return };
        let clip = prim.clip_rect.intersect(Rect::from_min_size(Pos2::ZERO, vec2(self.w as f32, self.h as f32)));
        if !clip.is_positive() {
            return;
        }
        for &[i, j, k] in mesh.indices.as_chunks::<3>().0 {
            let v = |n: u32| mesh.vertices.get(n as usize);
            let (Some(a), Some(b), Some(c)) = (v(i), v(j), v(k)) else { continue };
            self.triangle([a.pos, b.pos, c.pos], [a.color, b.color, c.color], [a.uv, b.uv, c.uv], mesh.texture_id, clip, tex);
        }
    }

    fn triangle(&mut self, pos: [Pos2; 3], col: [Color32; 3], uv: [Pos2; 3], id: TextureId, clip: Rect, tex: &Textures) {
        let [a, mut b, mut c] = pos;
        let [ca, mut cb, mut cc] = col;
        let [ua, mut ub, mut uc] = uv;
        if !pos.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        let mut area = edge(a, b, c);
        if area.abs() < 1e-9 {
            return;
        }
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
            std::mem::swap(&mut cb, &mut cc);
            std::mem::swap(&mut ub, &mut uc);
            area = -area;
        }
        let x_min = a.x.min(b.x).min(c.x).max(clip.min.x);
        let x_max = a.x.max(b.x).max(c.x).min(clip.max.x);
        let y_min = a.y.min(b.y).min(c.y).max(clip.min.y);
        let y_max = a.y.max(b.y).max(c.y).min(clip.max.y);
        if x_min >= x_max || y_min >= y_max {
            return;
        }
        // Pixel centres at +0.5; rows and columns whose centres lie in the clipped box.
        let row0 = (y_min - 0.5).ceil().max(0.0) as usize;
        let row1 = ((y_max - 0.5).floor().max(-1.0) as isize).min(self.h as isize - 1);
        let (own_bc, own_ca, own_ab) = (owns(b, c), owns(c, a), owns(a, b));
        let f = |c: Color32| [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32];
        let (ka, kb, kc) = (f(ca), f(cb), f(cc));
        let mut row = row0 as isize;
        while row <= row1 {
            let py = row as f32 + 0.5;
            // Where this row crosses the triangle, so long thin shapes cost only their area.
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for (p, q) in [(a, b), (b, c), (c, a)] {
                if (p.y - py) * (q.y - py) <= 0.0 {
                    if p.y == q.y {
                        lo = lo.min(p.x.min(q.x));
                        hi = hi.max(p.x.max(q.x));
                    } else {
                        let x = p.x + (py - p.y) * (q.x - p.x) / (q.y - p.y);
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
            row += 1;
            if !lo.is_finite() || !hi.is_finite() {
                continue;
            }
            let col0 = ((lo.max(x_min) - 0.5).ceil() - 1.0).max(0.0) as usize;
            let col1 = (((hi.min(x_max) - 0.5).floor() + 1.0).max(-1.0) as isize).min(self.w as isize - 1);
            let y = (row - 1) as usize;
            let mut x = col0 as isize;
            while x <= col1 {
                let p = pos2(x as f32 + 0.5, py);
                x += 1;
                if p.x < clip.min.x || p.x >= clip.max.x || py < clip.min.y || py >= clip.max.y {
                    continue;
                }
                let (e0, e1, e2) = (edge(b, c, p), edge(c, a, p), edge(a, b, p));
                let inside = |e: f32, own: bool| e > 0.0 || (e == 0.0 && own);
                if !(inside(e0, own_bc) && inside(e1, own_ca) && inside(e2, own_ab)) {
                    continue;
                }
                let (w0, w1, w2) = (e0 / area, e1 / area, e2 / area);
                let t = tex.sample(id, pos2(ua.x * w0 + ub.x * w1 + uc.x * w2, ua.y * w0 + ub.y * w1 + uc.y * w2));
                let mut s = [0.0_f32; 4];
                for (i, v) in s.iter_mut().enumerate() {
                    let k = |c: &[f32; 4]| c.get(i).copied().unwrap_or(0.0);
                    *v = (k(&ka) * w0 + k(&kb) * w1 + k(&kc) * w2) * t.get(i).copied().unwrap_or(255.0) / 255.0;
                }
                let Some(dst) = self.rgb.get_mut(y * self.w + (x - 1) as usize) else { continue };
                // Premultiplied colours, blended in gamma space like egui's own renderers.
                let keep = 1.0 - (s[3] / 255.0).clamp(0.0, 1.0);
                for (d, v) in dst.iter_mut().zip(s) {
                    *d = (v + *d * keep).clamp(0.0, 255.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, Kind};

    fn ink_pixels(r: &Raster) -> usize {
        r.rgb.as_chunks::<3>().0.iter().filter(|p| p.iter().any(|&v| v < 200)).count()
    }

    #[test]
    fn an_empty_board_has_no_picture() {
        assert!(Board::new().render_image(MAX_IMAGE_EDGE).is_none());
    }

    #[test]
    fn notes_shapes_arrows_text_and_drawings_are_drawn_inside_a_white_margin() {
        let mut b = Board::new();
        let n = Element::new(Kind::Note, 0.0, 0.0).with_size(200.0, 140.0).with_text("Photosynthese").with_color("yellow");
        let m = Element::new(Kind::Rect, 400.0, 0.0).with_size(160.0, 100.0).with_text("Licht").with_color("blue");
        let arrow = Element::connector(&n, &m);
        b.add_many(vec![
            n,
            m,
            arrow,
            Element::new(Kind::Ellipse, 0.0, 250.0).with_size(120.0, 80.0).with_color("green"),
            Element::new(Kind::Text, 200.0, 260.0).with_size(200.0, 30.0).with_text("Sauerstoff"),
            Element::path(Kind::Freehand, &[(400.0, 250.0), (450.0, 300.0), (520.0, 260.0)]),
            Element::new(Kind::Frame, -40.0, -40.0).with_size(700.0, 420.0).with_text("Pflanzen"),
        ]);
        let r = b.render_image(MAX_IMAGE_EDGE).unwrap();
        assert_eq!(r.rgb.len(), (r.width * r.height * 3) as usize);
        assert!(r.width <= MAX_IMAGE_EDGE && r.height <= MAX_IMAGE_EDGE);
        // 700 × 444 world units at twice the size, plus the margin.
        assert!(r.width > 1400 && r.width < 1500, "{}x{}", r.width, r.height);
        assert!(ink_pixels(&r) > 5_000, "something is drawn");
        // The margin stays white: first and last rows and columns.
        let px = |x: u32, y: u32| &r.rgb[((y * r.width + x) * 3) as usize..((y * r.width + x) * 3 + 3) as usize];
        for x in 0..r.width {
            assert_eq!(px(x, 0), [255, 255, 255]);
            assert_eq!(px(x, r.height - 1), [255, 255, 255]);
        }
        for y in 0..r.height {
            assert_eq!(px(0, y), [255, 255, 255]);
            assert_eq!(px(r.width - 1, y), [255, 255, 255]);
        }
        // The note's yellow is in the picture.
        assert!(r.rgb.as_chunks::<3>().0.contains(&[0xfd, 0xe6, 0x8a]), "the note's colour");
    }

    #[test]
    fn huge_and_crowded_boards_stay_within_the_cap() {
        let mut far = Board::new();
        far.add_many(vec![
            Element::new(Kind::Note, -1.0e7, -1.0e7).with_text("weit weg"),
            Element::new(Kind::Rect, 1.0e7, 1.0e7).with_size(1.0e6, 1.0e6),
            Element::segment(Kind::Line, (-1.0e7, 1.0e7), (1.0e7, -1.0e7)),
        ]);
        let r = far.render_image(MAX_IMAGE_EDGE).unwrap();
        assert!(r.width <= MAX_IMAGE_EDGE && r.height <= MAX_IMAGE_EDGE && ink_pixels(&r) > 0);

        let mut flat = Board::new();
        flat.add(Element::new(Kind::Rect, 0.0, 0.0).with_size(1.0e6, 1.0));
        let r = flat.render_image(MAX_IMAGE_EDGE).unwrap();
        assert_eq!(r.width, MAX_IMAGE_EDGE);
        assert!(r.height < 200);

        let mut crowd = Board::new();
        crowd.add_many((0..3_000).map(|i| Element::new(Kind::Note, (i % 60) as f64 * 220.0, (i / 60) as f64 * 160.0).with_text("x".repeat(i % 300))).collect());
        let r = crowd.render_image(1024).unwrap();
        assert!(r.width <= 1024 && r.height <= 1024 && ink_pixels(&r) > 0);
        assert!(Board::new().render_image(0).is_none());
    }
}
