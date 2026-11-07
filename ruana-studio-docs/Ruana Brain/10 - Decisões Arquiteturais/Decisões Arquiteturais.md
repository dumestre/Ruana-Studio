# Decisões Arquiteturais (ADRs)

> Registro das decisões que estruturam o Ruana Studio, com o **porquê**.
> Cada uma foi extraída do código e dos comentários do próprio projeto.

**Última verificação:** 2026-10-13

---

## ADR-001 — Timeline renderizada por wgpu, não pelo Slint

**Decisão:** a timeline (grid, clips, playhead, waveform) é desenhada por um
renderer wgpu nativo. O Slint **não** desenha a timeline.

**Motivo:** latência zero no playhead e liberdade para shaders customizados de
waveform. O playhead roda no loop do renderizador, sincronizado com o buffer
de áudio, livre do gerenciador de janelas.

**Consequência:** existem **dois** codebases renderizando a mesma cena
(`timeline-render/` standalone e `app/src/timeline_gpu/`) que precisam
permanecer sincronizados em shader, paleta, thresholds de grid e `header_h`.
Ver [[wgpu Timeline Rendering]].

---

## ADR-002 — Slint como overlay offscreen, não como janela

**Decisão:** o Slint roda numa `Platform` customizada que **nunca cria janela**.
Existe **uma** janela winit, e a UI é rasterizada (Skia sobre wgpu-30) numa
textura que a engine compõe por baixo/por cima da timeline.

**Motivo:** a abordagem antiga era FemtoVG-GL numa janela oculta + readback
`glReadPixels` + premultiply em CPU + upload. Isso custava **cópias de pixels
por frame**. Com `SkiaWGPU30Renderer` sobre os **mesmos**
`Instance`/`Adapter`/`Device`/`Queue` da engine, o desenho é **GPU→GPU**: zero
cópia.

**Consequência:** `timeline-render` precisa expor seus recursos wgpu; o Slint
precisa das features `renderer-skia` + `unstable-wgpu-30`. Ver
[[Fluxo de Interface]] §1.

---

## ADR-003 — Modificadores de teclado não são roteados ao Slint

**Decisão:** `ModifiersChanged` do winit atualiza apenas o `SharedState` do
host. **Não** é despachado ao Slint.

**Motivo:** `slint::platform::WindowEvent::PointerPressed` **não carrega
modificadores**. Como o Slint sempre veria `shift=false`, qualquer ação gated
por modificador precisa ser interceptada no handler winit com o estado real.

**Consequência:**

- Shift+clique (colar) é interceptado no `MouseInput` e **não** vai ao Slint;
- Ctrl/Alt+roda vai **direto à engine** (`handle_scroll`), porque a rota
  `engine-scroll` do Slint teria Ctrl/Alt sempre falsos;
- o `Released` do clique interceptado precisa ser **engolido**
  (`paste_click_active`), senão um `up` órfão confirma um arrasto fantasma.

---

## ADR-004 — Commit de gesto só no UP

**Decisão:** durante um arrasto de clip, **nenhuma** chamada ao Rust e
**nenhuma** escrita no modelo de dados. Só floats mudam no global
`ClipDragState`; a posição é derivada por binding. O core recebe **um** commit
no release.

**Motivo:** anti-lag. Recalcular e re-renderizar a cena a cada movimento do
mouse custa caro e não é necessário — o Slint já reavalia os bindings por frame.

**Consequência:**

- `ClipDragState.active` só vira `true` **depois do limiar** (3 px), nunca no
  DOWN — senão todo clique simples fazia o clip "desaparecer" por um frame;
- latch: nunca desliga no meio do gesto;
- `raw-beat` é calculado do **delta** do cursor, preservando o ponto de pega
  (o clip não salta para o cursor);
- o sufixo `-committed` nos callbacks marca o fim de gesto.

Ver [[Fluxo de Interface]] §8.

---

## ADR-005 — Métricas de geometria são publicadas por frame, não por timer

**Decisão:** `TimelineMetrics` (px-per-beat, row-height, ruler-height, scroll,
snap-beat-step, track-count) é publicado pelo `tick` do host **a cada frame**.

**Motivo:** antes era um Timer de 50 ms. Enquanto o Timer não rodava, o drop
usava os defaults do Slint (**40 px/beat** e **42 px de linha**) — números que
**nunca** foram os da engine (que desenha com `px_per_beat` vivo e
`64 * zoom_y` de linha). O preview e o ponto de inserção divergiam.

**Consequência:** o que o usuário vê (ghost/preview) e o que entra na timeline
não podem divergir. `snap-beat-step` espelha `grid_beat_step` da engine pela
mesma razão — sem ele o Slint arredondava em 0.5 beat e a engine em 1 beat.

---

## ADR-006 — Um global de tema, tokens, e a escada de 3 degraus

**Decisão:** toda cor da UI vem de `Theme.*`. A hierarquia é
`base` (fundo/poço) → `surface` (painel) → `surface-high` (widget). Não pular
degrau. A timeline tem degraus exclusivos (`timeline-bg`, `surface-step`,
`grid-line`, `grid-sub`).

**Motivo:** elimina duas classes de erro — painel com cor de widget (o filho
não contrasta com o pai) e botão com cor de fundo (o botão fica mais escuro
que o painel que o contém).

**Consequência:** a ordem das listas de paleta é **contrato**; adicionar tema
exige inserir em **todas** as listas. A sombra precisa ser sempre mais escura
que o fundo onde cai, por isso cada tema escuro tem o seu quase-preto tingido.
Ver [[Sistema de Tema]].

