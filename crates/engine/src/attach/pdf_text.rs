//! The text of a PDF, for project knowledge (#32): craft-pdf's interpreter draws every page into
//! a device that keeps the glyphs' Unicode values and positions; lines and spaces come from where
//! the glyphs stand. Good enough for searching and quoting curricula and worksheets; scanned
//! pages (pictures of text) give nothing, and that is said.

use aiscraft_core::{Error, Result};
use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::{BlendMode, ClipPath, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask};
use kurbo::{Affine, BezPath, Point, Shape};

/// At most this many pages are read.
const MAX_PAGES: usize = 2000;
/// At most this many characters are kept.
pub const MAX_CHARS: usize = 2_000_000;

#[derive(Default)]
struct Collector {
    out: String,
    /// Baseline and right edge of the last glyph, and its em size.
    last: Option<(f64, f64, f64)>,
    full: bool,
}

impl Collector {
    fn push(&mut self, s: &str) {
        if self.out.len() + s.len() > MAX_CHARS {
            self.full = true;
            return;
        }
        self.out.push_str(s);
    }
}

impl<'a> Device<'a> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, _: &Paint<'a>, _: &GlyphDrawMode) {
        if self.full {
            return;
        }
        let Some(u) = glyph.as_unicode() else { return };
        let text = match u {
            craft_pdf::interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
            craft_pdf::interpret::hayro_cmap::BfString::String(s) => s,
        };
        if text.chars().all(char::is_control) {
            return;
        }
        let m = transform * glyph_transform;
        let origin = m * Point::ORIGIN;
        // Glyph outlines are in a 1000-unit em; the matrix scales them to the page.
        let det = m.determinant().abs().sqrt();
        let em = if det * 1000.0 > 0.5 { det * 1000.0 } else { det.max(0.5) };
        let width = match glyph {
            Glyph::Outline(g) => {
                let b = g.outline().bounding_box();
                if b.width() > 0.0 { b.max_x() * det } else { em * 0.25 }
            }
            Glyph::Type3(_) => em * 0.5,
        };
        if let Some((y, right, last_em)) = self.last {
            let size = em.max(last_em);
            if (origin.y - y).abs() > size * 0.5 {
                // A new line; a wide gap starts a new paragraph.
                self.push(if (origin.y - y).abs() > size * 1.9 { "\n\n" } else { "\n" });
            } else if origin.x - right > size * 0.18 && !self.out.ends_with(' ') && !text.starts_with(' ') {
                self.push(" ");
            }
        }
        self.push(&text);
        self.last = Some((origin.y, origin.x + width, em));
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The text of every page, pages apart by a blank line.
pub fn text(bytes: &[u8]) -> Result<String> {
    let pdf = craft_pdf::open_slice(bytes, None).map_err(|e| {
        let m = e.to_string();
        if m.to_lowercase().contains("password") {
            Error::unsupported("Die PDF-Datei ist mit einem Passwort geschützt. Entferne den Schutz zuerst (z. B. „Drucken als PDF“).")
        } else {
            Error::invalid(format!("Die PDF-Datei lässt sich nicht lesen: {m}"))
        }
    })?;
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    let mut all = String::new();
    for page in pdf.pages().iter().take(MAX_PAGES) {
        let crop = page.intersected_crop_box();
        let crop = kurbo::Rect::new(crop.x0, crop.y0, crop.x1, crop.y1);
        let initial = page.initial_transform(true);
        let initial = Affine::new(initial.as_coeffs());
        let mut context = craft_pdf::interpret::Context::new(initial, crop, &cache, pdf.xref(), settings.clone());
        let mut dev = Collector::default();
        // A page the interpreter cannot read is left out; the others still count.
        if craft_pdf::interpret_page(page, &mut context, &mut dev).is_err() {
            continue;
        }
        let t = dev.out.trim();
        if !t.is_empty() {
            if !all.is_empty() {
                all.push_str("\n\n");
            }
            all.push_str(t);
        }
        if all.len() >= MAX_CHARS || dev.full {
            break;
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    #[test]
    fn text_of_a_pdf_written_by_the_app_comes_back() {
        let md = "# Lehrplan Biologie\n\nIgel halten Winterschlaf von November bis März.\n\nZweiter Absatz über Fotosynthese.";
        let pdf = crate::office::pdf::markdown_pdf("Lehrplan", md).unwrap();
        let t = super::text(&pdf).unwrap();
        assert!(t.contains("Lehrplan Biologie"), "{t}");
        assert!(t.contains("Igel halten Winterschlaf von November bis März."), "{t}");
        assert!(t.contains("Fotosynthese"), "{t}");
        assert!(super::text(b"keine pdf").is_err());
    }
}
