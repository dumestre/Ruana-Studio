# Componentes e Tokens

> Catálogo dos componentes `.slint` do Ruana Studio e as convenções que eles
> seguem.

**Última verificação:** 2026-10-13 · **Fonte:** inventário de `ui/` (~35.000
linhas de `.slint`)

---

## 1. Mapa de pastas

```
ui/
├── main.slint                    ← MainWindow (janela raiz, atalhos, tema)
├── theme.slint                   ← global Theme (7 paletas)
├── std-widgets.slint
├── toolbar.slint                 ← transporte, BPM, tema, minimizar/fechar
├── transport.slint               ← LCD bars:beat:ticks, play/stop/rec
├── edit_toolbar.slint            ← ferramentas 0-5
├── clip_pool.slint               ← biblioteca de clips + BEAT PATTERNS
├── clip_palette.slint            ← paleta de cores de clip
├── components/                   ← kit reutilizável (25 arquivos)
├── timeline/                     ← timeline, track rows, headers
│   └── clip_drag/                ← estado global do arrasto
├── beat/                         ← step sequencer (12 arquivos)
├── mixer/                        ← canais, inserts, pan (5 arquivos)
├── sampler/                      ← sampler (18 arquivos)
├── explorer/                     ← file browser, media/plugin browser
├── piano_roll/                   ← (vazio — piano roll vive na engine)
├── icons/                        ← 62 SVGs
└── windows/                      ← Workspace e FloatPanel
```

---

## 2. Kit de componentes (`ui/components/`)

| Componente | Papel |
|---|---|
| `button.slint` | botão base |
| `checkbox.slint` | checkbox |
| `knob.slint` | knob rotativo |
| `meter.slint` / `meter_reuse.slint` | medidores (ver §4) |
| `ruler.slint` | régua de compassos |
| `scrollbar.slint` | barra de rolagem |
| `splitter.slint` | `SplitterVerticalGrowLeft`, `SplitterVerticalGrowRight`, `SplitterHorizontal` |
| `theme_picker.slint` | seletor de tema (ver [[Sistema de Tema]] §6) |
| `context_menu.slint` | menu de contexto + `ClipContextMenuState` |
| `color_picker.slint` | seletor de cor |
| `clip_fade.slint` / `clip_gain.slint` | grips de fade e ganho do clip |
| `zoom_bar.slint` | `ZoomBarVertical` etc. |
| `bar_properties.slint` | barra de propriedades (tool + toggles) |
| `bottom_toolbar.slint` | abas do painel inferior |
| `rack.slint` | rack de devices |
| `browser_item.slint` | item de browser |
| `search_bar.slint` | campo de busca |
| `project_key.slint` | indicador de tonalidade |
| `audio_scale.slint` | escala de áudio |
| `horizontal_bar.slint` / `vertical_bar.slint` | barras genéricas |
| `sampler_area.slint` | área do sampler |
| `timeline_area.slint` | área da timeline |

---

## 3. Convenções de código Slint no projeto

### Declaração de estado

```slint
in-out property <bool> playing: false;      // bidirecional (UI ↔ UI)
in property  <string> text: "Click";        // entrada
out property <bool> pressed: false;         // saída (read-only p/ consumidor)
private property <length> internal: 40px;   // interno
```

### Callbacks — dois padrões de nome

| Sufixo | Significado | Exemplo |
|---|---|---|
| `-requested` | pedido da UI para o Rust **decidir** | `undo-requested`, `clip-delete-requested` |
| `-changed` | notificação de que um valor mudou | `tool-changed`, `bpm-changed` |
| `-committed` | **fim de gesto** — o valor já está final | `clip-fade-committed`, `clip-gain-committed` |

O sufixo `-committed` é a materialização do padrão "commit só no UP" (ver
[[Fluxo de Interface]] §8): durante o gesto nada é commitado; o `-committed`
sai uma única vez no release.

### Imports

Os arquivos importam por caminho relativo com a extensão:

```slint
import { Theme } from "../theme.slint";
import { ClipData } from "../timeline/clip_data.slint";
import { ScrollView } from "std-widgets.slint";
```

---

## 4. Padrão de medidor (meter) — evitar `HorizontalLayout`

Documentado em [[Patterns de UI Slint para DAW]]: **não** usar
`HorizontalLayout` dentro do componente `Meter`. Usar **posicionamento
absoluto**, porque o layout participa do ciclo de cache de layout e provoca
*binding loop* em tempo de compilação quando aninhado em `Flickable`.

```
largura total = triangle-width + gap + meter-width + gap + db-width
                14px          2px   13px          2px   16px   = 45px
```

**Pitfall:** `padding-bottom` **não funciona** em `Rectangle` — para posicionar
texto com padding inferior, usar `Text.y = parent.height - text-height`.

---

## 5. Grip de fade e ganho (clip)

Padrão "live vs display" — três camadas de estado por parâmetro:

| Propriedade | Origem |
|---|---|
| `fade-in`, `fade-out` | do **modelo** (Rust core) |
| `live-fade-in`, `live-fade-out` | durante o drag, seguem o mouse |
| `display-fade-in`, `display-fade-out` | **computado**: `fading ? live-* : model-*` |

Feedback de cursor:

```slint
mouse-cursor:
    root.fading ? ew-resize
    : root.on-fade-grip ? ew-resize
    : root.on-edge ? ew-resize
    : pointer;
```

Ganho: `gain-linear = Math.pow(10.0, display-gain-db / 20.0)`,
sensibilidade **1.5 px por dB**, faixa **−24 dB a +12 dB**, com grip central
mostrando o valor em dB durante o drag.

