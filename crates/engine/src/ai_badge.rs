//! „KI-Kennzeichnung“ on generated images, as AIS.chat does it: a filled black circle with the
//! white letters „AI“, 56 px across, 28 px from the right and bottom edge of the full picture,
//! drawn on top of it. Small pictures get a proportionally smaller badge (never more than a sixth
//! of the shorter side). The circle and the letters are vector shapes, sampled 4×4 per pixel, so
//! the edges are smooth without a font file.
//!
//! Our own drawing: AIS.chat's `ai-badge.png` is not used.

use aiscraft_core::{Error, Result};
use craft_image_codecs::{ChannelLayout, DecodeOptions, EncodeOptions, Format, Image, Limits, SampleType};

/// Diameter of the badge on a picture large enough for it (pixels).
pub const DIAMETER: f32 = 56.0;
/// Distance from the right and bottom edge (pixels).
pub const MARGIN: f32 = 28.0;
/// Larger files are refused before decoding.
const MAX_BYTES: usize = 25 * 1024 * 1024;
/// Larger pictures are refused before decoding (width × height).
const MAX_PIXELS: u64 = 40_000_000;
/// What decoding may allocate: a 40 MP picture as 8-bit RGBA is 160 MB; with the converted
/// copy and the encoder this stays well under half a gigabyte, also on school laptops.
const MAX_ALLOC: u64 = 256 * 1024 * 1024;
/// Samples per pixel and axis.
const SS: usize = 4;

/// Where the badge sits on a `w`×`h` picture: centre and radius in pixels.
pub fn geometry(w: u32, h: u32) -> (f32, f32, f32) {
    let short = w.min(h) as f32;
    let scale = (short / 6.0 / DIAMETER).min(1.0);
    let r = DIAMETER * scale / 2.0;
    let m = MARGIN * scale;
    (w as f32 - m - r, h as f32 - m - r, r)
}

/// Convex polygons of the letters in badge units (radius 1, y downwards), before centring.
const A_LEFT: [(f32, f32); 4] = [(-0.30, -0.42), (-0.14, -0.42), (-0.44, 0.42), (-0.62, 0.42)];
const A_RIGHT: [(f32, f32); 4] = [(-0.26, -0.42), (-0.10, -0.42), (0.22, 0.42), (0.04, 0.42)];
const A_BAR: [(f32, f32); 4] = [(-0.40, 0.08), (0.0, 0.08), (0.0, 0.22), (-0.40, 0.22)];
const I_STEM: [(f32, f32); 4] = [(0.30, -0.42), (0.46, -0.42), (0.46, 0.42), (0.30, 0.42)];
/// Moves the letters (spanning -0.62..0.46) to the middle of the circle.
const SHIFT_X: f32 = 0.08;

fn in_convex(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let n = poly.len();
    let (mut pos, mut neg) = (false, false);
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + 1) % n)) else { return false };
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        pos |= cross > 0.0;
        neg |= cross < 0.0;
    }
    !(pos && neg)
}

/// What a point in badge units shows: `None` outside the circle, `Some(true)` on a letter.
fn sample(u: f32, v: f32) -> Option<bool> {
    if u * u + v * v > 1.0 {
        return None;
    }
    let p = (u - SHIFT_X, v);
    Some([&A_LEFT, &A_RIGHT, &A_BAR, &I_STEM].iter().any(|poly| in_convex(p, &poly[..])))
}

