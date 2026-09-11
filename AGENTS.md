# AGENTS.md — RuanaStudio

Guia de contexto para agentes de IA trabalhando neste repositório. Leia antes
de editar código.

## Visão geral

DAW (digitador de áudio / estações de áudio) escrita em Rust. Workspace Cargo
mono-repo com o crate principal `ruana-studio` (`app/`) e uma coleção de crates
internos em `crates/`.

A **timeline real do produto é a do crate `timeline-render`** (janela winit
nativa + wgpu). A timeline em Slint (`ui/timeline/`, `clip*.slint`) está sendo
**descontinuada** — não manter nada novo nela.

## Estrutura do workspace

```
RuanaStudio/
├── app/                 # crate principal (ruana-studio), UI em Slint
│   └── src/
│       ├── main.rs
│       ├── timeline_gpu/    # ROTA DESCONTINUADA (render via slint::Image)
│       ├── waveform_gpu/
│       └── ... (explorer, mixer, piano_roll, sampler, playback, importer)
├── app-makepad/         # UI NOVA em Makepad (ruana-studio-makepad) — port de Slint→Makepad,
│   │                    #   fase 1: shell + transport + timeline nativa (timeline-render)
│   └── src/ (app.rs com live_design!, main.rs stub, lib.rs)
├── crates/
│   ├── daw-core/        # tipos centrais (Clip, Track, Project, Transport)
│   ├── daw-audio/       # motor de áudio (cpal opcional)
│   ├── daw-media/       # decodificação (symphonia), waveform
│   ├── daw-midi/        # notas/beat patterns
│   ├── daw-plugins/
│   └── daw-project/
├── timeline-render/     # A TIMELINE REAL (lib + bin, wgpu, janela winit)
├── ui/                  # arquivos .slint (timeline aqui é legado)
├── src/…                # (inexistente — o binário vive em app/src)
├── Cargo.toml           # workspace members
└── KEYMAP.md            # atalhos (documenta a UI legada em Slint)
```

## timeline-render (crítico)

É **lib + bin** ao mesmo tempo:

- `src/lib.rs` — declara os módulos, re-exports públicos
  (`RectInstance`, `OrthoCamera`, `palette`, `rgba`, …) e `pub fn run()`.
- `src/main.rs` — bootstrap mínimo (só chama `timeline_render::run()`).
- `src/app/mod.rs` — `struct App` (campos `pub`), construtor, métodos base,
  `current_cursor_pos` (Win32 `GetCursorPos`, usado durante drag de arquivo).
- `src/app/render.rs` — `build_instances` + `render`, paleta, fontes, texto.
- `src/app/interaction.rs` — mouse, scroll, ferramentas, clipboard, loop,
  edge-scroll.
- `src/app/audio.rs` — `load_audio_file`, `build_project`,
  `sync_audio_engine`.
- `src/app/window.rs` — `AppHandler` (winit) + `run()`.
- `src/clips/` — módulo de clips, subdividido:
  - `mod.rs` — re-exports públicos (`ClipDemo`, `ClipMetrics`,
    `clip_rect`, `ClipInteraction`, `DrawToolState`, `build_clip_instances`).
  - `data.rs` — tipos e geometria (`ClipDemo`, `ClipMetrics`, `clip_rect`).
  - `interaction.rs` — hover/arrasto/stretch/marquee/draw/glue.
  - `render.rs` — `build_clip_instances` e helpers de render.
- `src/fades.rs`, `src/waveform.rs`, `src/tools.rs`,
  `src/shctx.rs` — apoiam a renderização (resolvem itens comuns via
  re-exports do crate root).

Comportamentos importantes já implementados:
- Clips de áudio **importados** recebem `fades::ClipFade::none()` (fade zerado).
- Preview de drop de áudio segue o mouse em tempo real via polling de
  `GetCursorPos` no `RedrawRequested` (Windows não envia `CursorMoved` durante
  o drag de arquivo).