**Prevenção de movimento acidental** (regra de ouro do projeto):

```slint
if root.on-fade-grip { root.fading = true; return; }   // NÃO cai no drag genérico
if root.on-gain-grip { root.gaining = true; return; }
// só então resize/drag/marquee
```

O `return` é o que garante que pegar a alça de fade **nunca** mova o clip.

### Cores de grip tematizadas

```slint
private property <bool> fade-grip-active:
    root.on-fade-grip && (touch.pressed || root.fading);
```

| Estado | Cor |
|---|---|
| hover / normal | `Theme.accent-hover` |
| pressed / active | `Theme.accent` |
| borda/texto sobre accent | `Theme.on-accent` |

---

## 6. Structs de dados

A ponte de dados UI ↔ Rust usa structs declaradas em `.slint`:

`ui/timeline/clip_data.slint`:

```slint
export struct ClipData {
    id: int,
    track-index: int,
    name: string,
    beat: float,
    len: float,
    is-midi: bool,
    color: int,
    multi-selected: bool,
    fade-in: float,
    fade-out: float,
    gain-db: float,
    file-path: string,
    source-offset: float,
    midi-fingerprint: string,
    waveform: image,
}
```

Outros: `BrowserFileEntry` (`explorer/file_browser.slint`),
`TimelineCoords` (retorno de `map-window-coords-to-timeline`).

---

## 7. Globals de estado compartilhado

| Global | Arquivo | Papel |
|---|---|---|
| `Theme` | `theme.slint` | paletas |
| `ClipDragState` | `timeline/clip_drag/clip_drag_state.slint` | arrasto de clips |
| `TimelineMetrics` | idem | geometria real da timeline |
| `SamplerMetrics` / `SamplerState` | idem | sampler (inclui espelho p/ janela flutuante) |
| `BeatMetrics` | idem | geometria do beat popup |
| `BeatState` | `beat/beat_state.slint` | patterns, grid, swing, humanize, tracks |
| `ClipContextMenuState` | `components/context_menu.slint` | menu de contexto |

Globals existem para atravessar fronteiras onde properties não chegam: entre
componentes distantes e **entre janelas** (`SamplerState` existe porque
`FloatPanel` é uma `Window` separada).

---

## 8. BeatState — o estado do sequenciador

`ui/beat/beat_state.slint` — 12 arquivos compõem o beat sequencer
(`beat.slint`, `beat_popup.slint`, `beat_content.slint`, `beat_header.slint`,
`beat_menus.slint`, `beat_step.slint`, `beat_step_grid.slint`,
`beat_track.slint`, `beat_track_header.slint`, `beat_track_panel.slint`,
`step_cell.slint`).

`BeatState` carrega:

| Grupo | Propriedades |
|---|---|
| patterns | `patterns` (matriz `bool`), `pattern-names`, `pattern-durations`, `current-pattern` |
| grid | `grid-resolution`, `humanize`, `swing` |
| trilhas | `instrument-names`, `track-sample-names`, `track-active`, `track-solo`, `track-pan`, `track-volume`, `track-mixer-channel` |
| playhead | `playhead` (`-1` = parado) |

Callbacks de persistência (consumidos pelo Rust): `step-committed`,
`grid-resolution-committed`, `humanize-committed`, `swing-committed`,
`track-param-committed`, `pattern-changed`, `create-pattern`, `duplicate-beat`,
`copy-beat`, `clear-beat`, `delete-beat`, `copy/duplicate/delete/clear-track`,
`insert-pattern`, `preview-track`, `open-beat-popup`, `sampler-open`.

---

## 9. Ícones

`ui/icons/` tem **62 SVGs**. Convenção observada: muitos existem em **dois**
estados — `X.svg` (contorno) e `X_filled.svg` (preenchido), usado para
indicar a ferramenta/estado ativo:

`select`/`select_filled`, `slice`/`slice_filled`, `glue`/`glue_filled`,
`stretch`/`stretch_filled`, `draw`/`draw_filled`, `erase`/`erase_filled`,
`snap`/`snap_filled`, `follow`/`follow_filled`, `clippool`/`clippool_accent`.

Tipos de trilha: `audio`, `midi`, `instrument`, `drum`, `fx`, `bus`, `master`.
Transporte: `play`, `pause`, `stop`, `rec`, `skip_back`, `skip_fwd`, `loop`,
`metronome`. Janela: `minimize`, `maximize`, `restore`, `close`.

---

## 10. Pitfalls conhecidos

1. **`HorizontalLayout` dentro de `Flickable`** → binding loop / panic em
   tempo de compilação. Use posicionamento absoluto (§4).
2. **`padding-bottom` não existe em `Rectangle`** — posicione o `Text` à mão.
3. **`return` obrigatório nos grips** de fade/ganho, senão o clip se move.
4. **Eventos de pointer vazam entre popups e a timeline** — adicione uma
   `TouchArea` de fundo no popup para consumir.
5. **Globals não cruzam janelas automaticamente** — para `FloatPanel`, espelhe
   via global (é o caso de `SamplerState`).
6. **`ui/piano_roll/` está vazio.** O piano roll é desenhado pela engine wgpu,
   não por um componente Slint.

---

## Links

- [[Sistema de Tema]]
- [[Cursores e Ferramentas]]
- [[Arquitetura de Janelas e Painéis]]
- [[Fluxo de Interface]]
- [[Referência Slint 1.18]]
- [[Patterns de UI Slint para DAW]]
