# Arquitetura de Janelas e Painéis

> A composição da UI: uma janela principal com regiões e splitters, mais
> janelas flutuantes (mixer/sampler) e popups.

**Última verificação:** 2026-10-13 · **Fontes:** `ui/windows/workspace.slint`
(2435 linhas), `ui/windows/float_panel.slint`, `app/src/floats.rs`,
`ui/main.slint`

---

## 1. Hierarquia

```
MainWindow (ui/main.slint)              ← Window raiz, tema, atalhos globais
└── undo-scope : FocusScope             ← dono dos atalhos de teclado
    └── Workspace (ui/windows/workspace.slint)
        └── VerticalLayout
            ├── Toolbar                  ← transporte, BPM, tema, minimizar/fechar
            ├── BarProperties            ← edit toolbar (ferramentas 0-5) + toggles
            └── workspace : Rectangle    ← área principal (vertical-stretch: 1)
                ├── ClipPool (dockado)   ← esquerda, largura = left-panel-width - 10px
                ├── clip-pool-window     ← a MESMA ClipPool quando flutuante
                ├── center-panel         ← track panel + splitter + Timeline
                ├── SplitterHorizontal   ← acima do bottom panel
                ├── bottom-panel         ← Mixer (tab 0) ou Sampler (tab 1)
                ├── explorer (direita)   ← + SplitterVerticalGrowRight
                └── bottom-toolbar       ← abas do painel inferior
```

`Workspace` é `export component … inherits Rectangle` e expõe **toda** a
superfície de estado/eventos que o `main.slint` e o Rust consomem — é o
"contrato" da UI.

---

## 2. Janelas flutuantes vs. modo overlay

O projeto tem **duas** implementações do mesmo contrato, e a diferença é
arquitetural (ver [[Fluxo de Interface]] §1):

| | `floats::install` | `floats::install_no_floats` |
|---|---|---|
| Janelas nativas | **sim** — uma `FloatPanel` por tool | **não** |
| Mixer/Sampler | desacoplam para janela própria | ficam no painel principal |
| `floated` | espelha quais estão abertas | sempre `[false, false]` |
| `fit_float_mixer_windows` | redimensiona as janelas | no-op |

O `app/src/main.rs` chama `install_no_floats` (arquitetura do AGENTS.md: UI
composicionada como overlay offscreen numa única janela winit/wgpu). O
`install` completo permanece no código para o modo com janelas.

Ambos devolvem o mesmo `FloatHandles { apply_mixer_state,
fit_float_mixer_windows }` — é isso que permite trocar sem tocar nos
consumidores.

---

## 3. Janela flutuante (FloatPanel)

`ui/windows/float_panel.slint` — `export component FloatPanel inherits Window`:

```slint
in-out property <int> tool-tab: 0;    // 0 = Mixer, 1 = Sampler
in-out property <length> mixer-natural-height: mixer-content.natural-height;
callback close-panel();
callback mixer-param-changed(int, string, float);
callback channel-clip-cleared(int);
callback mixer-insert-added(int) / -removed(int, int) / -toggled(int, int);
callback resize-cancel() / resize-move(length, length) / resize-end();
```

O título é derivado: `root.tool-tab == 0 ? "Mixer" : "Sampler"`.
O cursor do canto de redimensionamento também: `ns-resize` no mixer,
`nwse-resize` no sampler.

### Slots

`floats.rs` mantém `[Option<FloatPanel>; 2]` — **slot 0 = Mixer, slot 1 =
Sampler**. `on_float_toggle(tab)` faz toggle: se o slot está ocupado, fecha;
senão, cria. Várias janelas podem estar abertas ao mesmo tempo.

### Fechamento por dois caminhos

1. **Botão da própria janela** → `on_close_panel` → `take()` + `hide()`.
2. **Botão X do sistema** → `window().on_close_requested` → `take()` + `hide()`
   + `CloseRequestResponse::HideWindow`.

Ambos chamam `sync_floated()` para manter `Workspace.floated` correto.

### Redimensionamento

O canto inferior direito redimensiona, com regras **diferentes por tool**:

| Tool | Largura | Altura |
|---|---|---|
| Mixer (0) | **automática** (segue os canais), não redimensiona | `clamp(240 * sf, 640 * sf)` |
| Sampler (1) | `max(360 * sf)` | `max(240 * sf)` |

No primeiro move do drag o estado ancora o tamanho inicial e o ponteiro
(`(w0, h0, x0, y0)`); os moves seguintes aplicam o delta × scale factor.
`resize-cancel`/`resize-end` zeram o estado.

### Ajuste automático ao conteúdo (mixer)

```rust
let w = ((proj.tracks.len() + 1) as f32 * 105.0 * sf) as u32;  // +1 = master
let h = p.get_mixer_natural_height().clamp(240.0, 640.0) * sf;
```

**105 px por canal** e **+1 para o master**. A largura só é recalculada quando
o **número de canais muda** — *"nunca durante drag de fader"* (o comentário é
explícito). `apply_mixer_state` é usado só em mudanças estruturais
(tracks/inserts); fader/knob usam `update_channel`, sem rebuild.

### Medidores compartilhados

```rust
p.set_channel_meters(meter_values.clone().into());
p.set_channel_clipped(meter_clipped.clone().into());
```

Os `Rc<VecModel>` são **os mesmos** da janela principal: o bombeamento escreve
nos mesmos modelos, então a barra do mixer flutuante e a da timeline **não
podem divergir**.

### Ajustes dentro da flutuante → core