---

## ADR-007 — Rust não escreve `Theme` diretamente

**Decisão:** o tema é aplicado via `MainWindow.theme-index`, cujo `changed`
seta `Theme.active`.

**Motivo:** o Slint **não gera acessor Rust para globals**. Não existe
`ui.global::<Theme>().set_active(...)` disponível no fluxo do projeto.

**Consequência:** a property raiz é o **único** caminho de escrita. O Rust
apenas clampa (0..6), persiste em `theme.cfg` e sincroniza os cursores.

---

## ADR-008 — Stream de áudio nunca fecha

**Decisão:** o motor abre o stream cpal no startup e mantém aberto até o
programa fechar.

**Motivo:** abrir stream custa dezenas de ms e estala no primeiro buffer. Sem
stream aberta **não existe pré-escuta** (arrastar e ouvir com transporte
parado).

**Consequência:** mutações do documento apenas marcam dirty
(`mark_dirty`, barato); um timer de **90 ms** consolida e publica o snapshot
uma vez. Ver [[Engine de Áudio]].

---

## ADR-009 — Dois modos de reprodução mutuamente exclusivos

**Decisão:** `PlayMode::Timeline` (clips tocam, agulha anda, sequenciador mudo)
e `PlayMode::BeatOnly` (só o pattern soa, timeline parada). Trocar de modo =
stop + start; nunca sobreposição.

**Motivo:** evita ambiguidade de transporte e sincronização. É o que faz o
`Espaço` ser **roteado pelo cursor**: sobre a popup do beat comanda o
sequenciador, em qualquer outro lugar comanda a timeline.

**Consequência:** `BeatOnly` **não** publica posição de playhead. O STOP devolve
a agulha para `play_start_ticks`. Ver [[Timeline e Transporte]] e [[KEYMAP]].

---

## ADR-010 — Timewarp híbrido (varispeed → keylock)

**Decisão:** quando o tempo muda, o motor toca o áudio original em
**varispeed** (pitch acompanha) imediatamente. Um worker renderiza em background
a versão com **pitch preservado**; quando fica pronta, o buffer entra no cache
do motor, o snapshot é republicado e o clip passa a tocar esticado 1:1.

**Motivo:** é o comportamento do Cubase durante gesto de tempo — resposta
imediata sem esperar o render caro.

**Consequência:** `StretchWorker` precisa de quatro estruturas (`READY`,
`IN_FLIGHT`, `FAILED`, `PUBLISHED` com LRU de 24) para não duplicar renders nem
insistir em alvos que falharam. Ver [[Engine de Áudio]].

---

## ADR-011 — Undo/redo por eventos do documento

**Decisão:** cada mutação emite um `DocumentEvent` que carrega os dados
**antes/depois**. Undo reverte, redo reaplica. A UI assina os eventos e
atualiza seus modelos **granularmente**.

**Motivo:** um único mecanismo serve para undo/redo, para a projeção na UI e
para o crescimento da timeline (`bars_for_end_beat` considera `old` **e**
`new`, para cobrir undo/redo sem ramificar).

**Consequência:** `apply_change` **nunca** toca no `Project` — ele é chamado de
dentro de uma mutação do core (reentrância proibida). Ver
[[Projeto e Persistência]].

---

## ADR-012 — Timewarp escreve de volta em dois estágios

**Decisão:** `tempo_write_back` grava o mapa no projeto a cada movimento
(preview ao vivo, sem histórico). `tempo_commit` grava **uma** operação de
undo no fim do gesto.

**Motivo:** gravar histórico a cada pixel de movimento poluiria a pilha de
undo. O commit é o que faz o motor republicar o snapshot e os clips musicais
serem reesticados.

**Consequência:** dois callbacks distintos na engine, com semânticas
diferentes. Ver [[Fluxo de Interface]] e [[Timeline e Transporte]].

---

## ADR-013 — Cursores de ferramenta via Win32 nativo

**Decisão:** cursores `.cur` carregados por `LoadImageW` e reimpostos por
`SetWindowSubclass` interceptando `WM_SETCURSOR`.

**Motivo:** o Slint não expõe cursor de arquivo. Numa DAW o cursor **é** o
indicador de ferramenta ativa.

**Consequência:** módulo inteiro com `#[cfg(windows)]` e no-ops fora do
Windows; conversão físico↔lógico obrigatória para a checagem de área
(`GetDpiForWindow / 96.0`). Os `.cur` são **gerados no build.rs** a partir de
SVG, em duas variantes (clara/escura). Ver [[Cursores e Ferramentas]].

---

## ADR-014 — Janelas flutuantes com dois backends

**Decisão:** existem `floats::install` (janelas nativas) e
`install_no_floats` (tudo numa janela), ambos devolvendo o mesmo
`FloatHandles`.

**Motivo:** permitir migrar para a arquitetura de overlay único (ADR-002) sem
reescrever os consumidores. O `main.rs` usa `install_no_floats`.

**Consequência:** o contrato `FloatHandles { apply_mixer_state,
fit_float_mixer_windows }` é o ponto de troca. Medidores são `Rc<VecModel>`
**compartilhados** para que janelas não possam divergir. Ver
[[Arquitetura de Janelas e Painéis]].

---

## Links

- [[Arquitetura Geral]]
- [[Fluxo de Interface]]
- [[Sistema de Tema]]
- [[Cursores e Ferramentas]]
- [[Engine de Áudio]]
- [[wgpu Timeline Rendering]]
- [[Estado Atual e Roadmap]]
