# Fluxo de Interface

> Como um evento de input atravessa o app: winit → roteamento → Slint overlay
> e/ou engine wgpu → core. Inclui o padrão "anti-lag" dos gestos.

**Última verificação:** 2026-10-13 · **Fontes:** `app/src/host.rs` (1231 linhas),
`app/src/slint_host.rs`, `app/src/interaction.rs`, `ui/main.slint`,
`ui/timeline/clip_drag/clip_drag_state.slint`, `app/src/os_drop.rs`

---

## 1. O modelo mental: uma janela, duas camadas

O Ruana Studio **não** usa a janela do Slint. Existe **uma** janela winit, e o
conteúdo dela é composto por duas camadas:

```
┌──────────────────────────────────────────────────────┐
│  Janela winit (a única janela nativa)                │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │ textura `timeline_bg`  ← engine wgpu           │  │
│  │ (timeline, grid, clips, playhead, waveform)    │  │
│  └────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────┐  │
│  │ textura `overlay`      ← Skia/wgpu-30          │  │
│  │ (UI Slint: toolbar, mixer, popups, painéis)    │  │
│  └────────────────────────────────────────────────┘  │
│                      ↓                               │
│            surface.present()                         │
└──────────────────────────────────────────────────────┘
```

O host offscreen (`app/src/slint_host.rs`) instala uma `Platform` do Slint que
**nunca cria janela**: `create_window_adapter` devolve sempre o mesmo adapter
virtual. A UI e a timeline **compartilham a mesma** `Instance`/`Adapter`/
`Device`/`Queue`, então o `draw()` do Slint rasteriza **GPU→GPU** na textura de
overlay — zero cópia de pixels (a abordagem antiga era FemtoVG-GL + janela
oculta + `glReadPixels` + premultiply em CPU + upload).

Consequência direta: a UI Slint pode declarar a timeline como **transparente**
("buraco") e o frame do wgpu aparece por baixo. É o que faz
`Timeline.overlay-hosted` (`ui/timeline/timeline.slint:49`).

---

## 2. O loop de eventos

`app/src/host.rs` implementa `ApplicationHandler` do winit. O único ponto de
render é:

```
Event::WindowEvent { WindowEvent::RedrawRequested }
  → SlintOverlay::update_timers_and_animations()   // avança timers/animações Slint
  → EngineBridge::tick(ui)                         // playhead, espelho de clips, render da timeline
  → EngineBridge::engine.render()                  // compõe overlay + apresenta
```

`SlintOverlay::draw()` só re-renderiza quando o Slint marcou `redraw` (dirty):
caso contrário a textura mantém o último frame e a composição sai de graça.

### Estado compartilhado

`struct SharedState` guarda o que o handler precisa entre eventos:

| Campo | Papel |
|---|---|
| `scale` | scale factor (DPI) |
| `win_pos` | canto superior-esquerdo da janela (px físicos) |
| `last_cursor` | última posição do cursor (px físicos) |
| `ctrl`, `alt`, `shift` | modificadores atuais |

O `win_pos` existe porque drags do SO entregam posição **global** de cursor:
sem o canto da janela não dá para converter em coordenadas locais.

---

## 3. Tabela de roteamento de eventos

Esta é a parte mais importante do fluxo. **Cada evento decide para quem vai.**

| Evento winit | Destino | Observação |
|---|---|---|
| `CloseRequested` | — | `reset_cursor_to_default()` + `exit()` |
| `Resized` | engine + overlay | `engine.resize()` e `overlay.resize()` (senão a UI estica) |
| `ScaleFactorChanged` | estado + engine + overlay | atualiza `scale`, `bridge.scale`, ambos os resizes |
| `Moved` | estado + bridge | guarda `win_pos` |
| `Focused(true)` | Slint | `restore_keyboard_focus()` — atalhos morrem após alt-tab sem isso |
| `RedrawRequested` | engine | ver §2 |
| `ModifiersChanged` | **só o estado** | Ctrl/Alt/Shift guardados; **NÃO** são roteados ao Slint |
| `CursorMoved` | Slint | `PointerMoved` em coordenadas **lógicas** |
| `CursorLeft` | Slint | `PointerExited` |
| `MouseInput` | Slint (com interceptação) | ver §4 |
| `MouseWheel` | engine **ou** Slint | Ctrl/Alt → engine; senão → Slint |
| `KeyboardInput` | engine, depois Slint | ver §5 |
| `HoveredFile` / `HoveredFileCancelled` / `DroppedFile` | os_drop | ver §7 |

