# Referência timestretch 0.15

> Referência do timestretch usado no RuanaStudio (0.15.0).

---

## Fontes oficiais

- **timestretch docs**: https://docs.rs/timestretch/
- **timestretch GitHub**: https://github.com/robmorgan/timestretch-rs

## Visão geral

Biblioteca de time-stretching de áudio pura-Rust otimizada para EDM. Estica áudio no tempo sem alterar o pitch, construída em torno de um motor real-time-first: eixo de tempo varispeed com keylock de dois bandas no domínio do tempo, dirigido como um deck de DJ.

Batch stretching roda o mesmo grafo do motor offline. Única dependência DSP externa é `rustfft`.

## Features

- **Motor real-time** — callback de áudio recebe exatamente os frames que precisa (`EngineProcessor::process`): infalível, sem alocações, sem lock na thread de áudio, com latência constante por perfil (12.7ms keylock, 0ms wide-range Master Tempo e tape)
- **Varispeed** — resampler sinc com keylock de dois bandas
- **Keylock** — CDJ-style full-spectrum keylock em rates 0.25–2.0
- **Warm-start seek/cue** (`controller.warm_start`), gapless loop wraps (`set_track_position`), pre-analysis artifacts (`EngineConfig::pre_analysis`)
- **WAV I/O** — reader/writer para 16-bit, 24-bit e 32-bit float
- **Safe Rust** — `#![forbid(unsafe_code)]`, sem panics no código da biblioteca

## Quick Start

```rust
use timestretch::StretchParams;

let params = StretchParams::new(1.5)  // 1.5x mais lento
    .with_sample_rate(44100)
    .with_channels(1);

let output = timestretch::stretch(&input, &params).unwrap();
assert!(output.len() > input.len());
```

## Motor real-time

```rust
use timestretch::{Engine, EngineConfig, EngineProfile};

let engine = Engine::build(EngineConfig {
    profile: EngineProfile::KeyLock,
    ..Default::default()
}).unwrap();

let handles = engine.handles();
// handles.controller — controle (lock-free)
// handles.processor — processamento (thread de áudio)
// handles.source — feed de source

// No callback de áudio:
let frames_needed = 512;
let output = handles.processor.process(frames_needed);
```

## Perfis

| Perfil | Latência | Descrição |
|---|---|---|
| `KeyLock` | 12.7ms | Keylock de dois bandas (crossover 120Hz) |
| `WideRange` | 0ms | Master Tempo wide-range (0.25–2.0) |
| `Tape` | 0ms | Modo tape (varispeed puro) |

## API de stretching

```rust
// Stretch simples
pub fn stretch(input: &[f32], params: &StretchParams) -> Result<Vec<f32>, StretchError>

// Stretch em buffer existente (zero-copy)
pub fn stretch_into(input: &[f32], params: &StretchParams, output: &mut Vec<f32>)

// Stretch de AudioBuffer
pub fn stretch_buffer(buffer: &AudioBuffer, params: &StretchParams) -> AudioBuffer

// BPM-based
pub fn stretch_to_bpm(input: &[f32], params: &StretchParams, source_bpm: f64, target_bpm: f64) -> Result<Vec<f32>, StretchError>
pub fn stretch_to_bpm_auto(input: &[f32], params: &StretchParams, target_bpm: f64) -> Result<Vec<f32>, StretchError>
```

## Detecção de BPM

```rust
let bpm = timestretch::detect_bpm(&samples, 44100.0).unwrap();
let beat_grid = timestretch::detect_beat_grid(&samples, 44100.0).unwrap();
```

## Formato de áudio

- **Sample format**: `f32` (32-bit float, range -1.0 a 1.0)
- **Channel layout**: mono ou stereo (interleaved)
- **Sample rates**: qualquer rate padrão (44100, 48000, etc.)
- **WAV I/O**: 16-bit PCM, 24-bit PCM, 32-bit float

## Integração com o projeto

`daw-audio` usa `timestretch` para:
1. Clips musicais em modo varispeed (pitch preservado)
2. Keylock de dois bandas para mudanças de tempo
3. Detecção de BPM para beatmatching
4. Stretch offline para renderização

## Links

- [[Referência Rust]]
- [[Referência cpal 0.18]]
- [[Referência symphonia 0.6]]
- [[Arquitetura Geral]]
