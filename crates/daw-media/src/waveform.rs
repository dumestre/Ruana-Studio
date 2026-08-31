//! Waveform decoding e preparação para rendering de arquivos de áudio.
//!
//! Pipeline:
//!
//! ```text
//! arquivo
//!    ↓
//! decoder
//!    ↓
//! interleaved original ──────────────→ reprodução
//!    ↓
//! downmix mono
//!    ↓
//! display_samples
//!    ↓
//! Mipmap
//!    ↓
//! renderer da waveform
//! ```
//!
//! `samples` mantém o mono completo.
//! `display_samples` é uma representação reduzida somente para UI.
//! `mipmap` é construída uma vez e reutilizada durante o zoom.

use hound;
use std::path::Path;

use crate::waveform_image::{build_display_wave, Mipmap};

#[derive(Debug, Clone)]
pub struct WaveformData {
    /// Downmix mono COMPLETO.
    ///
    /// Usado como fonte da waveform e mantém todos os frames
    /// disponíveis para processamento visual.
    ///
    /// NÃO usar para reprodução.
    pub samples: Vec<f32>,

    /// Representação reduzida usada pela UI.
    ///
    /// Arquivos menores permanecem intactos.
    /// Arquivos muito grandes são reduzidos preservando MIN/MAX,
    /// evitando perda de transientes.
    pub display_samples: Vec<f32>,

    /// Pirâmide MIN/MAX/RMS construída a partir de `display_samples`.
    ///
    /// É criada uma vez durante o decode e reutilizada em todos
    /// os níveis de zoom.
    pub mipmap: Mipmap,

    /// Samples no layout ORIGINAL do arquivo (intercalado).
    ///
    /// Esta é a fonte para o motor de áudio.
    ///
    /// NÃO modificar para fins de waveform.
    pub interleaved: Vec<f32>,

    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

impl WaveformData {
    /// Quantidade de frames.
    ///
    /// Um frame contém um sample por canal.
    pub fn frames(&self) -> usize {
        let ch = self.channels.max(1) as usize;

        self.interleaved.len() / ch
    }

    /// Extrai um canal do buffer intercalado original.
    ///
    /// Arquivos mono devolvem o mesmo conteúdo para qualquer
    /// canal pedido — comportamento útil para código que trabalha
    /// genericamente com L/R.
    pub fn channel(&self, index: usize) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;

        let index = index.min(ch - 1);

        if ch == 1 {
            return self.interleaved.clone();
        }

        self.interleaved
            .chunks(ch)
            .filter_map(|frame| {
                frame.get(index).copied()
            })
            .collect()
    }
}

// ============================================================
// WAV / HOUND
// ============================================================

