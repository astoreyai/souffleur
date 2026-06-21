//! Minimal, real audio helpers: channel downmix and sample-rate conversion.
//!
//! The resampler is plain linear interpolation — genuinely lossy, but a correct
//! and dependency-free conversion adequate for a Phase 0 spike. It is replaced
//! by a windowed-sinc resampler in Phase 1, once STT word-error on resampled
//! audio is something we measure rather than guess.

/// Downmix interleaved multi-channel samples to mono by averaging channels.
pub fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
        .collect()
}

/// Linear-interpolating resample of mono f32 audio from `from_hz` to `to_hz`.
pub fn resample_linear(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || input.len() < 2 {
        return input.to_vec();
    }
    let ratio = to_hz as f64 / from_hz as f64;
    let out_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let last = input.len() - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let idx = src.floor() as usize;
        let frac = src - idx as f64;
        let a = input[idx.min(last)] as f64;
        let b = input[(idx + 1).min(last)] as f64;
        out.push((a + (b - a) * frac) as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_averages_stereo() {
        // L=1.0 R=0.0 -> 0.5 ; L=-1.0 R=1.0 -> 0.0
        let stereo = [1.0, 0.0, -1.0, 1.0];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.0]);
    }

    #[test]
    fn downsample_halves_length() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample_linear(&input, 32000, 16000);
        assert_eq!(out.len(), 50);
        // endpoints preserved
        assert!((out[0] - 0.0).abs() < 1e-3);
    }

    #[test]
    fn identity_when_rates_match() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&input, 16000, 16000), input);
    }
}
