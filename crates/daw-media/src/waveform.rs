//! Waveform decoding e rendering para arquivos de áudio.

use hound;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WaveformData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

/// Decodifica um arquivo WAV via hound (rápido e confiável).
fn decode_wav_file(path: &Path) -> Result<WaveformData, Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / i32::MAX as f32))
            .collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
    };

    let channels = spec.channels as u16;
    let sample_rate = spec.sample_rate;

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

/// Decodifica MP3/FLAC/OGG via symphonia.
fn decode_symphonia(path: &Path) -> Result<WaveformData, Box<dyn std::error::Error>> {
    use symphonia::core::audio::{Channels, SampleBuffer};
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let src = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no supported audio track found")?;

    let codec_params = &track.codec_params;
    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.unwrap_or(Channels::FRONT_LEFT);
    let channel_count = channels.count();

    let mut decoder = symphonia::default::get_codecs()
        .make(codec_params, &DecoderOptions::default())?;

    let track_id = track.id;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let buf = sample_buf.get_or_insert_with(|| {
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec())
                });
                buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(buf.samples());
            }
            Err(_) => continue,
        }
    }

    // Mistura para mono se necessário
    let mono_samples = if channel_count == 2 {
        all_samples
            .chunks(2)
            .map(|c| if c.len() == 2 { (c[0] + c[1]) * 0.5 } else { c[0] })
            .collect()
    } else if channel_count > 2 {
        all_samples
            .chunks(channel_count)
            .map(|c| c[0])
            .collect()
    } else {
        all_samples
    };

    let duration_seconds = mono_samples.len() as f32 / sample_rate as f32;

    Ok(WaveformData {
        samples: mono_samples,
        sample_rate,
        channels: channel_count as u16,
        duration_seconds,
    })
}

/// Decodifica um arquivo de áudio (WAV, MP3, FLAC, OGG).
/// Usa hound para WAV (mais rápido) e symphonia para o resto.
pub fn decode_audio(path: impl AsRef<Path>) -> Result<WaveformData, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();

    match ext.as_str() {
        "WAV" => decode_wav_file(path),
        "MP3" | "FLAC" | "OGG" | "OGA" => decode_symphonia(path),
        _ => Err(format!("unsupported audio format: {}", ext).into()),
    }
}

/// Decodifica um arquivo WAV (mantido para compatibilidade).
pub fn decode_wav(path: impl AsRef<Path>) -> Result<WaveformData, Box<dyn std::error::Error>> {
    decode_wav_file(path.as_ref())
}

/// Gera pontos de waveform para rendering (downsample para N picos normalizados 0.0–1.0).
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
        assert!(pts[1] > 0.0);
        assert_eq!(pts[3], 1.0);
    }
}
