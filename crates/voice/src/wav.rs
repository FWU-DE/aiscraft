//! 16-bit PCM WAV files, written and read by hand (the format is a 44-byte header).

use aiscraft_core::{Error, Result};

/// Encode mono or interleaved 16-bit samples as a WAV file.
pub fn encode_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate == 0 {
        return Err(Error::invalid("WAV: Abtastrate und Kanalzahl müssen größer als null sein"));
    }
    let data_len = samples.len().checked_mul(2).and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= u32::MAX - 36);
    let Some(data_len) = data_len else {
        return Err(Error::invalid("WAV: Aufnahme zu groß"));
    };
    let block_align = channels.saturating_mul(2);
    let byte_rate = sample_rate.saturating_mul(u32::from(block_align));
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(out)
}

/// A decoded 16-bit PCM WAV file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples.
    pub samples: Vec<i16>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Read a 16-bit PCM WAV file. Other encodings are refused, malformed files give `Err`.
pub fn decode_wav(bytes: &[u8]) -> Result<Wav> {
    let bad = || Error::invalid("Keine gültige WAV-Datei");
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(bad());
    }
    let mut at = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), u32_at(bytes, at + 4)) {
        let body = at + 8;
        let len = len as usize;
        match id {
            b"fmt " => {
                let format = u16_at(bytes, body).ok_or_else(bad)?;
                let channels = u16_at(bytes, body + 2).ok_or_else(bad)?;
                let rate = u32_at(bytes, body + 4).ok_or_else(bad)?;
                let bits = u16_at(bytes, body + 14).ok_or_else(bad)?;
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => {
                let (format, channels, sample_rate, bits) = fmt.ok_or_else(bad)?;
                if format != 1 || bits != 16 || channels == 0 || sample_rate == 0 {
                    return Err(Error::unsupported("Nur 16-Bit-PCM-WAV wird unterstützt"));
                }
                let end = body.saturating_add(len).min(bytes.len());
                let data = bytes.get(body..end).ok_or_else(bad)?;
                let samples = data.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
                return Ok(Wav { sample_rate, channels, samples });
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        at = body.saturating_add(len).saturating_add(len & 1);
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let samples: Vec<i16> = (0..1000).map(|i| ((i * 37) % 65536 - 32768) as i16).collect();
        let bytes = encode_wav(&samples, 16_000, 1).unwrap();
        assert_eq!(bytes.len(), 44 + 2000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(&bytes, 4), Some(36 + 2000));
        assert_eq!(u32_at(&bytes, 24), Some(16_000));
        assert_eq!(u32_at(&bytes, 28), Some(32_000));
        let wav = decode_wav(&bytes).unwrap();
        assert_eq!(wav, Wav { sample_rate: 16_000, channels: 1, samples });
    }

    #[test]
    fn empty_and_stereo() {
        let wav = decode_wav(&encode_wav(&[], 8000, 2).unwrap()).unwrap();
        assert_eq!((wav.sample_rate, wav.channels, wav.samples.len()), (8000, 2, 0));
        assert!(encode_wav(&[1], 0, 1).is_err());
    }

    #[test]
    fn malformed_is_err() {
        assert!(decode_wav(b"").is_err());
        assert!(decode_wav(b"RIFF\0\0\0\0WAVE").is_err());
        assert!(decode_wav(b"RIFF\0\0\0\0WAVEfmt \xff\xff\xff\xff").is_err());
        let mut b = encode_wav(&[1, 2, 3], 16_000, 1).unwrap();
        b[20] = 3; // float format
        assert!(decode_wav(&b).is_err());
        // Truncated data is read as far as it goes.
        let b = encode_wav(&[1, 2, 3], 16_000, 1).unwrap();
        assert_eq!(decode_wav(&b[..48]).unwrap().samples, vec![1, 2]);
    }
}
