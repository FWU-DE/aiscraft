//! Photos for AIS.chat (#50): decoded, turned upright, scaled to at most 1000 px and encoded
//! again, so location, camera and other metadata stay on the device. Also how HEIC, AVIF, GIF,
//! BMP and TIFF become a format AIS.chat takes.

use aiscraft_core::{Error, Result};
use craft_image_codecs::{ChannelLayout, DecodeOptions, EncodeOptions, Format, Image, Limits, SampleType};

/// The longest side after scaling (AIS.chat scales to this on the server anyway).
pub const MAX_SIDE: u32 = 1000;
/// Larger pictures are refused before decoding.
const MAX_MEGAPIXELS: u64 = 120;

pub struct Photo {
    pub bytes: Vec<u8>,
    /// "jpg" or "png".
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
    pub scaled: bool,
    pub location_removed: bool,
    pub metadata_removed: bool,
}

/// Whether metadata blocks name a place: EXIF GPS or an XMP location property. When a block
/// cannot be read whole, assume it might.
fn has_location(exif: Option<&[u8]>, xmp: Option<&str>) -> bool {
    let gps = exif.is_some_and(|e| craft_metadata::read_exif(e).gps.is_some());
    let place = xmp.is_some_and(|x| craft_metadata::parse_xmp(x).map(|d| d.has_location_metadata() || d.properties_truncated).unwrap_or(true));
    gps || place
}

fn codec_error(e: craft_image_codecs::CodecError, name: &str) -> Error {
    let text = e.to_string();
    if text.to_lowercase().contains("heif") || text.to_lowercase().contains("heic") {
        return Error::unsupported(format!("{name}: HEIC-Fotos kann diese Version von AIS.ChatCraft nicht lesen. Exportiere das Foto als JPEG."));
    }
    Error::invalid(format!("{name} lässt sich als Bild nicht lesen: {text}"))
}

/// Box filter: each target pixel averages the source pixels it covers. `channels` 3 or 4.
fn shrink(src: &[u8], w: u32, h: u32, channels: usize, tw: u32, th: u32) -> Vec<u8> {
    let (w, h, tw, th) = (w as usize, h as usize, tw.max(1) as usize, th.max(1) as usize);
    let mut out = vec![0u8; tw * th * channels];
    let mut sums = vec![0u64; tw * channels];
    let mut counts = vec![0u64; tw];
    let mut ty = 0usize;
    for y in 0..h {
        let row_target = y * th / h.max(1);
        if row_target != ty {
            flush(&mut out, &mut sums, &mut counts, ty, tw, channels);
            ty = row_target;
        }
        let row = src.get(y * w * channels..(y + 1) * w * channels).unwrap_or_default();
        for x in 0..w {
            let tx = x * tw / w.max(1);
            if let (Some(px), Some(c)) = (row.get(x * channels..(x + 1) * channels), counts.get_mut(tx)) {
                *c += 1;
                for (k, v) in px.iter().enumerate() {
                    if let Some(s) = sums.get_mut(tx * channels + k) {
                        *s += u64::from(*v);
                    }
                }
            }
        }
    }
    flush(&mut out, &mut sums, &mut counts, ty, tw, channels);
    out
}

fn flush(out: &mut [u8], sums: &mut [u64], counts: &mut [u64], ty: usize, tw: usize, channels: usize) {
    for tx in 0..tw {
        let n = counts.get(tx).copied().unwrap_or(0).max(1);
        for k in 0..channels {
            let i = tx * channels + k;
            if let (Some(o), Some(s)) = (out.get_mut(ty * tw * channels + i), sums.get(i)) {
                *o = u8::try_from(s / n).unwrap_or(u8::MAX);
            }
        }
    }
    sums.iter_mut().for_each(|s| *s = 0);
    counts.iter_mut().for_each(|c| *c = 0);
}

/// Target size for the longest side `MAX_SIDE`; unchanged when it is smaller.
pub fn fit(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_SIDE {
        return (w, h);
    }
    let scale = |v: u32| u32::try_from((u64::from(v) * u64::from(MAX_SIDE) + u64::from(long) / 2) / u64::from(long)).unwrap_or(1).max(1);
    (scale(w), scale(h))
}