### O detalhe crítico dos modificadores

**`slint::platform::WindowEvent::PointerPressed` NÃO carrega modificadores de
teclado.** O `ModifiersChanged` do winit é guardado no `SharedState` mas
**deliberadamente não é despachado para o Slint**. Por isso:

- toda ação *gated* por modificador precisa ser interceptada **no handler
  winit**, usando o estado real;
- o Slint, quando recebe um evento de ponteiro, sempre veria `shift=false`.

Isso é a raiz de vários comportamentos do §4 e §5.

---

## 4. Mouse: a interceptação do Shift+clique

`MouseInput` (Pressed) tem um caso especial **antes** de virar evento do Slint:

```rust
if shift && !ctrl && !alt && button == Left {
    // converte px físicos de janela → coordenadas do quad da timeline
    let (ox, oy, ow, oh) = engine.timeline_rect;
    let (qx, qy) = (px - ox, py - oy);
    if ow > 1.0 && oh > 1.0
        && qx >= 0.0 && qx < ow
        && qy >= engine.ruler_px && qy < oh
    {
        engine.paste_at_beat(target.beat, target.track);
        self.paste_click_active = true;
        return;   // NÃO despacha para o Slint
    }
}
```

Três decisões importantes:

1. **A engine trabalha em px físicos relativos ao quad da timeline**, não em
   coordenadas de janela. Sem a conversão, o beat/track saem deslocados pelo
   offset do painel/header e o clip colado nasce fora da área visível.
2. **Guarda de área:** só cola dentro do quad, **abaixo da régua** — não em
   popups/knobs do Slint nem no painel de tracks.
3. **`paste_click_active`:** o clique não vai para o Slint, então o `Released`
   correspondente é **engolido**. Um `up` sem `down` confirmaria um arrasto
   fictício na engine.

---

## 5. Teclado: quem decide o que tocar

`KeyboardInput` passa primeiro pela engine:

```rust
match engine.handle_key(&key) {
    KeyAction::TogglePlay => { ui.invoke_toggle_play_routed(); false }
    // …
}
```

O comentário no código é explícito sobre a divisão de responsabilidade:

- a engine processa os **mesmos atalhos do renderer standalone** (1-6, P,
  F/Ctrl+F, Ctrl+C/A/Z/Y/V/D, Delete, …);
- **`Espaço` é resolvido no handler**, mas a **decisão de o que tocar é do
  Slint**, via `toggle-play-routed` — porque só a UI conhece o **contexto do
  ponteiro**. Sobre a popup do beat o espaço comanda o step sequencer; em
  qualquer outro lugar, a timeline;
- o callback do Rust confirma o estado real (`set_playing`/`set_beat_playing`
  em `playback.rs`), então os dois botões de play ficam coerentes;
- a tecla **não vai para o Slint como tecla** e não há double-toggle.

### Foco de teclado

Os atalhos globais vivem num `FocusScope` (`ui/main.slint:714`). O foco se
perde quando o usuário clica num campo editável (busca do Explorer, rename de
clip) e é recuperado em **dois** pontos:

1. clique na área da timeline (`timeline.slint → keyboard-focus-requested`);
2. quando a janela volta a ter foco do SO (Rust → `restore_keyboard_focus`).

Sem isso os atalhos "morrem silenciosamente" — está comentado no arquivo.

---

## 6. Scroll e zoom (o padrão anti-lag)

`app/src/interaction.rs` implementa zoom/scroll da timeline. O cabeçalho do
arquivo descreve o algoritmo melhor que qualquer resumo:

### Zoom horizontal — suave SEM perder a âncora

1. No **início do gesto** (Ctrl+roda / Ctrl+arrasto 2 dedos) captura o ponto
   sob o cursor (`anchor_px`) **uma vez**. Ele fica **fixo** durante o gesto.
2. Acumula o delta (`dz_accum`) e deriva um alvo contínuo **exponencial**:
   `base * exp(dz * ZOOM_GAIN)`.
3. Um `Timer` de **8 ms** interpola o zoom exibido em direção ao alvo
   (amortecimento frame-independente).
