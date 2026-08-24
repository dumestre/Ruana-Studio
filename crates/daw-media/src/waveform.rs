//! Waveform decoding e rendering para arquivos de áudio.

use hound;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WaveformData {
    /// Downmix mono — usado APENAS para desenhar a waveform.
    ///
    /// Não usar para reprodução: somar L+R cancela conteúdo fora
    /// de fase e destrói a imagem stereo do sample.
    pub samples: Vec<f32>,
    /// Samples no layout ORIGINAL do arquivo (intercalado).
    ///
    /// Esta é a fonte para o motor de áudio: preserva os canais
    /// como estão no arquivo, então o sample sai igual ao original.
    pub interleaved: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

impl WaveformData {
    /// Quantidade de frames (samples por canal).
    pub fn frames(&self) -> usize {
        let ch = self.channels.max(1) as usize;
        self.interleaved.len() / ch
    }

    /// Extrai um canal do buffer intercalado.
    ///
    /// Arquivos mono devolvem o mesmo conteúdo para qualquer canal
    /// pedido — quem chama trata mono como "L igual a R".
    pub fn channel(&self, index: usize) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        let index = index.min(ch - 1);
        if ch == 1 {
            return self.interleaved.clone();
        }
        self.interleaved
            .chunks(ch)
            .filter_map(|frame| frame.get(index).copied())
            .collect()
    }
}

/// Decodifica um arquivo WAV via hound (rápido e confiável).
fn decode_wav_file(path: &Path) -> Result<WaveformData, Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1i32 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max_val))
                .collect::<Result<Vec<_>, _>>()?
        }
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
    };

    let channels = spec.channels as u16;
    let sample_rate = spec.sample_rate;

    let mono_samples = downmix_mono(&samples, channels as usize);
    let duration_seconds = mono_samples.len() as f32 / sample_rate as f32;

    Ok(WaveformData {
        samples: mono_samples,
        interleaved: samples,
        sample_rate,
        channels,
        duration_seconds,
    })
}

/// Downmix para mono — SÓ para o desenho da waveform.
///
/// Stereo vira média dos dois canais; multicanal usa o primeiro.
fn downmix_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        0 | 1 => interleaved.to_vec(),
        2 => interleaved
            .chunks(2)
            .map(|c| if c.len() == 2 { (c[0] + c[1]) * 0.5 } else { c[0] })
            .collect(),
        n => interleaved.chunks(n).map(|c| c[0]).collect(),
    }
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

    // Mistura para mono só para o desenho; a reprodução usa o
    // buffer intercalado original.
    let mono_samples = downmix_mono(&all_samples, channel_count);

    let duration_seconds = mono_samples.len() as f32 / sample_rate as f32;

    Ok(WaveformData {
        samples: mono_samples,
        interleaved: all_samples,
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

/// Gera pontos de waveform para rendering.
///
/// FIEL AO ÁUDIO: cada valor combina RMS (energia audível do
/// trecho) com o pico absoluto, SEM normalizar por arquivo —
/// um sample gravado baixo DEVE desenhar baixo. É o mesmo
/// padrão de DAWs profissionais (corpo RMS + crista de pico):
/// arquivos com volumes diferentes produzem waveforms com
/// alturas visivelmente diferentes.
///
/// Retorna N valores em 0.0–1.0 (amplitude real, teto 1.0).
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

        // Pico absoluto do trecho (crista).
        let peak = chunk.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));

        // RMS do trecho (energia média — o "corpo" da waveform).
        // Um trecho denso tem RMS próximo do pico; um trecho
        // esparso tem RMS bem menor — é isto que diferencia os
        // desenhos de arquivos com níveis diferentes.
        let sum_sq: f32 = chunk.iter().map(|&s| s * s).sum();
        let rms = (sum_sq / chunk.len() as f32).sqrt();

        // Corpo RMS + brilho do pico por cima: fiel ao áudio e
        // ainda mostra as transientes. Piso mínimo só para o
        // traço não sumir completamente no silêncio digital.
        let value = (rms + peak * 0.35).min(1.0);
        result.push(value.max(0.004));
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
            interleaved: vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.0, 0.0, 0.0],
            sample_rate: 44100,
            channels: 1,
            duration_seconds: 0.0,
        };
        let pts = waveform_points(&data, 4);
        assert_eq!(pts.len(), 4);
        // Chunks de 2: [0,0.5]→RMS+peak, [-0.5,1]→..., etc.
        // RMS + 35% do pico: chunk denso fica mais alto que
        // chunk esparso com o mesmo pico.
        let expected = |chunk: &[f32]| -> f32 {
            let peak = chunk.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            let rms = (chunk.iter().map(|&s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            ((rms + peak * 0.35).min(1.0)).max(0.004)
        };
        assert_eq!(pts[0], expected(&data.samples[0..2]));
        assert_eq!(pts[1], expected(&data.samples[2..4]));
        assert_eq!(pts[2], expected(&data.samples[4..6]));
        assert_eq!(pts[3], expected(&data.samples[6..8]));
    }

    #[test]
    fn waveform_respects_file_level() {
        // Arquivo alto vs arquivo baixo com o MESMO formato: o
        // desenho do baixo tem que ser visivelmente mais baixo —
        // nada de normalizar por arquivo.
        let loud = WaveformData {
            samples: vec![1.0; 100],
            interleaved: vec![1.0; 100],
            sample_rate: 44100,
            channels: 1,
            duration_seconds: 0.0,
        };
        let quiet = WaveformData {
            samples: vec![0.2; 100],
            interleaved: vec![0.2; 100],
            sample_rate: 44100,
            channels: 1,
            duration_seconds: 0.0,
        };
        let loud_pts = waveform_points(&loud, 10);
        let quiet_pts = waveform_points(&quiet, 10);
        assert!(loud_pts.iter().all(|&p| (p - 1.0).abs() < 1e-6));
        assert!(quiet_pts.iter().all(|&p| (p - 0.27).abs() < 1e-6));
        assert!(loud_pts[0] > quiet_pts[0] * 3.0);
    }

    #[test]
    fn stereo_keeps_channels_separate() {
        // L = 1.0 constante, R = -1.0 constante: o downmix mono
        // cancela (0.0), mas os canais originais sobrevivem.
        let data = WaveformData {
            samples: downmix_mono(&[1.0, -1.0, 1.0, -1.0], 2),
            interleaved: vec![1.0, -1.0, 1.0, -1.0],
            sample_rate: 48000,
            channels: 2,
            duration_seconds: 0.0,
        };
        assert_eq!(data.samples, vec![0.0, 0.0]);
        assert_eq!(data.frames(), 2);
        assert_eq!(data.channel(0), vec![1.0, 1.0]);
        assert_eq!(data.channel(1), vec![-1.0, -1.0]);
    }

    #[test]
    fn mono_channel_duplicates() {
        let data = WaveformData {
            samples: vec![0.25, 0.5],
            interleaved: vec![0.25, 0.5],
            sample_rate: 48000,
            channels: 1,
            duration_seconds: 0.0,
        };
        assert_eq!(data.channel(0), vec![0.25, 0.5]);
        assert_eq!(data.channel(1), vec![0.25, 0.5]);
    }
}
