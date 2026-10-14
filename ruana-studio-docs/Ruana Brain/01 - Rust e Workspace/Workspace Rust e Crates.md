# Workspace Rust e Crates

> Estrutura do Cargo workspace RuanaStudio (verificado em `Cargo.toml`).

---

## Workspace

```toml
[workspace]
resolver = "3"
members = [
    "app",
    "crates/daw-core",
    "crates/daw-audio",
    "crates/daw-media",
    "crates/daw-midi",
    "crates/daw-plugins",
    "crates/daw-project",
    "timeline-render",
    "ruana-hub",
]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
slint = "1.18.0"
```

## Crates

| Crate | Responsabilidade | Dependências principais |
|---|---|---|
| `app` | Binário principal, integra tudo | slint, winit, wgpu, todos os crates |
| `daw-core` | Modelos de dados (clip, track, project) | daw-midi |
| `daw-audio` | Engine de áudio, stream cpal | daw-core, daw-media, daw-midi, cpal, timestretch |
| `daw-media` | Decodificação de áudio | symphonia, hound, tiny-skia |
| `daw-midi` | Modelos MIDI (note, pattern) | — |
| `daw-plugins` | Sistema de plugins | — |
| `daw-project` | Persistência de projeto | — |
| `timeline-render` | Renderer wgpu standalone | wgpu, winit, ab_glyph, daw-core |
| `ruana-hub` | Hub de plugins/updates | slint, reqwest, serde, zip |

## Build profiles

```toml
[profile.dev]
opt-level = 0          # nosso código: compila rápido, debugável
debug = "line-tables-only"
incremental = true

[profile.dev.package."*"]
opt-level = 2          # dependências: rápidas em runtime

[profile.dev.package.daw-audio]
opt-level = 2

[profile.dev.package.daw-media]
opt-level = 2

[profile.release]
lto = "thin"
codegen-units = 16
debug = false
```

## Comandos essenciais

```bash
# Build do workspace
cargo build

# Build de crate específico
cargo build -p daw-audio
cargo build -p timeline-render

# Testes
cargo test -p daw-core
cargo test -p daw-audio

# Check sem compilar
cargo check

# Formatação
cargo fmt --check
```

## Links

- [[Arquitetura Geral]]
- [[Referência Rust]]