4. O scroll de compensação é **SEMPRE derivado do zoom exibido**:

```
sx = pad + anchor_px * zoom - mx
```

Como `anchor_px` é constante, **o ponto sob o cursor não se move nunca** —
não importa o amortecimento. Zero drift, zero "ímã".

Constantes:

| Nome | Valor |
|---|---|
| `ZOOM_GAIN` | `0.02` (sensibilidade por px de delta) |
| `ZOOM_SPEED` | `30.0` (rigidez do amortecimento) |
| `MIN_ZOOM` / `MAX_ZOOM` | `0.25` / `64.0` |
| `MIN_ZOOM_Y` / `MAX_ZOOM_Y` | `0.25` / `6.0` |
| `BASE_ROW_HEIGHT` | `64.0` (`row-height = 64 * zoom_y`) |
| `SCROLL_GAIN` | `2.5` |

### Zoom vertical e scroll — instantâneos

Deliberadamente **sem** amortecimento (previsíveis e diretos):

- **Alt+roda** → zoom vertical; mantém a linha sob o cursor ancorada via
  `new_sy = sy + t * BASE_ROW_HEIGHT * (new_zy - zy)`;
- **Shift+roda** → scroll horizontal (`sx - raw_dy * SCROLL_GAIN`);
- **roda normal** → scroll nas duas dimensões.

### Onde o wheel é decidido

No handler winit, `MouseWheel` com **Ctrl ou Alt vai direto para a engine**
(`engine.handle_scroll(dx, dy, ctrl, alt)`), **sem passar pelo Slint**. O
comentário explica: como `ModifiersChanged` não é enviado ao Slint, a rota
`engine-scroll` do Slint teria Ctrl/Alt **sempre falsos**. Sem Ctrl/Alt, o
wheel vai ao Slint como `PointerScrolled`.

O delta é normalizado: `LineDelta` × 40.0, `PixelDelta` cru.

---

## 7. Drag & drop do SO (Explorer → DAW)

`app/src/os_drop.rs` implementa um fluxo com feedback **idêntico** ao drag
interno do ClipPool:

| Fase | O que acontece |
|---|---|
| `HoveredFile` | ativa o **mesmo overlay visual** do drag interno **e** dispara a decodificação em background (thread) preenchendo o cache de samples |
| durante o hover | ghost/snap/preview de drop aparecem como se fosse um drag do ClipPool (`drag-active` etc.) |
| `DroppedFile` | o sample **já está em RAM**; a inserção é imediata |
| `HoveredFileCancelled` | desliga o overlay e **descarta o job** |

O truque é o **pré-decode durante o hover**: quando o usuário solta, não há
espera de I/O.

`HoveredFile` também chama `restore_keyboard_focus()` — o drag do SO tira o
foco de teclado da janela.

A janela flutuante do sampler tem o **mesmo tratamento** via
`on_winit_window_event` (`app/src/floats.rs`): `HoveredFile`/`DroppedFile` de
áudio viram `invoke_sampler_sample_dropped`.

---

## 8. O padrão "commit só no UP"

A filosofia anti-lag está escrita no topo de
`ui/timeline/clip_drag/clip_drag_state.slint`:

> DURANTE o gesto, NENHUMA chamada ao Rust e NENHUMA escrita no modelo de
> dados. Apenas floats mudam aqui e a posição é derivada por binding —
> atualiza a cada frame. O core só recebe UM commit no release (UP/drop).

`ClipDragState` (global do Slint) implementa isso:

| Grupo | Propriedades |
|---|---|
| estado | `active`, `clip-id` |
| snapshot do clip (no DOWN) | `name`, `orig-beat`, `orig-track`, `len-beats`, `color-index`, `is-midi` |
| métricas capturadas | `px-per-beat`, `row-height`, `track-count`, `snap-enabled` |
| marquee | `marquee-active`, `marquee-x0/y0/x1/y1` |
| cursor (janela) | `start-cursor-x/y` (fixo), `cursor-x/y` (a cada move) |
| derivados (por frame) | `dx`, `dy`, `abs-dx`, `abs-dy`, `moved`, `raw-beat`, `live-beat`, `row-delta`, `live-track` |

Detalhes que valem ouro:

- **`active` só vira `true` depois do limiar** (`moved` = |dx| > 3px ou
  |dy| > 3px), **nunca no DOWN**. Senão todo clique simples fazia o clip
  desaparecer e o overlay nascer no mesmo lugar por um frame — piscada/tremor.