- `App::new(window: Arc<Window>)` é público — pronto para o `app/` abrir a
  timeline numa **janela nativa winit** em thread separada.

## Rota da timeline (decisão de arquitetura)

1. **Abandonada:** `app/src/timeline_gpu/wgpu.rs` → gera `slint::Image`
   (offscreen); e a timeline em `.slint`.
2. **Atual:** `timeline-render` é standalone; roda via `cargo run -p
   timeline-render`.
3. **Próximo passo (roadmap):** integrar `timeline-render` como dependência do
   `app`, abrindo a janela winit numa **thread dedicada** (opção A: janela
   winit da timeline + janela do app, em threads separadas), compartilhando o
   `Transport` (`Arc<Mutex<Transport>>`). Depois remover a timeline Slint legada.

> ⚠️ Não reutilizar a lógica de `timeline_gpu` (slint::Image). O alvo é janela
> nativa.

## Port Slint → Makepad (em andamento)

- `app-makepad/` é a UI nova em Makepad (`makepad-widgets = "1.0"`, `app_main!`,
  `live_design!`). **Fase 1 concluída:** shell (janela + transport play/stop/bpm/
  relógio) + timeline `timeline-render` EMBUTIDA na janela Makepad.
- A timeline é uma janela winit/wgpu **borderless "encaixada"**: o shell chama
  `timeline_render::run_docked_window(transport, rx, DockSpec { parent_title, top, left, right, bottom })`,
  que espawana uma thread com a timeline rastreando a janela Makepad (por
  título, `FindWindowW`) e se mantém **colada ao client area com margens**
  (ex.: top=46 header, laterais 10, bottom 34 rodapé). A filha é registrada como
  janela **owned** da main (`GWLP_HWNDPARENT`): o Windows esconde/restaura ela
  junto no minimize e ela nunca fica atrás do app. O rastreio é **instantâneo**:
  um hook `WH_CALLWNDPROC` na thread da main reposiciona a filha no mesmo ciclo
  de `WM_MOVE`/`WM_SIZE`/`WM_WINDOWPOSCHANGED` (sem lag de polling; um loop de
  150ms fica só como fallback de DPI/estado inicial). `Transport` compartilhado
  (`Arc<Mutex>`) + clips espelhados via `WindowCommand` (read-only).
- `EventLoop` winit fora da main thread exige
  `EventLoopBuilderExtWindows::with_any_thread(true)` (timeline-render já faz).
- O app antigo (`app/`, Slint) permanece no workspace até o port cobrir os
  painéis (explorer, mixer, piano roll, sampler, beat). Núcleo reaproveitável
  (não-UI): `daw-*`, `timeline-render`, waveform_gpu, clip_cache.
- Próximas fases do port: explorer, mixer, piano roll, sampler, beat, floats.
- Rodar: `cargo run -p ruana-studio-makepad`.

## Comandos comuns

```sh
# Workspace inteiro (mais lento; inclui app + timeline-render)
cargo build

# Só a timeline real
cargo build -p timeline-render
cargo run -p timeline-render

# Só o app
cargo build -p ruana-studio
cargo run -p ruana-studio

# Shell Makepad (port em andamento)
cargo build -p ruana-studio-makepad
cargo run -p ruana-studio-makepad
```

Perfil dev: nosso código compila sem otimização (rápido), dependências de
runtime (skia, symphonia, cpal, wgpu) em `opt-level = 2`. Configuração em
`Cargo.toml` do workspace.

## Convenções

- Comentários e documentação **em português**.
- Edição Rust **2024**.
- Fazer *lint/check* antes de considerar concluído: `cargo build -p
  timeline-render`.
- Não adicionar comentários em código sem necessidade (seguir o estilo dos
  arquivos vizinhos).
- Ao mexer em `timeline-render`, lembrar que ele é consumido como biblioteca
  pelo futuro app — manter a API pública (`App`, re-exports) estável.