/// Draws the badge into interleaved 8-bit pixels (`channels` 1 to 4; the first three are colour
/// when there are three or more, the fourth alpha). Wrong sizes leave the pixels as they are.
pub fn draw(px: &mut [u8], w: u32, h: u32, channels: usize) {
    if w == 0 || h == 0 || !(1..=4).contains(&channels) || px.len() != w as usize * h as usize * channels {
        return;
    }
    let (cx, cy, r) = geometry(w, h);
    if r.is_nan() || r <= 0.0 {
        return;
    }
    let x0 = (cx - r).floor().max(0.0) as u32;
    let y0 = (cy - r).floor().max(0.0) as u32;
    let x1 = ((cx + r).ceil().max(0.0) as u32).min(w);
    let y1 = ((cy + r).ceil().max(0.0) as u32).min(h);
    let total = (SS * SS) as f32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (mut black, mut white) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px_x = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let px_y = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    match sample((px_x - cx) / r, (px_y - cy) / r) {
                        Some(true) => white += 1,
                        Some(false) => black += 1,
                        None => {}
                    }
                }
            }
            if black + white == 0 {
                continue;
            }
            let (fb, fw) = (black as f32 / total, white as f32 / total);
            let keep = 1.0 - fb - fw;
            let at = (y as usize * w as usize + x as usize) * channels;
            let Some(p) = px.get_mut(at..at + channels) else { continue };
            let colour = if channels >= 3 { 3 } else { 1 };
            for (k, c) in p.iter_mut().enumerate() {
                let old = f32::from(*c);
                let new = if k < colour { old * keep + 255.0 * fw } else { old * keep + 255.0 * (fb + fw) };
                *c = new.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A picture with the badge baked in, and the format it was encoded in.
pub struct Badged {
    pub bytes: Vec<u8>,
    /// "jpg" for a JPEG (so a file named `.jpg` stays one), "png" for everything else.
    pub ext: &'static str,
}

/// Decodes `bytes` (PNG, JPEG, WebP, GIF …), draws the badge and encodes again: JPEG stays
/// JPEG, everything else becomes PNG. Hostile or oversized input gives an error.
pub fn bake(bytes: &[u8]) -> Result<Badged> {
    let unreadable = |e: &dyn std::fmt::Display| Error::invalid(format!("Das Bild lässt sich für die KI-Kennzeichnung nicht lesen: {e}"));
    if bytes.len() > MAX_BYTES {
        return Err(Error::invalid("Das Bild ist größer als 25 MB"));
    }
    let format = craft_image_codecs::detect(bytes).ok_or_else(|| {
        Error::invalid(
            "Das Bild lässt sich für die KI-Kennzeichnung nicht lesen: unbekanntes Format. Schalte die KI-Kennzeichnung aus, um es unverändert zu speichern.",
        )
    })?;
    if let Ok((w, h)) = craft_image_codecs::probe_size(bytes)
        && u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(too_large());
    }
    let limits = Limits { max_width: 1 << 15, max_height: 1 << 15, max_pixels: MAX_PIXELS, max_alloc: MAX_ALLOC };
    let img = craft_image_codecs::decode_with(bytes, &DecodeOptions { limits, ..DecodeOptions::default() }).map_err(|e| match e {
        craft_image_codecs::CodecError::LimitExceeded(_) => too_large(),
        e => unreadable(&e),
    })?;
    let alpha = matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA);
    let (layout, channels) = if alpha { (ChannelLayout::Rgba, 4) } else { (ChannelLayout::Rgb, 3) };
    let icc = img.icc.clone();
    let flat = img.convert(layout, SampleType::U8);
    // Hold at most two copies of the pixels at a time.
    drop(img);
    let (w, h) = (flat.width(), flat.height());
    let mut pixels = flat.data().to_vec();
    drop(flat);
    draw(&mut pixels, w, h, channels);
    let mut out = Image::from_u8(w, h, layout, pixels).map_err(|e| unreadable(&e))?;
    out.icc = icc;
    let jpeg = format == Format::Jpeg && !alpha;
    let (fmt, ext) = if jpeg { (Format::Jpeg, "jpg") } else { (Format::Png, "png") };
    let opts = EncodeOptions { jpeg_quality: 92, ..EncodeOptions::default() };
    let bytes = craft_image_codecs::encode(&out, fmt, &opts).map_err(|e| Error::invalid(format!("Das Bild ließ sich nicht neu speichern: {e}")))?;
    Ok(Badged { bytes, ext })
}