- **Latch:** quem liga é o handler de MOVE (uma escrita só) e **nunca desliga
  no meio do gesto**, mesmo que o cursor volte para dentro do limiar.
- **`raw-beat` usa o DELTA do cursor**, não a posição absoluta: o ponto de
  pega dentro do clip é preservado, **o clip não salta para o cursor**.
- **`row-delta` é arredondado** → snap de linha inteira, como em qualquer DAW.
- **`reset()` zera o delta junto** — sem isso `moved` continuaria `true` com o
  cursor do gesto anterior e o próximo DOWN já nasceria "movido".
- **Snap de 1 beat de propósito:** o grid do core é inteiro em beats, é o que
  o projeto sabe persistir.

### O marquee pode nascer em cima de um clip

`Ctrl+clique` pode começar **sobre um clip**: nesse caso a `TouchArea` do clip
captura o gesto e o `zoom-pan-area` da timeline **nunca vê o DOWN**. Guardando
o estado no global, o clip **inicia** o marquee no seu DOWN (com Ctrl) e o
`zoom-pan-area` apenas **continua** o gesto nos move/up que chegam até ele.

---

## 9. Métricas publicadas (ponte UI ↔ engine)

Quatro globals carregam geometria real entre as camadas. **O dono é o Rust**:

| Global | Conteúdo | Quem publica |
|---|---|---|
| `TimelineMetrics` | abs-x/y, content-top, width, rect-height, ruler-height, scroll, px-per-beat, row-height, snap-beat-step, track-count | `host.rs` `tick`, **a cada frame** |
| `SamplerMetrics` | abs-x/y, width, height, visible | workspace, timer 50 ms |
| `BeatMetrics` | abs-x/y, width, height, rows-y, row-height, track-count, visible | main, timer 50 ms |
| `SamplerState` | sample-name, loaded, format, duration | workspace → janela flutuante |

O comentário do `TimelineMetrics` explica por que ele **não** pode usar timer:

> Publicá-las por um Timer de 50 ms fazia o preview e o ponto de inserção
> divergirem: enquanto o Timer não rodava, o drop usava 40px/beat e 42px de
> linha — números que NUNCA foram os da engine (que desenha com `px_per_beat`
> vivo e 64*zoom_y de linha).

Ou seja: **as métricas descrevem a cena que o wgpu REALMENTE desenhou no
último frame**. O ghost do ClipPool, o preview de drop e o ponto de inserção
leem daqui — então o card que o usuário vê e o clip que entra na timeline não
podem divergir. `snap-beat-step` espelha `grid_beat_step` da engine pela mesma
razão: sem ele o Slint arredondava sempre em 0.5 beat e o preview em 1 beat.

`TimelineMetrics` também declara o callback
`waveform-resolution-requested(id, width, wx0, wx1, height, gain, fade-in,
fade-out)` — o pedido nasce dentro de cada `TrackClip`, o handler vive no Rust.

---

## 10. Pitfalls conhecidos

1. **Modificadores não chegam ao Slint em eventos de ponteiro.** Toda ação
   gated por Ctrl/Shift/Alt precisa ser interceptada no handler winit (§3).
2. **`up` órfão = arrasto fantasma.** Sempre pareie a interceptação com um
   flag de "engolir o release" (o `paste_click_active`).
3. **Coordenadas: três sistemas.** px físicos de janela, px físicos do quad da
   timeline, unidades lógicas do Slint. Confundir os três desloca beats e
   tracks.
4. **Timer de 50 ms não serve para geometria de drag.** Use publicação por
   frame (§9).
5. **O foco de teclado morre ao clicar em campo editável.** Restaure nos dois
   pontos (§5).
6. **`Resized` exige dois resizes.** `engine.resize()` **e**
   `overlay.resize()`; esquecer o segundo deixa a UI esticada/embaçada ao
   maximizar.

---

## Links

- [[Arquitetura Geral]]
- [[Referência winit 0.30]]
- [[Referência Slint 1.18]]
- [[Cursores e Ferramentas]]
- [[Arquitetura de Janelas e Painéis]]
- [[Patterns de UI Slint para DAW]]
- [[wgpu Timeline Rendering]]
- [[KEYMAP]]
