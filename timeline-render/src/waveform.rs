//! Waveforms procedurais de teste (placeholder com aparência real).
//!
//! Gera uma amostra de amplitude por coluna — o "osciloscópio" da timeline.
//! O shader liga as amostras com segmentos de reta (1 coluna = 1 vértice),
//! então mais colunas = linha mais suave e nítida. Sem dependências externas:
//! cada amostra vem de um hash determinístico, estável entre execuções mas
//! com textura de áudio de verdade (ataque, sustentação, reverberação de
//! caixa, variação de dinâmica).

use crate::clips::ClipDemo;
use crate::fades::{ClipFade, eval_bezier_2d_in, eval_bezier_2d_out};
use daw_media::waveform::decode_audio;
use std::path::Path;

/// Número de colunas de waveform de um clip de áudio. Deve bater com o
/// mesmo cálculo usado na geração — 1 vértice por coluna.
pub fn wave_cols(len: f32, px_per_beat: f32) -> u32 {
    ((len * px_per_beat * 4.0).ceil() as u32).max(1).min(16384)
}

/// Hash determinístico 64-bit, sem dependências.
fn hash01(mut x: u64) -> f32 {
    x = x.wrapping_mul(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = x ^ (x >> 27);
    x = x.wrapping_mul(0x94D049BB133111EB);
    x = x ^ (x >> 31);
    (x >> 33) as f32 / (1u64 << 31) as f32
}

/// Envelope de ganho do clip: nunca zera (piso 35%) para a linha não ficar
/// reta/morta nas bordas e nas quedas — tipo um indicador RSI/MA contínuo.
fn gain(frac: f32) -> f32 {
    let fade_in = (frac / 0.06).min(1.0);
    let fade_out = ((1.0 - frac) / 0.08).min(1.0);
    0.35 + 0.65 * fade_in * fade_out
}

/// Baixo: 2 oscilações lentas superpostas — curva única e contínua.
fn sample_bass(i: u32, n: u32, seed: u64) -> f32 {
    let frac = i as f32 / n as f32;
    let t = frac * std::f32::consts::TAU;
    let phase = hash01(seed);
    (t * 2.0 + phase).sin() * 0.85 + (t * 4.3 + phase * 2.0).sin() * 0.15
}

/// Bateria: onda oscilante modulada em amplitude — pulsos largos, sem spikes.
fn sample_drums(i: u32, n: u32, seed: u64) -> f32 {
    let frac = i as f32 / n as f32;
    let t = frac * std::f32::consts::TAU;
    let phase = hash01(seed.wrapping_mul(3));
    let pulse = 0.5 + 0.5 * (t * 6.0 + phase).sin();
    (t * 3.0 + phase).sin() * (0.35 + 0.65 * pulse)
}

/// Pad: oscilação lenta com leve variação — linha aberta e constante.
fn sample_pads(i: u32, n: u32, seed: u64) -> f32 {
    let frac = i as f32 / n as f32;
    let t = frac * std::f32::consts::TAU;
    let phase = hash01(seed.wrapping_add(9));
    (t * 1.5 + phase).sin() * 0.7 + (t * 4.0 + phase * 3.0).sin() * 0.3
}

/// Estilo da waveform por índice de clip de áudio (3 estilos para teste).
fn sample_wave(style: usize, i: u32, n: u32, seed: u64) -> f32 {
    match style % 3 {
        0 => sample_bass(i, n, seed),
        1 => sample_drums(i, n, seed),
        _ => sample_pads(i, n, seed),
    }
}

/// Constrói todas as amplitudes (uma por coluna) dos clips de áudio,
/// concatenadas na mesma ordem em que os clips aparecem (os offsets de
/// `clips.rs` coincidem com a segmentação: cada clip é seu próprio trecho).
///
/// Além do buffer concatenado, devolve a segmentação (offset, count) por id
/// de clip e ARMAZENA as amostras em `clip.wave` — a fonte da verdade por
/// clip. Um slice futura divide `clip.wave` ao meio (corte verdadeiro, sem
/// regenerar as peças).
pub fn build_wave_bars_and_segments(
    clips: &mut [ClipDemo],
    px_per_beat: f32,
) -> (Vec<f32>, Vec<(u32, u32)>) {
    let mut bars = Vec::new();
    let mut segments = Vec::with_capacity(clips.len());
    let mut audio_idx = 0u64;
    for clip in clips.iter_mut() {
        if clip.is_midi {
            segments.push((0u32, 0u32));
            clip.wave = Vec::new();
            continue;
        }

        let n_cols = wave_cols(clip.len, px_per_beat);
        let wave: Vec<f32> = if let Some(ref path) = clip.file_path {
            match decode_real_wave(path, n_cols) {
                Ok(w) => w,
                Err(_) => generate_procedural_wave(clip, n_cols, audio_idx),
            }
        } else {
            let style = (audio_idx % 3) as usize;
            let seed = audio_idx.wrapping_mul(0x9E3779B97F4A7C15)
                ^ (clip.beat as u32 as u64).wrapping_mul(11)
                ^ (clip.track_index as u64).wrapping_mul(0xD1B54A32D192ED03);
            (0..n_cols)
                .map(|col| {
                    sample_wave(style, col, n_cols, seed) * gain(col as f32 / n_cols as f32)
                })
                .collect()
        };

        let offset = bars.len() as u32;
        bars.extend_from_slice(&wave);
        segments.push((offset, wave.len() as u32));
        clip.wave = wave;
        audio_idx += 1;
    }
    (bars, segments)
}

/// Decodifica um arquivo de áudio real e converte para amplitudes por coluna.
fn decode_real_wave(path: &str, n_cols: u32) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let data = decode_audio(Path::new(path))?;
    let samples = &data.samples;
    if samples.is_empty() || n_cols == 0 {
        return Ok(vec![0.0; n_cols as usize]);
    }

    let mut wave = Vec::with_capacity(n_cols as usize);
    let total = samples.len();
    let cols = n_cols as usize;

    for col in 0..cols {
        let start = (col * total / cols).min(total.saturating_sub(1));
        let end = ((col + 1) * total / cols).min(total);
        if start >= end {
            wave.push(0.0);
            continue;
        }
        let peak = samples[start..end]
            .iter()
            .map(|&s| s.abs())
            .fold(0.0f32, f32::max);
        wave.push(peak);
    }

    Ok(wave)
}