/// Very large pictures are not baked (memory); the original stays usable.
fn too_large() -> Error {
    Error::invalid(
        "Das Bild ist zu groß für die KI-Kennzeichnung (höchstens 40 Megapixel). Schalte die KI-Kennzeichnung aus, um es unverändert zu speichern oder zu kopieren.",
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG header that claims `w`×`h` and has no pixel data.
    pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0]);
        let crc = crate::ai_label::crc32(v.get(12..).unwrap_or_default());
        v.extend_from_slice(&crc.to_be_bytes());
        v
    }

    #[test]
    fn pictures_above_forty_megapixels_are_refused_before_decoding() {
        // 8000 × 6000 = 48 MP: refused by its header, with a hint how to get the original.
        let err = bake(&png_header(8000, 6000)).err().unwrap();
        assert!(err.message.contains("40 Megapixel") && err.message.contains("Schalte die KI-Kennzeichnung aus"), "{}", err.message);
        // 7000 × 5800 = 40.6 MP, also refused; so is a header that promises more than its data can hold.
        assert!(bake(&png_header(7000, 5800)).err().unwrap().message.contains("40 Megapixel"));
        assert!(bake(&png_header(6000, 6000)).is_err());
        const { assert!(MAX_ALLOC <= 256 * 1024 * 1024 && MAX_PIXELS == 40_000_000) };
    }

    fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let img = craft_image_codecs::decode_with(bytes, &DecodeOptions::default()).unwrap().convert(ChannelLayout::Rgb, SampleType::U8);
        (img.width(), img.height(), img.data().to_vec())
    }

    fn rgb(w: u32, h: u32, colour: [u8; 3], fmt: Format) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgb, (0..w * h).flat_map(|_| colour).collect()).unwrap();
        craft_image_codecs::encode(&img, fmt, &EncodeOptions { jpeg_quality: 95, ..EncodeOptions::default() }).unwrap()
    }

    fn at(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) * 3) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    #[test]
    fn badge_sits_bottom_right_and_leaves_the_rest() {
        let src = rgb(1024, 768, [240, 200, 40], Format::Png);
        let out = bake(&src).unwrap();
        assert_eq!(out.ext, "png");
        let (w, h, px) = decode(&out.bytes);
        assert_eq!((w, h), (1024, 768), "size unchanged");
        // Centre 28 + 28 px from the right and bottom edge; the circle is black just inside its rim.
        let (cx, cy, r) = geometry(w, h);
        assert_eq!((cx, cy, r), (968.0, 712.0, 28.0));
        assert_eq!(at(&px, w, 968, 712 - 24), [0, 0, 0], "top of the circle, above the letters");
        assert_eq!(at(&px, w, 968 - 24, 712), [0, 0, 0], "left of the letters");
        // The letters are white: the I's stem lies at u = 0.38..0.54 of the radius.
        let stem_x = (968.0 + 0.46 * 28.0) as u32;
        assert_eq!(at(&px, w, stem_x, 712), [255, 255, 255], "the I");
        let dark = (cy as u32 - 28..cy as u32 + 28)
            .flat_map(|y| (cx as u32 - 28..cx as u32 + 28).map(move |x| (x, y)))
            .filter(|&(x, y)| at(&px, w, x, y) == [0, 0, 0])
            .count();
        let white = (cy as u32 - 28..cy as u32 + 28)
            .flat_map(|y| (cx as u32 - 28..cx as u32 + 28).map(move |x| (x, y)))
            .filter(|&(x, y)| at(&px, w, x, y) == [255, 255, 255])
            .count();
        assert!(dark > 1200 && white > 200, "{dark} dark, {white} white");
        // Elsewhere nothing changed: corners, the margin below and right of the badge.
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w - 10, h - 10), (500, 400), (cx as u32, h - 27)] {
            assert_eq!(at(&px, w, x, y), [240, 200, 40], "({x}, {y})");
        }
        // Anti-aliased: the rim has pixels between the picture and black.
        let rim = (0..w).map(|x| at(&px, w, x, 712)).filter(|p| *p != [0, 0, 0] && *p != [240, 200, 40] && *p != [255, 255, 255]).count();
        assert!(rim >= 2, "smooth edge");
    }

    #[test]
    fn jpeg_stays_jpeg_and_tiny_pictures_get_a_tiny_badge() {
        let out = bake(&rgb(300, 200, [255, 255, 255], Format::Jpeg)).unwrap();
        assert_eq!(out.ext, "jpg");
        let (w, h, px) = decode(&out.bytes);
        assert_eq!((w, h), (300, 200));
        let (cx, cy, r) = geometry(w, h);
        assert!(r <= 200.0 / 12.0 + 0.01 && r > 16.0, "a sixth of the shorter side: {r}");
        let p = at(&px, w, cx as u32, (cy - r * 0.8) as u32);
        assert!(p.iter().all(|&v| v < 40), "dark in the circle: {p:?}");
        assert!(at(&px, w, 5, 5).iter().all(|&v| v > 245), "corner stays white");

        for (w, h) in [(1, 1), (2, 7), (12, 12)] {
            let out = bake(&rgb(w, h, [9, 9, 9], Format::Png)).unwrap();
            assert_eq!(decode(&out.bytes).0, w);
        }
        let (_, _, r) = geometry(12, 12);
        assert!(r * 2.0 <= 2.0 + 0.01);
    }

    #[test]
    fn transparency_is_kept_and_the_badge_is_opaque() {
        let img = Image::from_u8(120, 120, ChannelLayout::Rgba, vec![0; 120 * 120 * 4]).unwrap();
        let png = craft_image_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let out = bake(&png).unwrap();
        let back = craft_image_codecs::decode_with(&out.bytes, &DecodeOptions::default()).unwrap().convert(ChannelLayout::Rgba, SampleType::U8);
        let (cx, cy, _) = geometry(120, 120);
        let i = ((cy as usize) * 120 + cx as usize - 6) * 4;
        assert_eq!(back.data()[i + 3], 255, "badge opaque");
        assert_eq!(back.data()[3], 0, "corner still transparent");
    }

    #[test]
    fn hostile_input_errors() {
        assert!(bake(b"").is_err());
        assert!(bake(b"kein Bild").is_err());
        assert!(bake(b"\x89PNG\r\n\x1a\nkaputt").is_err());
        assert!(bake(&[0xFF, 0xD8, 0xFF, 0xD9]).is_err());
        let mut huge_header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        huge_header.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 8, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert!(bake(&huge_header).is_err(), "65536² is refused");
        // Wrong buffer sizes leave the pixels alone.
        let mut px = vec![7u8; 10];
        draw(&mut px, 100, 100, 3);
        draw(&mut px, 0, 0, 3);
        draw(&mut px, 3, 5, 1);
        assert_eq!(px, vec![7u8; 10]);
    }
}
