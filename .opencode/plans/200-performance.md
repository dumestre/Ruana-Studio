# Plano — Playhead + Waveform (parte por parte, push a cada parte)

## Contexto / decisões do usuário
- Playhead: **projeção pura sem servo** (estilo PlayheadLab) — correções só em seek/transports e drift > 0.25s.
- RAM: para arquivos grandes, guardar **mono down-sampled** (~1 sample por 1k) em vez de mono completo.
- Escopo: **Fases 1 (playhead) + 2 (waveform)**, feitas **em partes**, com **commit + push por parte**.
- Esquema de nomes de commit: numeração sequencial (`git log`). Atual: `199 Base` → próximo **200**, depois 201, 202.
- Branch `master`, árvore de trabalho limpa hoje.

## Esqueleto de commits
| Nº | Parte | Conteúdo |
|----|-------|----------|
| 200 | Playhead puro | playback.rs: remover servo/nudge; âncora+elapsed linear; resync só em seek/bpm/glitch >0.25s |
| 201 | Waveform mipmap | caches passam a guardar mono downsample + níveis min/max; zoom escolhe nível por samples/px |
| 202 | Waveform offload | raster em worker thread + debounce ~40ms; sampler preview assíncrono (fora da UI) |
| 203 | Limpeza | remover logs de debug (`[drop]`, `[slint] DRAG-END`, `debug-log`) após validação do usuário |

---

## Fase 1 — Playhead por projeção pura (commit 200)
Arquivo único: `app/src/playback.rs`.

Diagnóstico:
- Cadeia de render ok: `timeline.slint:256 playhead-x` = `left-padding + current-time-seconds * px-per-seg`, via `PlayheadOverlay` isolado.
- Jitter vem do **servo de posição** no timer 8ms (`VisualClock::nudge`): a cada tick a âncora absorve 15% do drift → micro-correções → movimento não-linear ("pulando").
- playheadlab (33ms, projeção pura) parece liso justamente por não ter servo.

Mudanças:
1. `VisualClock`: remover `nudge()`. Manter `resync(secs)` (re-ancora: play, seek, bpm) e `projected()` = `anchor_secs + anchor_at.elapsed()` (taxa 1.0).
2. Timer 8ms: substituir o ramo `nudge` por **nada** — projeção linear pura; re-ancorar `resync(actual)` **só** quando `|drift| > 0.25` (glitch real de áudio).
3. Atualizar doc header (remover descrição de servo/PLL).
4. NÃO mexer: `beat_only`, LCD dedup, `publish_beat_step` (deriva do mesmo relógio → alinhamento beat grid + agulha preservado por construção), timer de meters.

Verificação: `cargo build -p ruana-studio`; rodar e pedir feedback visual no play (timeline + beat só-beat). Se ainda "pular", medir antes/depois.

---

## Fase 2 — Waveform: mipmap + offload (commits 201–202)

Diagnóstico:
- Zoom re-renderiza RGBA **na thread da UI** com debounce 120ms (`app/src/waveform.rs:150`) e reconstroi linha via `set_row_data`.
- `app/src/clip_cache.rs:164` guarda `mono: Vec::new()` → zoom cai em fallback de 256–2048 picos agregados (borrão).
- Sampler preview decodifica **síncrono na UI** a cada seleção de arquivo (`main.rs` → `decode_audio`).
- GPU não é gargalo: compositor já é Skia/GPU; custo é rasterização CPU + decode na event loop.

### Commit 201 — dados: mono downsample + mipmap min/max
1. `CachedWave` (`app/src/waveform.rs`): adicionar `mono_ds: Vec<f32>` (downsample ~1/1k, teto de RAM) e `levels: Vec<Vec<(f32, f32)>>` (picos min/max hierárquicos, níveis ~2x, gerados 1x pós-decode).
2. Preencher no import: `importer.rs` (rota pura + `import_from_entry`) e `clip_cache.rs decode_entry` (parar de guardar `mono` vazio; guardar `mono_ds` + `levels`).
3. Zoom (`wire_resolution_requests`): escolher nível por samples/px (`view`/`width`); raster O(samples visíveis) em vez de re-varrer o arquivo.

### Commit 202 — offload da thread da UI
1. Raster do re-render (`waveform_image_rgba_at_samples`) num **worker thread** (std::thread), postar `Vec<u8>`+dims de volta via `Weak`+event loop single-shot (criação de `slint::Image` fica na UI, mas só envolve o buffer).
2. Debounce 120ms → ~40ms (raster agora é rápido e fora da UI; imagem antiga esticada no máximo 1–2 frames).
3. Sampler preview: decode + picos fora da UI (reusar worker/pool), placeholder nítido imediato.

Verificação: `cargo build` + teste de zoom contínuo (sem blur persistente, sem travar playhead enquanto importa/colore).

---

## Fase 3 — Import/paste sustains (sem commit próprio; consequência)
- `importer.rs:68 decode_audio` na UI já sai do caminho pelo offload do 202 (worker para decode do caminho puro).
- Paste (Ctrl+C/V) já é memória pura (`clip_cache`); com raster fora da UI, `set_row_data` deixa de competir com o tick de 8ms.

## Riscos / atenção
- Alarm falso de "drift": projeção pura acompanha o relógio de parede; deriva vs relógio de áudio é da ordem de ppm (≤ ~3ms/min) — irrelevante para o alinhamento beat/grid em escala de música. Gate 0.25s continua para underruns/glitches.
- `mono_ds` downsampling: garante re-render real em qualquer nível de zoom; mipmaps guardam o detalhe.