/// Gera waveform procedural para um clip (fallback).
fn generate_procedural_wave(clip: &ClipDemo, n_cols: u32, seed: u64) -> Vec<f32> {
    let style = (seed % 3) as usize;
    let seed = seed.wrapping_mul(0x9E3779B97F4A7C15)
        ^ (clip.beat as u32 as u64).wrapping_mul(11)
        ^ (clip.track_index as u64).wrapping_mul(0xD1B54A32D192ED03);
    (0..n_cols)
        .map(|col| {
            sample_wave(style, col, n_cols, seed) * gain(col as f32 / n_cols as f32)
        })
        .collect()
}

/// Concatena as ondas ARMAZENADAS nos clips (fonte da verdade, campo `wave`)
/// num buffer único com a segmentação (offset, count) por id. Não regenera
/// nada: cortes, glues, erases etc. refletem as ondas persistidas.
pub fn concat_wave_data(clips: &[ClipDemo]) -> (Vec<f32>, Vec<(u32, u32)>) {
    let mut bars = Vec::new();
    let mut segments = Vec::with_capacity(clips.len());
    for clip in clips {
        if clip.is_midi || clip.wave.is_empty() {
            segments.push((0u32, 0u32));
            continue;
        }
        segments.push((bars.len() as u32, clip.wave.len() as u32));
        bars.extend_from_slice(&clip.wave);
    }
    (bars, segments)
}

/// Aplica o envelope de fade in/out e ganho às amostras da waveform.
/// Cada sample é multiplicado pelo envelope correspondente à sua posição
/// dentro do clip, fazendo com que a waveform visual reflita os fades.
pub fn apply_fade_envelope(
    bars: Vec<f32>,
    segments: &[(u32, u32)],
    clips: &[ClipDemo],
    fades: &[ClipFade],
) -> Vec<f32> {
    let mut out = bars;

    for (i, clip) in clips.iter().enumerate() {
        if clip.is_midi || clip.wave.is_empty() {
            continue;
        }
        if i >= fades.len() {
            continue;
        }
        let fade = &fades[i];
        let (offset, count) = segments[i];
        let n = count as usize;
        if n == 0 {
            continue;
        }

        let clip_len = clip.wave.len() as f32 / 160.0;
        if clip_len <= 0.0 {
            continue;
        }

        let gain_linear = fade.gain_linear();

        let start = offset as usize;
        let end = start + n;

        for (j, sample) in out[start..end].iter_mut().enumerate() {
            let frac = if n > 1 { j as f32 / (n - 1) as f32 } else { 0.0 };
            let beat_pos = frac * clip_len;

            let env_in = if fade.in_len > 0.0 && beat_pos < fade.in_len {
                let t = (beat_pos / fade.in_len).clamp(0.0, 1.0);
                eval_bezier_2d_in(t, fade.in_curve_x, fade.in_curve_y)
            } else {
                1.0
            };

            let env_out = if fade.out_len > 0.0 && beat_pos > clip_len - fade.out_len {
                let t = ((beat_pos - (clip_len - fade.out_len)) / fade.out_len).clamp(0.0, 1.0);
                eval_bezier_2d_out(t, fade.out_curve_x, fade.out_curve_y)
            } else {
                1.0
            };

            let envelope = env_in * env_out * gain_linear;
            *sample = *sample * envelope;
        }
    }

    out
}