/// Decodifica WAV usando hound.
///
/// É o caminho rápido para WAVs bem formados.
fn decode_wav_file(
    path: &Path,
) -> Result<WaveformData, Box<dyn std::error::Error>> {
    let mut reader =
        hound::WavReader::open(path)?;

    let spec = reader.spec();

    let interleaved: Vec<f32> =
        match spec.sample_format {
            hound::SampleFormat::Int => {
                let max_val =
                    (1i32
                        << (spec.bits_per_sample - 1))
                        as f32;

                reader
                    .samples::<i32>()
                    .map(|s| {
                        s.map(|v| {
                            v as f32 / max_val
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }

            hound::SampleFormat::Float => {
                reader
                    .samples::<f32>()
                    .collect::<Result<Vec<_>, _>>()?
            }
        };

    let channels = spec.channels as u16;
    let sample_rate = spec.sample_rate;

    // ========================================================
    // MONO PARA WAVEFORM
    // ========================================================

    let samples =
        downmix_mono(
            &interleaved,
            channels as usize,
        );

    // ========================================================
    // REPRESENTAÇÃO DE DISPLAY + MIPMAP
    // ========================================================

    let (display_samples, mipmap) =
        build_display_wave(&samples);

    // ========================================================
    // DURAÇÃO
    // ========================================================

    let duration_seconds =
        samples.len() as f32
            / sample_rate as f32;

    Ok(WaveformData {
        samples,
        display_samples,
        mipmap,
        interleaved,
        sample_rate,
        channels,
        duration_seconds,
    })
}

// ============================================================
// DOWNMIX
// ============================================================

/// Downmix para mono — SOMENTE para o desenho da waveform.
///
/// Stereo:
///
/// ```text
/// mono = (L + R) / 2
/// ```
///
/// Multicanal:
///
/// ```text
/// usa o primeiro canal.
/// ```
///
/// A reprodução continua usando `interleaved`.
fn downmix_mono(
    interleaved: &[f32],
    channels: usize,
) -> Vec<f32> {
    match channels {
        0 | 1 => {
            interleaved.to_vec()
        }

        2 => {
            interleaved
                .chunks(2)
                .map(|c| {
                    if c.len() == 2 {
                        (c[0] + c[1]) * 0.5
                    } else {
                        c[0]
                    }
                })
                .collect()
        }

        n => {
            interleaved
                .chunks(n)
                .filter_map(|c| c.first().copied())
                .collect()
        }
    }
}

// ============================================================
// SYMPHONIA
// ============================================================

/// Decodifica MP3/FLAC/OGG/WAV via Symphonia.
///
/// Também funciona como fallback para WAVs rejeitados pelo hound.
fn decode_symphonia(
    path: &Path,
) -> Result<WaveformData, Box<dyn std::error::Error>> {
    use symphonia::core::audio::{
        Channels,
        SampleBuffer,
    };
    use symphonia::core::codecs::{
        DecoderOptions,
        CODEC_TYPE_NULL,
    };
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let src =
        std::fs::File::open(path)?;

    let mss =
        MediaSourceStream::new(
            Box::new(src),
            Default::default(),
        );

    let mut hint = Hint::new();

    if let Some(ext) =
        path.extension().and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let meta_opts =
        MetadataOptions::default();

    let fmt_opts =
        FormatOptions::default();

    let probed =
        symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &fmt_opts,
                &meta_opts,
            )?;

    let mut format =
        probed.format;

    let track =
        format
            .tracks()
            .iter()
            .find(|t| {
                t.codec_params.codec
                    != CODEC_TYPE_NULL
            })
            .ok_or(
                "no supported audio track found",
            )?;

    let codec_params =
        &track.codec_params;

    let sample_rate =
        codec_params
            .sample_rate
            .unwrap_or(44100);

    let channels =
        codec_params
            .channels
            .unwrap_or(
                Channels::FRONT_LEFT,
            );

    let channel_count =
        channels.count();

    let mut decoder =
        symphonia::default::get_codecs()
            .make(
                codec_params,
                &DecoderOptions::default(),
            )?;

    let track_id = track.id;

    let mut sample_buf:
        Option<SampleBuffer<f32>> = None;

    let mut all_samples:
        Vec<f32> = Vec::new();

    // ========================================================
    // DECODER LOOP
    // ========================================================

    loop {
        let packet =
            match format.next_packet() {
                Ok(p) => p,

                Err(
                    symphonia::core::errors::Error::IoError(
                        ref e,
                    ),
                ) if e.kind()
                    == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }

                Err(_) => {
                    break;
                }
            };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let buf =
                    sample_buf
                        .get_or_insert_with(|| {
                            SampleBuffer::<f32>::new(
                                decoded.capacity() as u64,
                                *decoded.spec(),
                            )
                        });

                buf.copy_interleaved_ref(
                    decoded,
                );

                all_samples.extend_from_slice(
                    buf.samples(),
                );
            }

            Err(_) => {
                continue;
            }
        }
    }

    // ========================================================
    // MONO PARA WAVEFORM
    // ========================================================

    let samples =
        downmix_mono(
            &all_samples,
            channel_count,
        );

    // ========================================================
    // DISPLAY + MIPMAP
    // ========================================================

    let (display_samples, mipmap) =
        build_display_wave(&samples);

    // ========================================================
    // DURAÇÃO
    // ========================================================

    let duration_seconds =
        samples.len() as f32
            / sample_rate as f32;

    Ok(WaveformData {
        samples,
        display_samples,
        mipmap,
        interleaved: all_samples,
        sample_rate,
        channels: channel_count as u16,
        duration_seconds,
    })
}

// ============================================================
// DECODER PÚBLICO
// ============================================================

