# Engine de Áudio

> Pipeline de áudio completo do RuanaStudio: stream cpal, workers, playback, timewarp e meters.

---

## Fontes

- `app/src/audio_bridge.rs` — `AudioBridge` (ponte UI ↔ motor)
- `app/src/playback.rs` — `VisualClock`, instalação dos handlers de transporte
- `app/src/async_sampler.rs` — `SamplerDecode` (load/drop com decode em background)
- `app/src/stretch_worker.rs` — worker de render do timewarp (keylock em background)
- `app/src/decode_worker.rs` — `DecodeWorker` (fila global de decode)
- `app/src/waveform.rs` — `WaveformCache`, `LodBars`, renderização de waveform
- `app/src/meters.rs` — `MeterPump` (bombeamento de medidores)
- `crates/daw-audio/` — motor de áudio (engine)
- `crates/daw-media/waveform` — decodificação via symphonia

## Arquitetura de threads

```
┌─────────────────────────────────────────────────────────┐
│ Thread da UI (event loop)                                │
│  AudioBridge, playback, timers, Slint, wgpu              │
│  Toca no Project, AudioBridge, WaveformCache             │
└────────────┬───────────────────────────────┬────────────┘
             │ mpsc                           │ Arc<Mutex<>>
┌────────────▼────────────┐    ┌────────────▼────────────┐
│ Worker thread           │    │ Thread RT de áudio      │
│ "ruana-decode"           │    │ (callback cpal)         │
│ DecodeWorker (fila)      │    │ Motor de áudio          │
│ SamplerDecode            │    │ Avança Transport        │
│ stretch_worker (threads) │    │ Mistura buffers         │
└─────────────────────────┘    └─────────────────────────┘
```

## AudioBridge

Ponte entre a UI e o motor de áudio. O motor abre o stream no startup e fica aberto até o programa fechar.

**Por que o stream nunca fecha:**
- Abrir stream custa dezenas de ms e estala no primeiro buffer
- Sem stream aberta não existe pré-escuta (arrasta e ouve com transporte parado)

**Consolidação de snapshot:**

```rust
const SYNC_INTERVAL: Duration = Duration::from_millis(90);

// Mutação do documento → mark_dirty (barato)
fn mark_dirty(&self) { self.dirty.set(true); }

// Timer consolida e publica uma vez
fn flush(&self, project: &Project) {
    stretch_worker::publish_ready(handle, project);
    stretch_worker::request_missing(handle, project);
    if !self.dirty.get() { return; }
    self.dirty.set(false);
    handle.sync_project(project);
}
```

## Playback e VisualClock

O motor de áudio avança o `Transport` na thread RT (clock autoritativo). O `VisualClock` vive na UI:

```rust
struct VisualClock {
    anchor_secs: f64,
    anchor_at: Instant,
}

// Projeção LINEAR do tempo — sem servo de posição por tick
fn projected(&self) -> f64 {
    self.anchor_secs + self.anchor_at.elapsed().as_secs_f64()
}
```

- Âncora é re-ancorada em: play, seek, mudança de BPM, ou drift real de áudio (> 0.25 s)
- Timer de 4ms na event loop publica segundos contínuos para a agulha
- LCD (bars:beat:ticks) deduplica por tick inteiro

## Modos de reprodução

| Modo | Descrição |
|---|---|
| `Timeline` | Clips tocam, playhead anda. Step sequencer mudo. |
| `BeatOnly` | Pattern soa, grid acende. Timeline parada. |

- `play()` define Timeline, `play_beat()` define BeatOnly
- Trocar de modo = stop + start (nunca sobreposição)
- STOP: agulha volta para `play_start_ticks`
- Modo BeatOnly NÃO publica posição de playhead (timeline parada)

## DecodeWorker (fila global)

Uma worker thread (`ruana-decode`) consumindo fila global via `mpsc`.

**Trabalhos:**
- `Decode` — decodifica arquivo + metadados (amostras mono)
- `Import` — decode + aparar silêncio + construir LODs

**Regras do import:**
- Threshold de audibilidade: 0.001
- Padding: 2ms (1ms cada lado)
- mínimo de trim: 50ms
- Arquivo todo em silêncio = usa inteiro

**Drainer:** Timer de 16ms na event loop aplica resultados na UI (mover para Slint/core só na UI).

## SamplerDecode (Load/Drop)

Mesmo padrão do DecodeWorker, mas para o channel rack / sampler:

- `Load` — diálogo de arquivo, linha = parâmetro
- `Drop` — explorador/SO, linha = selecionada no momento do apply
- `Clip` — duplo clique em clip de áudio (mostra arquivo, não grava no pattern)

Aplica na UI: `track_params.sample_path = Some(path)`, `refresh_beat()`, `preload()`, `preview()`.

## StretchWorker (Timewarp/Keylock)

Fluxo híbrido (estilo Cubase durante gesto de tempo):

1. Tempo muda → motor toca original em **varispeed** (pitch acompanha)
2. Worker renderiza versão com **pitch preservado** (buffer já em memória)
3. Render pronto → inserido no cache do motor → snapshot republicado → clip toca buffer esticado 1:1

**Threads:**
- `READY` — fila de renders prontos (Mutex)
- `IN_FLIGHT` — alvos em render (evita duplicar)
- `FAILED` — alvos que falharam (não insistir)
- `PUBLISHED` — LRU de chaves publicadas (máx 24)

**Cache do motor:** `insert_buffer`, `evict_buffer`, `has_buffer`, `buffer_for`.

## WaveformCache

LOD multinível de barras min/max por coluna:

```rust
struct LodBars { mins: Vec<f32>, maxs: Vec<f32> }
struct CachedWave { lods: Vec<LodBars>, frames: usize }
```

- Nível mais fino: 16384 colunas
- Níveis em potências de 2 (afinando/engrossando)
- Cache LRU por `clip_id`
- `pick_level(lods, width_px)` escolhe o nível que cobre a largura
- `bake_bars()` aplica ganho linear + fades nas barras

**Renderização:** `render_waveform_bars()` produz RGBA compatível com o pipeline wgpu.

## MeterBank

Banco de medição da engine (pico por canal). A engine escreve, a UI lê.

- `MeterPump` bombeia em intervalo regular
- Latch de clip (vermelho) zerado a cada play
- `signal-fill-only` no track header (transparente sem sinal)

## Regras de thread

| Thread | Pode | Não pode |
|---|---|---|
| UI | Tocar em Project, Slint, wgpu | Decode pesado, symphonia I/O |
| Worker (decode) | Decodificar, construir LODs | Tocar em Slint/core/RT |
| RT (cpal callback) | Avançar Transport, misturar buffers | Alocar, I/O, lock, logging |

## Links

- [[Referência cpal 0.18]]
- [[Referência symphonia 0.6]]
- [[Referência timestretch 0.15]]
- [[Timeline e Transporte]]
- [[Projeto e Persistência]]
- [[wgpu Timeline Rendering]]
