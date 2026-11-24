//! Gradients egui does not have as fills: Glas' backdrop (a diagonal gradient with five soft
//! lights, painted as meshes, so it is already "blurred" and needs no backdrop filter) and the
//! vertical gradient of Mutig's sidebar.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, pos2, vec2};

use crate::theme::{Glass, mix};

fn vertex(pos: Pos2, color: Color32) -> Vertex {
    Vertex { pos, uv: WHITE_UV, color }
}

/// The backdrop: the diagonal gradient over `rect`, then the lights, scaled from the 1280 × 800
/// frame of the design to the rect. `strength` dims the lights (1.0 as designed).
pub fn paint(p: &Painter, rect: Rect, g: &Glass, strength: f32) {
    let mut m = Mesh::default();
    let mid = mix(g.backdrop[0], g.backdrop[1], 0.5);
    m.vertices.extend([
        vertex(rect.left_top(), g.backdrop[0]),
        vertex(rect.right_top(), mid),
        vertex(rect.right_bottom(), g.backdrop[1]),
        vertex(rect.left_bottom(), mid),
    ]);
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    p.add(m);
    let (sx, sy) = (rect.width() / 1280.0, rect.height() / 800.0);
    let s = (sx + sy) / 2.0;
    for (x, y, d, blur, color) in g.lights {
        light(p, pos2(rect.left() + x * sx, rect.top() + y * sy), d / 2.0 * s, blur / 2.0 * s, color, strength);
    }
}

/// A soft disc: a circle of radius `r` blurred with a Gaussian of `sigma`, as rings of falling
/// opacity.
fn light(p: &Painter, c: Pos2, r: f32, sigma: f32, color: Color32, strength: f32) {
    if r <= 0.0 || sigma <= 0.0 || !r.is_finite() || !sigma.is_finite() {
        return;
    }
    const RINGS: usize = 28;
    const SEGMENTS: usize = 72;
    let outer = r + 3.0 * sigma;
    let alpha = |d: f32| 0.5 * erfc((d - r) / (sigma * std::f32::consts::SQRT_2)) * strength;
    let tint = |a: f32| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (a.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut m = Mesh::default();
    m.vertices.push(vertex(c, tint(alpha(0.0))));
    for k in 1..=RINGS {
        let d = outer * k as f32 / RINGS as f32;
        let col = tint(alpha(d));
        for s in 0..SEGMENTS {
            let a = s as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            m.vertices.push(vertex(c + vec2(a.cos(), a.sin()) * d, col));
        }
    }
    let ring = |k: usize, s: usize| (1 + (k - 1) * SEGMENTS + s % SEGMENTS) as u32;
    for s in 0..SEGMENTS {
        m.indices.extend([0, ring(1, s), ring(1, s + 1)]);
    }
    for k in 1..RINGS {
        for s in 0..SEGMENTS {
            let (a, b, c2, d) = (ring(k, s), ring(k, s + 1), ring(k + 1, s), ring(k + 1, s + 1));
            m.indices.extend([a, c2, d, a, d, b]);
        }
    }
    p.add(m);
}

/// The complementary error function (Abramowitz and Stegun 7.1.26, error below 1.5e-7).
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let y = t * (0.254_829_6 + t * (-0.284_496_7 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4)))) * (-z * z).exp();
    if x >= 0.0 { y } else { 2.0 - y }
}

/// A rounded rect filled with a vertical gradient from `top` to `bottom` (a fan over its
/// outline, since it is convex).
pub fn vertical_gradient(p: &Painter, rect: Rect, radius: CornerRadius, top: Color32, bottom: Color32) {
    if top == bottom {
        p.rect_filled(rect, radius, top);
        return;
    }
    let mut path = Vec::with_capacity(64);
    egui::epaint::tessellator::path::rounded_rectangle(&mut path, egui::epaint::RoundedRect::new(rect, radius));
    if path.len() < 3 {
        p.rect_filled(rect, radius, top);
        return;
    }
    let h = rect.height().max(1.0);
    let color = |pt: Pos2| lerp(top, bottom, ((pt.y - rect.top()) / h).clamp(0.0, 1.0));
    let mut m = Mesh::default();
    let c = rect.center();
    m.vertices.push(vertex(c, color(c)));
    for pt in &path {
        m.vertices.push(vertex(*pt, color(*pt)));
    }
    let n = path.len() as u32;
    for i in 0..n {
        m.indices.extend([0, 1 + i, 1 + (i + 1) % n]);
    }
    p.add(m);
}

/// Linear blend of two premultiplied colours (alpha too).
fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// The light 1 px edge along the top of a glass surface, inset by its corner radius.
pub fn glass_edge(p: &Painter, rect: Rect, radius: f32, color: Color32) {
    if color.a() == 0 {
        return;
    }
    let y = rect.top() + 0.5;
    p.line_segment([pos2(rect.left() + radius, y), pos2(rect.right() - radius, y)], egui::Stroke::new(1.0, color));
}

#[cfg(test)]
mod tests {
    #[test]
    fn erfc_is_close() {
        assert!((super::erfc(0.0) - 1.0).abs() < 1e-5);
        assert!(super::erfc(3.0) < 1e-4);
        assert!((super::erfc(-3.0) - 2.0).abs() < 1e-4);
    }
}