/// Decodifica um arquivo de áudio.
///
/// Suportados:
///
/// - WAV
/// - MP3
/// - FLAC
/// - OGG
/// - OGA
///
/// WAV:
///     hound primeiro
///     ↓
///     Symphonia como fallback
///
/// MP3/FLAC/OGG:
///     Symphonia
pub fn decode_audio(
    path: impl AsRef<Path>,
) -> Result<
    WaveformData,
    Box<dyn std::error::Error>,
> {
    let path = path.as_ref();

    let ext =
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_uppercase();

    match ext.as_str() {
        "WAV" => {
            decode_wav_file(path)
                .or_else(|_| {
                    decode_symphonia(path)
                })
        }

        "MP3"
        | "FLAC"
        | "OGG"
        | "OGA" => {
            decode_symphonia(path)
        }

        _ => {
            Err(
                format!(
                    "unsupported audio format: {}",
                    ext
                )
                .into(),
            )
        }
    }
}

// ============================================================
// COMPATIBILIDADE — WAVEFORM POINTS
// ============================================================

/// Gera pontos de waveform para APIs antigas.
///
/// Esta função é mantida para compatibilidade.
///
/// O renderer novo deve usar:
///
/// ```text
/// data.display_samples
/// data.mipmap
/// waveform_image_rgba_mip()
/// ```
pub fn waveform_points(
    data: &WaveformData,
    points: usize,
) -> Vec<f32> {
    if data.samples.is_empty()
        || points == 0
    {
        return vec![0.0; points];
    }

    let chunk_size =
        (data.samples.len() as f32
            / points as f32)
            .max(1.0) as usize;

    let mut result =
        Vec::with_capacity(points);

    for i in 0..points {
        let start =
            i * chunk_size;

        let end =
            ((i + 1) * chunk_size)
                .min(data.samples.len());

        if start >= data.samples.len() {
            result.push(0.0);
            continue;
        }

        let chunk =
            &data.samples[start..end];

        // Pico absoluto.
        let peak =
            chunk
                .iter()
                .fold(
                    0.0f32,
                    |acc, &s| {
                        acc.max(s.abs())
                    },
                );

        // RMS.
        let sum_sq: f32 =
            chunk
                .iter()
                .map(|&s| s * s)
                .sum();

        let rms =
            (sum_sq
                / chunk.len() as f32)
                .sqrt();

        // RMS + 25% do pico residual.
        let value =
            (rms
                + (peak - rms) * 0.25)
                .min(1.0);

        result.push(
            value.max(0.002),
        );
    }

    result
}

