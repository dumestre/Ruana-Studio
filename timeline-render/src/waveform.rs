//! Waveforms procedurais de teste (placeholder com aparência real).
//!
//! Gera uma amostra de amplitude por coluna — o "osciloscópio" da timeline.
//! O shader liga as amostras com segmentos de reta (1 coluna = 1 vértice),
//! então mais colunas = linha mais suave e nítida. Sem dependências externas:
//! cada amostra vem de um hash determinístico, estável entre execuções mas
//! com textura de áudio de verdade (ataque, sustentação, reverberação de
//! caixa, variação de dinâmica).

use crate::clips::ClipDemo;

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
pub fn build_wave_bars(clips: &[ClipDemo], px_per_beat: f32) -> Vec<f32> {
    let mut bars = Vec::new();
    let mut audio_idx = 0u64;
    for clip in clips {
        if clip.is_midi {
            continue;
        }
        let n_cols = wave_cols(clip.len, px_per_beat);
        let style = (audio_idx % 3) as usize;
        let seed = audio_idx.wrapping_mul(0x9E3779B97F4A7C15)
            ^ (clip.beat as u32 as u64).wrapping_mul(11)
            ^ (clip.track_index as u64).wrapping_mul(0xD1B54A32D192ED03);
        let seg: Vec<f32> = (0..n_cols)
            .map(|col| {
                // Seed é constante por clip: o oscilador é uma função
                // contínua de `col` (frac), sem fase mudando por coluna.
                sample_wave(style, col, n_cols, seed) * gain(col as f32 / n_cols as f32)
            })
            .collect();
        bars.extend_from_slice(&seg);
        audio_idx += 1;
    }
    bars
}