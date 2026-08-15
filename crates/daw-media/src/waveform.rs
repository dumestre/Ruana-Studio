//! Waveform decoding e rendering para arquivos de áudio.

use hound;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WaveformData {
    pub samples: Vec<f32>, // amostras normalizadas -1.0 a 1.0
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

/// Decodifica um arquivo WAV e retorna os dados de waveform.
pub fn decode_wav(path: impl AsRef<Path>) -> Result<WaveformData, Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 / i32::MAX as f32)).collect::<Result<Vec<_>, _>>()?
        }
        hound::SampleFormat::Float => {
            reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?
        }
    };

    let channels = spec.channels as u16;
    let sample_rate = spec.sample_rate;

    // Se estéreo, mistura para mono (média dos canais)
    let mono_samples = if channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) * 0.5)
            .collect()
    } else {
        samples
    };

    let duration_seconds = mono_samples.len() as f32 / sample_rate as f32;

    Ok(WaveformData {
        samples: mono_samples,
        sample_rate,
        channels,
        duration_seconds,
    })
}

/// Gera pontos de waveform para rendering (downsample para N pontos).
pub fn waveform_points(data: &WaveformData, points: usize) -> Vec<f32> {
    if data.samples.is_empty() || points == 0 {
        return vec![0.0; points];
    }

    let chunk_size = (data.samples.len() as f32 / points as f32).max(1.0) as usize;
    let mut result = Vec::with_capacity(points);

    for i in 0..points {
        let start = i * chunk_size;
        let end = ((i + 1) * chunk_size).min(data.samples.len());
        if start >= data.samples.len() {
            result.push(0.0);
            continue;
        }
        let chunk = &data.samples[start..end];
        let peak = chunk.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
        result.push(peak);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_downsample() {
        let data = WaveformData {
            samples: vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.0, 0.0, 0.0],
            sample_rate: 44100,
            channels: 1,
            duration_seconds: 0.0,
        };
        let pts = waveform_points(&data, 4);
        assert_eq!(pts.len(), 4);
        assert!(pts[1] > 0.0); // chunk com 0.5 e -0.5 -> peak 0.5
        assert_eq!(pts[3], 1.0); // chunk com 1.0 e -1.0 -> peak 1.0
    }
}