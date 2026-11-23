//! Downmix to mono and resample to the 16 kHz speech recognisers expect, in a streaming way
//! (input arrives in device-sized buffers).

/// Average interleaved frames into mono.
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let ch = usize::from(channels.max(1));
    if ch == 1 {
        return interleaved.to_vec();
    }
    interleaved.chunks(ch).map(|frame| frame.iter().sum::<f32>() / frame.len() as f32).collect()
}

/// Root mean square of a buffer, clamped to 0..1.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    ((sum / samples.len() as f64).sqrt() as f32).clamp(0.0, 1.0)
}

/// Convert a float sample (-1..1) to i16, clipping.
pub fn to_i16(s: f32) -> i16 {
    if s.is_nan() {
        return 0;
    }
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Streaming mono resampler. Downsampling averages the input over each output period (a box
/// filter, enough against aliasing for speech); upsampling interpolates linearly.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    // Downsampling state.
    acc: f64,
    count: u32,
    consumed: f64,
    boundary: f64,
    // Upsampling state.
    prev: Option<f32>,
    t: f64,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        let step = f64::from(from_rate.max(1)) / f64::from(to_rate.max(1));
        Self { step, acc: 0.0, count: 0, consumed: 0.0, boundary: step, prev: None, t: 0.0 }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.step - 1.0).abs() < 1e-9 {
            out.extend_from_slice(input);
        } else if self.step > 1.0 {
            for &x in input {
                self.acc += f64::from(x);
                self.count += 1;
                self.consumed += 1.0;
                if self.consumed >= self.boundary {
                    out.push((self.acc / f64::from(self.count)) as f32);
                    self.acc = 0.0;
                    self.count = 0;
                    self.boundary += self.step;
                }
            }
            // Keep the counters small so precision does not drift over a long recording.
            if self.consumed > 1e6 {
                self.boundary -= self.consumed;
                self.consumed = 0.0;
            }
        } else {
            for &x in input {
                let Some(prev) = self.prev else {
                    out.push(x);
                    self.prev = Some(x);
                    self.t = self.step;
                    continue;
                };
                while self.t <= 1.0 {
                    out.push(prev + (x - prev) * self.t as f32);
                    self.t += self.step;
                }
                self.t -= 1.0;
                self.prev = Some(x);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_frames() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2), vec![0.5, 0.5, 0.0]);
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
        assert_eq!(downmix(&[0.3, 0.3, 0.3], 3), vec![0.3]);
    }

    #[test]
    fn rms_and_clipping() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(rms(&[4.0]), 1.0);
        assert_eq!(to_i16(1.5), 32767);
        assert_eq!(to_i16(-1.0), -32767);
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(f32::NAN), 0);
    }

    #[test]
    fn downsample_48k_counts_and_averages() {
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        // In odd buffer sizes, like a device would deliver.
        let input: Vec<f32> = (0..48_000).map(|i| (i % 3) as f32).collect();
        for chunk in input.chunks(441) {
            r.process(chunk, &mut out);
        }
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| (s - 1.0).abs() < 1e-6), "mean of 0,1,2");
    }

    #[test]
    fn downsample_44k1_length() {
        let mut r = Resampler::new(44_100, 16_000);
        let mut out = Vec::new();
        for chunk in vec![0.25f32; 44_100 * 2].chunks(512) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 32_000).abs() <= 1, "{}", out.len());
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn upsample_interpolates() {
        let mut r = Resampler::new(8_000, 16_000);
        let mut out = Vec::new();
        r.process(&[0.0, 1.0], &mut out);
        r.process(&[0.0], &mut out);
        assert_eq!(out, vec![0.0, 0.5, 1.0, 0.5, 0.0]);
    }

    #[test]
    fn same_rate_passes_through() {
        let mut r = Resampler::new(16_000, 16_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2], &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
    }
}