// ============================================================
// TESTES
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(
        samples: Vec<f32>,
        channels: u16,
    ) -> WaveformData {
        let interleaved =
            samples.clone();

        let (display_samples, mipmap) =
            build_display_wave(&samples);

        WaveformData {
            samples,
            display_samples,
            mipmap,
            interleaved,
            sample_rate: 44100,
            channels,
            duration_seconds: 0.0,
        }
    }

    #[test]
    fn waveform_downsample() {
        let data = make_data(
            vec![
                0.0,
                0.5,
                -0.5,
                1.0,
                -1.0,
                0.0,
                0.0,
                0.0,
            ],
            1,
        );

        let pts =
            waveform_points(
                &data,
                4,
            );

        assert_eq!(
            pts.len(),
            4
        );

        let expected =
            |chunk: &[f32]| -> f32 {
                let peak =
                    chunk
                        .iter()
                        .fold(
                            0.0f32,
                            |a, &s| {
                                a.max(s.abs())
                            },
                        );

                let rms =
                    (
                        chunk
                            .iter()
                            .map(|&s| s * s)
                            .sum::<f32>()
                            / chunk.len() as f32
                    )
                    .sqrt();

                (
                    rms
                        + (peak - rms) * 0.25
                )
                .min(1.0)
                .max(0.002)
            };

        assert_eq!(
            pts[0],
            expected(
                &data.samples[0..2]
            )
        );

        assert_eq!(
            pts[1],
            expected(
                &data.samples[2..4]
            )
        );

        assert_eq!(
            pts[2],
            expected(
                &data.samples[4..6]
            )
        );

        assert_eq!(
            pts[3],
            expected(
                &data.samples[6..8]
            )
        );
    }

    #[test]
    fn waveform_respects_file_level() {
        let loud =
            make_data(
                vec![1.0; 100],
                1,
            );

        let quiet =
            make_data(
                vec![0.2; 100],
                1,
            );

        let loud_pts =
            waveform_points(
                &loud,
                10,
            );

        let quiet_pts =
            waveform_points(
                &quiet,
                10,
            );

        assert!(
            loud_pts
                .iter()
                .all(|&p| {
                    (p - 1.0).abs()
                        < 1e-6
                })
        );

        assert!(
            quiet_pts
                .iter()
                .all(|&p| {
                    (p - 0.2).abs()
                        < 1e-6
                })
        );

        assert!(
            loud_pts[0]
                > quiet_pts[0] * 3.0
        );
    }

    #[test]
    fn stereo_keeps_channels_separate() {
        let mono =
            downmix_mono(
                &[
                    1.0,
                    -1.0,
                    1.0,
                    -1.0,
                ],
                2,
            );

        let mut data =
            make_data(
                mono,
                2,
            );

        data.interleaved =
            vec![
                1.0,
                -1.0,
                1.0,
                -1.0,
            ];

        data_clone_channel_test(
            &data,
        );
    }

    fn data_clone_channel_test(
        data: &WaveformData,
    ) {
        assert_eq!(
            data.samples,
            vec![0.0, 0.0]
        );

        assert_eq!(
            data.frames(),
            2
        );

        assert_eq!(
            data.channel(0),
            vec![1.0, 1.0]
        );

        assert_eq!(
            data.channel(1),
            vec![-1.0, -1.0]
        );
    }

    #[test]
    fn mono_channel_duplicates() {
        let data =
            make_data(
                vec![
                    0.25,
                    0.5,
                ],
                1,
            );

        assert_eq!(
            data.channel(0),
            vec![0.25, 0.5]
        );

        assert_eq!(
            data.channel(1),
            vec![0.25, 0.5]
        );
    }

    #[test]
    fn display_wave_has_mipmap() {
        let samples: Vec<f32> =
            (0..10000)
                .map(|i| {
                    (i as f32 * 0.01)
                        .sin()
                })
                .collect();

        let data =
            make_data(
                samples,
                1,
            );

        assert!(
            !data.display_samples.is_empty()
        );

        assert!(
            !data.mipmap.levels.is_empty()
        );
    }

    #[test]
    fn samples_remain_complete() {
        let samples: Vec<f32> =
            (0..5000)
                .map(|i| i as f32)
                .collect();

        let data =
            make_data(
                samples.clone(),
                1,
            );

        assert_eq!(
            data.samples.len(),
            samples.len()
        );

        assert_eq!(
            data.interleaved.len(),
            samples.len()
        );
    }

    #[test]
    fn wav_with_broken_block_align_decodes() {
        use std::io::Write;

        let dir =
            std::env::temp_dir()
                .join(
                    "ruana_wav_test",
                );

        std::fs::create_dir_all(
            &dir,
        )
        .unwrap();

        let path =
            dir.join(
                "broken_align.wav",
            );

        let mut f =
            std::fs::File::create(
                &path,
            )
            .unwrap();

        // RIFF.
        f.write_all(b"RIFF")
            .unwrap();

        f.write_all(
            &44u32.to_le_bytes(),
        )
        .unwrap();

        f.write_all(b"WAVE")
            .unwrap();

        // fmt.
        f.write_all(b"fmt ")
            .unwrap();

        f.write_all(
            &16u32.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &1u16.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &1u16.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &44100u32.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &44100u32.to_le_bytes(),
        )
        .unwrap();

        // Deliberadamente errado.
        f.write_all(
            &1u16.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &16u16.to_le_bytes(),
        )
        .unwrap();

        // data.
        f.write_all(b"data")
            .unwrap();

        f.write_all(
            &4u32.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &0i16.to_le_bytes(),
        )
        .unwrap();

        f.write_all(
            &i16::MAX.to_le_bytes(),
        )
        .unwrap();

        drop(f);

        let data =
            decode_audio(&path)
                .expect(
                    "WAV com block_align quebrado deveria decodificar",
                );

        assert_eq!(
            data.frames(),
            2
        );

        assert_eq!(
            data.channels,
            1
        );

        assert_eq!(
            data.sample_rate,
            44100
        );

        assert!(
            !data.display_samples.is_empty()
        );

        assert!(
            !data.mipmap.levels.is_empty()
        );
    }
}