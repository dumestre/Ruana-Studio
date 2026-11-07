# Timeline e Transporte

> Timeline (mapa de tempo, timewarp) e transporte (playhead, modos de reprodução).

---

## Fontes

- `crates/daw-core/src/timeline.rs` — `TempoMap`, `Grid`, `RulerTick`, `TempoChange`
- `crates/daw-core/src/warp.rs` — `WarpPlan`, `WarpSegment`, `plan_for_clip`
- `crates/daw-core/src/transport.rs` — `Transport`, `PlayMode`
- `app/src/playback.rs` — integração com o motor de áudio
- `app/src/timeline.rs` — canvas da timeline (Slint)
- `app/src/timewarp.rs` — ferramenta de time warp

## Transport

```rust
pub struct Transport {
    pub playing: bool,
    pub mode: PlayMode,
    pub position_ticks: u64,
    pub play_start_ticks: u64,
    pub bpm: u32,
    pub ppq: u32,           // pulsos por batida (default 960)
    pub beats_per_bar: u32, // default 4
    pub metronome_enabled: bool,
    pub metronome_sound: u8,
    pub metronome_volume: f32,
    pub loop_enabled: bool,
    pub loop_start_ticks: u64,
    pub loop_end_ticks: u64,
}
```

### PlayMode

| Modo | Descrição |
|---|---|
| `Timeline` (default) | Clips tocam, playhead anda. Step sequencer silenciado. |
| `BeatOnly` | Só beat sequencer. Timeline parada. |

Os dois modos são exclusivos. `play()` define `Timeline`, `play_beat()` define `BeatOnly`. `pause()` volta para `Timeline`.

### Conversão de posição

```rust
fn position_beats() -> f32     // ticks / ppq
fn position_seconds() -> f64   // ticks / ppq * 60 / bpm
fn seek_seconds(secs: f64)    // segundos → ticks
fn ticks_per_bar() -> u64      // ppq * beats_per_bar
```

A UI converte segundos → pixels via `pixels_per_second = pixels_per_beat / seconds_per_beat`, sem quantização para beat/grid.

## TempoMap (Time-Warp)

```rust
pub struct TempoMap {
    pub changes: Vec<TempoChange>,  // sempre começa com beat 0
}

pub struct TempoChange {
    pub beat: f32,
    pub bpm: f32,
}
```

### Conversão beats ↔ segundos

```rust
fn bpm_at(beat: f32) -> f32
fn beats_to_seconds(beat: f32) -> f64
fn seconds_to_beats(seconds: f64) -> f32
fn set_change(beat: f32, bpm: f32)       // insere/atualiza (clamp 1..999)
fn remove_change(beat: f32) -> bool       // não remove beat 0
```

### Time Warp (Cubase)

```rust
fn warp_beat_to_target(musical_beat: f32, target_beat: f32) -> bool
```

Faz a posição musical `musical_beat` soar no instante em que hoje soa `target_beat`. O BPM do trecho `[prev, musical_beat]` é recalculado. O tempo DEPOIS do ponto arrastado não muda.

**Recusa (devolve `false`) quando:**
- Alvo não é anterior a `prev` (não existe BPM finito)
- BPM necessário sai da faixa 20..300

## WarpPlan (Quanto esticar)

```rust
pub struct WarpPlan {
    pub segments: Vec<WarpSegment>,
    pub total_ratio: f64,
}

pub struct WarpSegment {
    pub source_offset_seconds: f64,
    pub source_seconds: f64,
    pub project_offset_beats: f64,
    pub project_beats: f64,
    pub ratio: f64,
}
```

```rust
fn plan_for_clip(clip: &Clip, tempo_map: &TempoMap, region_len_seconds: f64) -> WarpPlan
```

- Calcula quanto cada trecho da fonte deve ser esticado para casar com o span do clip
- Usa o mapa de tempo para converter beats → segundos
- Marcadores de AudioWarp dividem em trechos com ratios próprios
- No modo Linear (default) nada é esticado (ratio 1.0)
- No modo Musical o conteúdo é esticado para caber no span

## Grid (Grade musical)

```rust
pub struct Grid {
    pub first_note: i32,
    pub last_note: i32,
    pub bars: i32,
    pub note_lines: Vec<i32>,
    pub beat_lines: Vec<i32>,
    pub octave_range: Vec<i32>,
    pub bar_range: Vec<i32>,
    pub ruler_ticks: Vec<RulerTick>,
}
```

16 ticks por compasso (1 barra + 3 subdivisões de 1/4 + 4 de 1/8 + 8 de 1/16).

## Integração

- `Project` carrega `tempo_map: TempoMap`
- `Transport` vive no core, UI projeta
- `daw-audio` consome e avança o `Transport` no callback de áudio
- A timeline (`app/src/timeline.rs`) renderiza playhead sincronizado com `Transport.position_seconds()`

## Testes

- `daw-core/src/transport.rs`: 8 testes (default, modos exclusivos, play/pause/toggle, advance, seek, seconds)
- `daw-core/src/timeline.rs`: 12 testes (grid, tempo map, timewarp, warp plan)
- `daw-core/src/warp.rs`: 7 testes (linear, MIDI, musical, marcadores, bpm variável)

## Links

- [[MIDI e Sequenciador]]
- [[Projeto e Persistência]]
- [[Engine de Áudio]]
- [[wgpu Timeline Rendering]]
