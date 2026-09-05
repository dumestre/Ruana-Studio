//! Waveforms procedurais de teste (placeholder com aparência real).
//!
//! Gera os pares (min, max) por coluna — o mesmo layout consumido pelo
//! shader via `wave_bars`. Sem dependências externas: cada coluna vira um
//! pico determinístico baseado num hash inteiro, então o visual é estável
//! entre execuções mas com textura de áudio de verdade (ataque, sustentação,
//! reverberação de caixa, variação de dinâmica).

use crate::clips::ClipDemo;

/// Hash determinístico 64-bit -> [0,1), sem dependências.
fn hash01(mut x: u64) -> f32 {
    x = x.wrapping_mul(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = x ^ (x >> 27);
    x = x.wrapping_mul(0x94D049BB133111EB);
    x = x ^ (x >> 31);
    (x >> 33) as f32 / (1u64 << 31) as f32
}

/// Envelope de ganho do clip: fade-in curto, corpo estável, fade-out final.
fn envelope(frac: f32) -> f32 {
    let fade_in = (frac / 0.06).min(1.0);
    let fade_out = ((1.0 - frac) / 0.08).min(1.0);
    fade_in * fade_out
}

/// True peak de uma coluna (assimétrico de propósito): devolve (min, max).
fn peaks(v: f32, dc: f32) -> (f32, f32) {
    let mn = (v + dc).min(0.0);
    let mx = (v + dc).max(0.0);
    (mn, mx)
}

/// Baixo: onda de poucas frequências, sustentada, bem contínua.
fn sample_bass(i: u32, n: u32, seed: u64) -> (f32, f32) {
    let frac = i as f32 / n as f32;
    let t = frac * std::f32::consts::TAU;
    let lfo = 0.8 + 0.2 * (t * 4.0).sin();
    let s = (t * 2.0).sin() * 0.75
        + (t * 5.0 + hash01(seed) * std::f32::consts::TAU).sin() * 0.45
        + (hash01(seed.wrapping_add(i as u64 * 7)) - 0.5) * 0.10;
    let v = s * lfo * envelope(frac);
    peaks(v, 0.02 * s)
}

/// Bateria: ataques curtos e regulares, decaimento rápido, muito silêncio.
fn sample_drums(i: u32, n: u32, seed: u64) -> (f32, f32) {
    let beats = 16.0;
    let spb = (n as f32 / beats).max(1.0);
    let beat_idx = (i as f32 / spb) as u32;
    let within = (i as f32 - beat_idx as f32 * spb) / spb;
    let decay = (-within * 22.0).exp();
    let variation = 0.35 + 0.65 * hash01(seed ^ (beat_idx as u64).wrapping_mul(0x517CC1B727220A95));
    let v = decay * variation * envelope(i as f32 / n as f32);
    peaks(v, v * 0.15)
}

/// Pad: sustentado, largo, com shimmer de alta frequência por cima.
fn sample_pads(i: u32, n: u32, seed: u64) -> (f32, f32) {
    let frac = i as f32 / n as f32;
    let t = frac * std::f32::consts::TAU;
    let vib = (t * 3.0 + hash01(seed.wrapping_mul(3)) * std::f32::consts::TAU).sin() * 0.55
        + (t * 11.0 + hash01(seed.wrapping_add(9)) * std::f32::consts::TAU).sin() * 0.25;
    let shimmer = (hash01(seed ^ (i as u64).wrapping_mul(0x517CC1B727220A95)) - 0.5) * 0.10;
    let v = (vib + shimmer) * envelope(frac);
    peaks(v, 0.0)
}

/// Estilo da waveform por índice de clip de áudio (3 estilos para teste).
fn sample_wave(style: usize, i: u32, n: u32, seed: u64) -> (f32, f32) {
    match style % 3 {
        0 => sample_bass(i, n, seed),
        1 => sample_drums(i, n, seed),
        _ => sample_pads(i, n, seed),
    }
}

/// Constrói todos os pares (min, max) das waveforms dos clips de áudio,
/// na ordem em que o shader espera (mesmos offsets de `clips.rs`).
pub fn build_wave_bars(clips: &[ClipDemo], px_per_beat: f32) -> Vec<f32> {
    let mut bars = Vec::new();
    let mut audio_idx = 0u64;
    for clip in clips {
        if clip.is_midi {
            continue;
        }
        let n_cols = ((clip.len * px_per_beat).ceil() as u32).max(1).min(256);
        let style = (audio_idx % 3) as usize;
        let seed = audio_idx.wrapping_mul(0x9E3779B97F4A7C15)
            ^ (clip.beat as u32 as u64).wrapping_mul(11)
            ^ (clip.track_index as u64).wrapping_mul(0xD1B54A32D192ED03);
        for col in 0..n_cols {
            let (mn, mx) = sample_wave(style, col, n_cols, seed ^ (col as u64).wrapping_mul(0x9E3779B97F4A7C15));
            bars.push(mn);
            bars.push(mx);
        }
        audio_idx += 1;
    }
    bars
}