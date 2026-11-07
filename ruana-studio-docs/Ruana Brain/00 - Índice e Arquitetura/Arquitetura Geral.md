# Arquitetura Geral

> Visão de cima do Ruana Studio: uma DAW desktop em Rust + Slint + wgpu.

---

## Stack tecnológica (verificado nos manifests)

| Camada | Tecnologia | Versão | Manifest |
|---|---|---|---|
| Linguagem | Rust | edition 2024 | `Cargo.toml` workspace |
| UI declarativa | Slint | 1.18.0 | `Cargo.toml` workspace.dependencies |
| Renderer GPU | wgpu | 30.0.x | `app/Cargo.toml`, `timeline-render/Cargo.toml` |
| Janelas/eventos | winit | 0.30 | `app/Cargo.toml` |
| Backend Slint | i-slint-backend-winit + Skia | 1.18.0 | `app/Cargo.toml` |
| Áudio I/O | cpal | 0.18 | `crates/daw-audio/Cargo.toml` |
| Decodificação | symphonia | 0.6 | `crates/daw-media/Cargo.toml` |
| Time-stretch | timestretch | 0.15.0 | `crates/daw-audio/Cargo.toml` |
| Formato WAV | hound | 3.5 | `crates/daw-audio`, `crates/daw-media` |

## Arquitetura em camadas

```
┌─────────────────────────────────────────────────┐
│  UI Layer (Slint .slint)                        │
│  main.slint, timeline, mixer, piano roll, etc.  │
└──────────────────────┬──────────────────────────┘
                       │ callbacks / properties
┌──────────────────────▼──────────────────────────┐
│  App Layer (app/src/)                           │
│  host.rs, playback.rs, waveform.rs,             │
│  timeline_canvas.rs, hit.rs                     │
└───────┬───────────────────────┬─────────────────┘
        │                       │
┌───────▼────────┐    ┌─────────▼──────────────┐
│ wgpu renderer  │    │ daw-audio engine        │
│ (timeline GPU) │    │ (cpal + timestretch)    │
└───────┬────────┘    └─────────┬──────────────┘
        │                       │
┌───────▼───────────────────────▼─────────────────┐
│  daw-core (modelos: clip, track, project)       │
│  daw-midi (note, pattern, sequencer)            │
│  daw-media (decodificação via symphonia)        │
│  daw-project (persistência)                     │
│  daw-plugins (extensibilidade)                  │
└─────────────────────────────────────────────────┘
```

## Fluxo de áudio

1. `daw-media` decodifica arquivos (WAV/MP3/FLAC/OGG) via `symphonia` para PCM `f32`
2. `daw-audio` gerencia o stream `cpal` (output callback em thread de alta prioridade)
3. Clips musicais passam por `timestretch` para varispeed/keylock
4. O callback de áudio mistura tracks → envia ao dispositivo
5. `timeline-render` (wgpu) renderiza waveform + playhead sincronizado

## Fluxo de renderização

1. `winit` captura eventos de janela e input
2. O loop principal despacha para o renderer wgpu e para o Slint
3. wgpu renderiza timeline, clips, playhead no framebuffer
4. Slint renderiza UI por cima (overlay)
5. `surface.present()` apresenta o frame

## Decisões arquiteturais

- **Timeline renderizada por wgpu, não Slint**: latência zero no playhead, shaders customizados para waveform
- **Slint como overlay**: botões, knobs, painéis flutuando sobre a timeline
- **Áudio em thread separada**: callback cpal nunca bloqueia com UI ou I/O
- **Crates separados por responsabilidade**: core (modelos), media (decodificação), audio (engine), midi (sequencer), project (persistência)

---

## Links

- [[Workspace Rust e Crates]]
- [[Referência wgpu 30]]
- [[Referência Slint 1.18]]
- [[Referência cpal 0.18]]
- [[wgpu Timeline Rendering]]