`on_mixer_param_changed` converte o valor, aplica em
`project.set_mixer_param(...)`, atualiza **só o canal**
(`update_channel`) e chama `audio.mark_dirty()`. `MixerParam::from_str` valida
o nome; `Volume` passa por `fader_pos_to_linear` (posição de fader → linear).

`on_channel_clip_cleared` → `audio.clear_meter_clip(channel)` (apaga o latch).

### Drop do SO na flutuante do sampler

Via `on_winit_window_event`:

| Evento | Ação |
|---|---|
| `HoveredFile` (áudio) | `set_sampler_drop_hover(true)` |
| `HoveredFileCancelled` | `set_sampler_drop_hover(false)` |
| `DroppedFile` (áudio) | dispara `ui.invoke_sampler_sample_dropped(name, path)` na janela **principal** |

O estado do sampler é espelhado para a flutuante pelo global `SamplerState`
(uma `Window` separada não tem acesso às properties do `MainWindow`).

---

## 4. Regiões e dimensões (defaults)

| Propriedade | Default | Splitter |
|---|---|---|
| `left-panel-width` (ClipPool) | `230px` | — |
| `tracks-header-width` | `200px` | `SplitterVerticalGrowLeft`, 120–420px |
| `explorer-width` | `300px` | `SplitterVerticalGrowRight`, 220–500px |
| `bottom-panel-height` | `215px` | `SplitterHorizontal`, 120–600px |
| `clip-pool-float-*` | x 260, y 80, w 320, h 520 | arrastável |

### O `center-panel` é calculado

```slint
x: clip-pool-open && !clip-pool-floating ? left-panel-width : 5px;
width: parent.width
     - (clip-pool-open && !clip-pool-floating ? left-panel-width : 5px)
     - (explorer-visible ? explorer-width + 10px : 0px);
height: bottom-panel-visible ? parent.height - bottom-panel-height - 5px
                             : parent.height;
```

**Ponto de atenção:** o `ClipPool` dockado reserva `left-panel-width`, mas ele
mesmo tem largura `left-panel-width - 10px` (fica com `x: 5px`). O
`bottom-panel` e o `SplitterHorizontal` **alinham o `x` pela mesma condição** —
se um dia a condição mudar, os três precisam mudar juntos.

### `timeline-area-expanded`

Quando o piano roll ou o timewarp abrem (teclas P/T), `Workspace` seta
`timeline-area-expanded = true`: a timeline assume a **largura total da
janela**, ocultando a track panel e o splitter (`visible: !root.timeline-area-expanded`,
`width: 0px`).

---

## 5. ClipPool: dockado ou flutuante

A **mesma** `ClipPool` é usada nos dois modos; o que muda é a visibilidade:

| Modo | Visibilidade |
|---|---|
| dockado (`clip_pool`) | `clip-pool-open && !clip-pool-floating` |
| flutuante (`clip-pool-window`) | `clip-pool-open && clip-pool-floating` |

O flutuante é um `Rectangle` com `border-width: 1px`, `border-color:
Theme.border`, `border-radius: 8px`, posicionado em `clip-pool-float-x/y` com
`clip-pool-float-width/height`.

Ambos roteiam os mesmos três gestos para o `Workspace`:

```
clip-clicked(id)        → root.selected-clip = id
clip-drag-started(...)  → root.drag-begin(...)
clip-drag-moved(x, y)   → root.drag-move(x, y)
clip-drag-finished      → root.drag-end()
```

O comentário no código deixa claro: **clique apenas seleciona; NÃO abre o Clip
Pool flutuante.**

---

## 6. Bottom panel: Mixer / Sampler por aba

O `bottom-panel` tem `current-tab = bottom-toolbar.selected-tab` e mostra **um
componente por vez** dentro de um `Rectangle` com `clip: true`:

| Tab | Componente | Visível quando |
|---|---|---|
| 0 | `Mixer` | `current-tab == 0` |
| 1 | `Sampler` | `current-tab == 1` |

Cada um ocupa `100% × 100%`. O `Mixer` recebe `floating: root.floated[0]`.

---

## 7. Estado de dados do Workspace

O `Workspace` declara defaults que permitem rodar a UI **sem Rust** (útil para
preview):

- `track-names: ["Drums", "Bass", "Vocal", "Synth"]`
- `clips: [Drums A, Bass A, Vocal Verse]`
- `mixer-volumes: [0.8 × 5]`, `mixer-pans: [0.5 × 5]`,
  `mixer-mutes/soloes/arms: [false × 5]`
- `mixer-max-inserts: 8`
- `total-bars: 32`, `ppq: 960`, `beats-per-bar: 4`, `bpm: 120`

Os arrays de mixer têm **5 posições** porque o default do Slint é estático;
em runtime o Rust publica modelos com `tracks.len() + 1`.

---

## 8. Pitfalls conhecidos

1. **Três lugares alinham o `x` pela condição do ClipPool** (§4). Mudar um só
   desalinha os painéis.
2. **`apply_mixer_state` faz rebuild — não use em drag de fader.** Use
   `update_channel`.
3. **Largura do mixer flutuante é derivada**, não livre: 105 px/canal + master.
4. **`FloatPanel` é uma `Window`.** Globals do Slint (`SamplerState`) existem
   justamente para atravessar essa fronteira.
5. **`floated` precisa ser sincronizado nos dois caminhos de fechamento**
   (botão interno e X do sistema).
6. **`bottom-panel` é escondido por `bottom-panel-visible`**, não por
   `selected-tab` — a aba escolhe *qual* tool, o flag decide *se* aparece.

---

## Links

- [[Fluxo de Interface]]
- [[Sistema de Tema]]
- [[Referência Slint 1.18]]
- [[Arquitetura Geral]]
- [[Patterns de UI Slint para DAW]]