/// Decode `bytes`, drop every metadata block, scale and encode as JPEG (PNG when the picture
/// has transparency or was a PNG, so screenshots stay sharp).
pub fn prepare(bytes: &[u8], name: &str) -> Result<Photo> {
    let format = craft_image_codecs::detect(bytes).ok_or_else(|| Error::invalid(format!("{name} ist kein Bild, das AIS.ChatCraft kennt")))?;
    if let Ok((w, h)) = craft_image_codecs::probe_size(bytes)
        && u64::from(w) * u64::from(h) > MAX_MEGAPIXELS * 1_000_000
    {
        return Err(Error::invalid(format!("{name} ist zu groß (mehr als {MAX_MEGAPIXELS} Megapixel)")));
    }
    let limits = Limits { max_width: 1 << 16, max_height: 1 << 16, max_pixels: MAX_MEGAPIXELS * 1_000_000, max_alloc: 1 << 30 };
    let img = craft_image_codecs::decode_with(bytes, &DecodeOptions { limits, ..DecodeOptions::default() }).map_err(|e| codec_error(e, name))?;
    // Read the blocks in the file as well as those the decoder kept.
    let embedded = craft_metadata::embedded(bytes);
    let location = has_location(img.meta.exif.as_deref(), img.meta.xmp.as_deref())
        || has_location(embedded.exif.as_deref(), embedded.xmp.as_deref())
        || has_location(None, embedded.xmp_extended.as_deref());
    let metadata = img.meta.exif.is_some()
        || img.meta.xmp.is_some()
        || embedded.exif.is_some()
        || embedded.xmp.is_some()
        || embedded.iptc.is_some()
        || !img.meta.text.is_empty();
    let alpha = matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA);
    let png = alpha || format == Format::Png;
    let (layout, channels) = if alpha { (ChannelLayout::Rgba, 4) } else { (ChannelLayout::Rgb, 3) };
    let flat = img.convert(layout, SampleType::U8);
    let (w, h) = (flat.width(), flat.height());
    let (tw, th) = fit(w, h);
    let scaled = (tw, th) != (w, h);
    let pixels = if scaled { shrink(flat.data(), w, h, channels, tw, th) } else { flat.data().to_vec() };
    let mut out = Image::from_u8(tw, th, layout, pixels).map_err(|e| codec_error(e, name))?;
    // Only the colour profile comes along: it holds no personal data.
    out.icc = img.icc.clone().filter(|_| matches!(img.layout(), ChannelLayout::Rgb | ChannelLayout::Rgba));
    let opts = EncodeOptions { jpeg_quality: 85, ..EncodeOptions::default() };
    let (fmt, ext) = if png { (Format::Png, "png") } else { (Format::Jpeg, "jpg") };
    let bytes = craft_image_codecs::encode(&out, fmt, &opts).map_err(|e| Error::invalid(format!("{name} ließ sich nicht neu speichern: {e}")))?;
    Ok(Photo { bytes, ext, width: tw, height: th, scaled, location_removed: location, metadata_removed: metadata })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A JPEG of `w`×`h` with an EXIF block that holds a GPS position and a camera model.
    pub(crate) fn jpeg_with_gps(w: u32, h: u32) -> Vec<u8> {
        let pixels: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 200]).collect();
        let mut img = Image::from_u8(w, h, ChannelLayout::Rgb, pixels).unwrap();
        let meta = craft_metadata::Metadata {
            model: Some("Testkamera X".into()),
            gps: Some(craft_metadata::Gps { latitude: 52.52, longitude: 13.40, altitude: None }),
            ..Default::default()
        };
        img.meta.exif = Some(craft_metadata::write_exif(&meta));
        craft_image_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap()
    }

    #[test]
    fn strips_location_and_scales() {
        let src = jpeg_with_gps(1600, 800);
        assert!(craft_metadata::read_exif(craft_metadata::embedded(&src).exif.as_deref().unwrap()).gps.is_some(), "fixture carries GPS");
        let p = prepare(&src, "Foto.jpg").unwrap();
        assert_eq!((p.width, p.height, p.ext), (1000, 500, "jpg"));
        assert!(p.scaled && p.location_removed && p.metadata_removed);
        let back = craft_metadata::embedded(&p.bytes);
        assert!(back.exif.is_none() && back.xmp.is_none(), "no metadata left");
        assert_eq!(craft_image_codecs::probe_size(&p.bytes).unwrap(), (1000, 500));
    }

    #[test]
    fn small_png_stays_png_and_hostile_bytes_fail() {
        let img = Image::from_u8(4, 3, ChannelLayout::Rgba, vec![9; 48]).unwrap();
        let png = craft_image_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let p = prepare(&png, "a.png").unwrap();
        assert_eq!((p.width, p.height, p.ext, p.scaled, p.location_removed), (4, 3, "png", false, false));
        assert!(prepare(b"\x89PNG\r\n\x1a\nkaputt", "b.png").is_err());
        assert!(prepare(b"kein bild", "c.jpg").is_err());
        assert_eq!(fit(3000, 10), (1000, 3));
        assert_eq!(fit(10, 10), (10, 10));
    }
}
