//! Carregamento de WAV em runtime para o laboratório.
//!
//! Lê qualquer WAV do diretório `assests/` na inicialização e
//! expõe como TestSignal igual aos sintéticos.

use crate::synth::TestSignal;
use std::path::Path;

/// Decodifica um WAV (PCM 8/16/24/32-bit ou float) para f32 L/R.
pub fn load_wav(path: &Path) -> Option<TestSignal> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let sr = spec.sample_rate as f32;

    let max_val = (1i32 << (spec.bits_per_sample.saturating_sub(1))) as f32;
    let mut samples: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                samples.push(s.ok()?);
            }
        }
        hound::SampleFormat::Int => {
            for s in reader.samples::<i32>() {
                samples.push(s.ok()? as f32 / max_val);
            }
        }
    }

    let ch = spec.channels.max(1) as usize;
    if ch == 1 {
        // Mono: duplica nos dois lados.
        return Some(TestSignal {
            name: "wav",
            label_name: "wav".into(),
            left: samples.clone(),
            right: samples,
            sample_rate: sr,
        });
    }
    let mut left = Vec::with_capacity(samples.len() / ch);
    let mut right = Vec::with_capacity(samples.len() / ch);
    for frame in samples.chunks(ch) {
        left.push(frame[0]);
        right.push(frame[1.min(ch - 1)]);
    }
    Some(TestSignal { name: "wav", label_name: "wav".into(), left, right, sample_rate: sr })
}

/// Escaneia o diretório de assets e carrega todos os WAV.
pub fn load_assets(dir: &Path) -> Vec<TestSignal> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("wav"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for p in paths {
            if let Some(sig) = load_wav(&p) {
                let label = p
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("wav")
                    .to_string();
                out.push(TestSignal {
                    name: "wav",
                    label_name: format!("[wav] {label}"),
                    ..sig
                });
            }
        }
    }
    out
}
