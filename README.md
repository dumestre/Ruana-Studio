# Ruana Studio

Digital Audio Workstation (DAW) desenvolvida em Rust com interface Slint.

## Sobre

Ruana Studio é uma estação de trabalho de áudio digital com timeline, mixer, plugins e suporte a MIDI/áudio. Construída para performance com renderização GPU e engine de áudio em Rust.

## Screenshots

![Screenshot 1](docs/screenshots/screenshot-1.png)
![Screenshot 2](docs/screenshots/screenshot-2.png)
![Screenshot 3](docs/screenshots/screenshot-3.png)
![Screenshot 4](docs/screenshots/screenshot-4.png)
![Screenshot 5](docs/screenshots/screenshot-5.png)
![Screenshot 6](docs/screenshots/screenshot-6.png)

## Funcionalidades

- Timeline de áudio/MIDI com zoom e scroll
- Mixer com canais e efeitos
- Suporte a plugins
- Renderização de timeline acelerada por GPU
- Projeto salvo em formato próprio

## Build

```bash
cargo build --release -p ruana-studio
```

## Execução

```bash
cargo run -p ruana-studio
```

No Windows (binário já compilado):

```powershell
& 'target\debug\ruana-studio.exe' -p ruana-studio
```

## Estrutura

- `app/` — aplicação principal (UI Slint)
- `crates/daw-core` — núcleo do DAW
- `crates/daw-audio` — engine de áudio
- `crates/daw-midi` — MIDI
- `crates/daw-media` — mídia
- `crates/daw-plugins` — plugins
- `crates/daw-project` — formato de projeto
- `timeline-render/` — renderer da timeline
- `ruana-hub/` — hub

## Contribuidores

<!-- Adicione seu nome abaixo -->
- 

## Licença

A definir.
