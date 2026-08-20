/// Resampling linear simples. Trocar por sinc interpolation em v2.
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (input.len() as f64 / ratio).ceil() as usize;

    (0..output_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;

            let a = input.get(idx).copied().unwrap_or(0.0);
            let b = input.get(idx + 1).copied().unwrap_or(0.0);

            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_passthrough() {
        let input = vec![1.0, 2.0, 3.0];
        assert_eq!(resample_linear(&input, 44100, 44100), input);
    }

    #[test]
    fn downsample_2x() {
        let input = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&input, 44100, 22050);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 0.01);
        assert!((out[1] - (-0.5)).abs() < 0.01);
    }

    #[test]
    fn upsample_2x() {
        let input = vec![0.0, 1.0];
        let out = resample_linear(&input, 22050, 44100);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 0.0).abs() < 0.01);
        assert!((out[1] - 0.5).abs() < 0.01);
        assert!((out[2] - 1.0).abs() < 0.01);
        assert!((out[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn empty_input() {
        assert!(resample_linear(&[], 44100, 48000).is_empty());
    }
}
