# Estado Atual e Roadmap

> O que está implementado, o que é placeholder e o que falta. Separando
> **fato confirmado no código** de **trabalho futuro**.

**Última verificação:** 2026-10-13 · **Método:** leitura direta do código-fonte
e dos manifests

---

## 1. Implementado e verificado no código

### Modelo de documento (`daw-core`)

| Módulo | Conteúdo |
|---|---|
| `project.rs` | `Project`, `DocumentEvent`, `ChangeKind`, subscribe, undo/redo |
| `clip.rs` | `Clip`, `ClipKind`, `TimeMode`, `WarpMarker`, cores determinísticas |
| `track.rs` | `Track`, `TrackKind` (Audio/Midi/Instrument/Drum/Fx/Bus/Master) |
| `mixer.rs` | `MixerChannel`, `MixerParam`, `InsertSlot` |
| `transport.rs` | `Transport`, `PlayMode`, conversões de posição |
| `timeline.rs` | `TempoMap`, `Grid`, `RulerTick`, `TempoChange` |
| `warp.rs` | `WarpPlan`, `WarpSegment`, `plan_for_clip` |
| `history.rs` | pilhas de undo/redo |
| `hit.rs` | `TimelineHitIndex` (rstar) |

### Áudio (`daw-audio`, `daw-media`)

- Engine com stream cpal permanente, mistura e transporte na thread RT;
- Timewarp híbrido (varispeed → keylock) com worker de render;
- `MeterBank` / medidores por canal com latch de clip;
- Decodificação via symphonia (WAV/MP3/FLAC/OGG) + hound para WAV;
- `WaveformCache` com LOD multinível (`LodBars` min/max, nível fino de
  16384 colunas, LRU por `clip_id`).

### MIDI (`daw-midi`)

- `Note`, `NoteEditor` (add/move/resize/velocity/quantize/hit-test);
- `DrumPattern` (grid, resolution, swing, humanize, `to_notes`);
- `BeatPatterns` (song mode: set/add/duplicate/delete).

### Interface

- `Workspace` completo com splitters, painéis dockados/flutuantes;
- 7 temas com troca em runtime e persistência;
- Cursores de ferramenta Win32 (gerados de SVG no build);
- Beat sequencer (12 componentes), mixer (5), sampler (18), explorer (7);
- Clip Pool dockado/flutuante + seção BEAT PATTERNS;
- Drag & drop do SO com pré-decode no hover;
- Zoom horizontal amortecido com âncora fixa.

### Renderer

- `timeline-render`: renderer wgpu standalone (shader WGSL, instancing,
  storage buffer de waveform, régua com fonte bitmap 8×8, subdivisões de grid
  por zoom);
- `app/src/timeline_gpu/`: renderer headless embutido, composto com o overlay
  Slint GPU→GPU.

---

## 2. Placeholders (existem como crate, sem implementação)

| Crate | Estado |
|---|---|
| `daw-project` | **vazio** — formato de arquivo de projeto ainda não definido |
| `daw-plugins` | **vazio** — apenas o doc-comment "plugin host (VST3/CLAP, Fase futura)" |
| `ui/piano_roll/` | **vazio** — o piano roll é desenhado pela engine wgpu, não por Slint |

**Persistência de projeto não existe.** Não há "salvar/abrir projeto" — o
`Project` vive em memória. `theme.cfg` é a **única** coisa persistida em disco
(índice do tema, ao lado do executável).

---

## 3. Pendências conhecidas (declaradas no código)

| Pendência | Onde | Natureza |
|---|---|---|
| Integração UI do hit-testing de timeline | `daw-core/src/hit.rs` + `app/src/hit.rs` | scaffold pronto; falta ligar em glue/marquee/move-resize |
| Importar waveform real no `timeline-render` | `timeline-render/` | hoje usa cena demo |
| Manutenção dual do renderer | `timeline-render/src/main.rs` × `app/src/timeline_canvas.rs` | qualquer mudança de grid/header/paleta precisa ser aplicada nos dois |
| Plugins VST3/CLAP | `daw-plugins` | dependências já no app (`libloading`, `reqwest`, `zip`, `sha2`) |

---

## 4. Riscos ativos

1. **Manutenção dual do renderer** (ADR-001). É o risco estrutural mais citado
   no próprio código.
2. **`ui/mixer/mixer_channel.slint` já quebrou o build** por desbalanceamento
   de chaves (51 vs 52). Corrigido, mas o pitfall do build.rs permanece: erro
   de sintaxe `.slint` reporta linha errada.
3. **Buffer de áudio do cpal**: `BufferSize::Default` era traduzido para 512
   frames fixos (~10,6 ms a 48k) — pouco para um app que também renderiza
   Skia+wgpu, causando áudio granulado. Corrigido para consultar
   `supported_output_configs()` e escolher ≥1024. **Falta teste de escuta.**
4. **Sem persistência de projeto** — todo o trabalho do usuário se perde ao
   fechar o app.

---

## 5. Próximos passos recomendados

Em ordem de dependência:

1. **Fechar o ciclo do áudio** — validar a correção do buffer cpal com teste
   de escuta real.
2. **Ligar o hit-testing na UI** — o scaffold de `hit.rs` já existe; falta
   consumi-lo em glue/marquee/move-resize.
3. **Definir o formato de projeto** (`daw-project`) — é o que destrava "salvar/
   abrir" e é pré-requisito para qualquer uso real.
4. **Sincronizar o renderer** — resolver a manutenção dual extraindo o código
   comum entre standalone e headless.

---

## 6. Como manter este documento

Ao concluir uma tarefa que muda arquitetura, contratos, dependências ou
procedimento:

1. atualizar o documento temático correspondente;
2. mover o item de §3 (pendência) para §1 (implementado) **somente** se o
   código estiver no repositório e verificado;
3. registrar a decisão em [[Decisões Arquiteturais]] se ela for estrutural;
4. atualizar a data de "última verificação".

**Não registrar como implementado algo que existe apenas no roadmap.**

---

## Links

- [[Arquitetura Geral]]
- [[Decisões Arquiteturais]]
- [[Workspace Rust e Crates]]
- [[Build, Testes e Verificação]]
- [[Projeto e Persistência]]
- [[Plugins e Extensibilidade]]
