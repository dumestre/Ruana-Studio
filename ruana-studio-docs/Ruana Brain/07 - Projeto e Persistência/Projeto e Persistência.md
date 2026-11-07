# Projeto e Persistência

> Modelo de documento do RuanaStudio: Project, Clip, Track, Mixer e histórico de mutações.

---

## Fontes

- `crates/daw-core/src/project.rs` — `Project`, `DocumentEvent`, `ChangeKind`
- `crates/daw-core/src/clip.rs` — `Clip`, `ClipKind`, `TimeMode`, `WarpMarker`
- `crates/daw-core/src/track.rs` — `Track`, `TrackKind`
- `crates/daw-core/src/mixer.rs` — `MixerChannel`, `MixerParam`, `InsertSlot`
- `crates/daw-core/src/history.rs` — `History`
- `crates/daw-project/src/lib.rs` — placeholder (fase futura)

## Project

Fonte de verdade do documento. Emite eventos a cada mutação.

```rust
pub struct Project {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub tempo_map: TempoMap,
    pub master_mixer: MixerChannel,
    pub beat_patterns: BeatPatterns,
    history: History,
    subscribers: RefCell<Vec<Option<Box<dyn Fn(&DocumentEvent, ChangeKind)>>>>,
}
```

### Fábricas

| Método | Descrição |
|---|---|
| `Project::startup()` | Projeto inicial: 1 trilha de áudio vazia |
| `Project::demo()` | Projeto com dados fictícios (Drums/Bass/Vocal/Synth) |

### Assinatura de eventos

```rust
fn subscribe(f: impl Fn(&DocumentEvent, ChangeKind) + 'static) -> usize
fn unsubscribe(handle: usize)
```

A UI assina e mantém seus modelos sincronizados.

### Undo/Redo

```rust
fn can_undo() -> bool
fn can_redo() -> bool
fn undo() -> bool
fn redo() -> bool
```

## DocumentEvent

Evento emitido a cada mutação. Carrega dados antes/depois para UI e undo/redo.

| Evento | Descrição |
|---|---|
| `TrackAdded(Track)` | Trilha adicionada |
| `ClipAdded(Clip)` | Clip adicionado |
| `ClipSliced { original, left, right }` | Clip cortado |
| `ClipMoved { old, new }` | Clip movido |
| `ClipDeleted(Clip)` | Clip removido |
| `ClipsGlued { left_before, left, right }` | Clips adjacentes fundidos |
| `ClipResized { old, new }` | Clip redimensionado |
| `ClipRenamed { old, new }` | Clip renomeado |
| `ClipColorChanged { old, new }` | Cor alterada |
| `ClipFadesChanged { old, new }` | Fades alterados |
| `ClipGainChanged { old, new }` | Ganho alterado |
| `ClipNotesChanged { old, new }` | Notas MIDI alteradas |
| `ClipTimewarpChanged { old, new }` | Timewarp alterado |
| `TempoChanged { old, new }` | Mapa de tempo alterado |
| `MixerInsertsChanged { channel, old, new }` | Inserts do mixer alterados |

## Clip

```rust
pub struct Clip {
    pub id: u64,
    pub track_id: u64,
    pub name: String,
    pub kind: ClipKind,           // Audio | Midi
    pub start_beat: f32,          // posição em beats
    pub length_beats: f32,        // duração em beats
    pub source_start_beats: f30,  // offset no arquivo (áudio)
    pub color: u8,                // 0 = padrão, 1..=20 = paleta
    pub color_override: bool,      // true = cor manual
    pub file_path: Option<String>,
    pub fade_in_beats: f32,
    pub fade_out_beats: f32,
    pub gain_db: f32,
    pub time_mode: TimeMode,      // Musical | Linear
    pub warp_markers: Vec<WarpMarker>,
    pub notes: Vec<Note>,         // só MIDI
}
```

### TimeMode

| Modo | Descrição |
|---|---|
| `Musical` (default para áudio importado) | Áudio esticado para casar com a grade. BPM/timewarp muda a duração. |
| `Linear` (default) | Áudio toca na velocidade natural. Span decide quanto do arquivo é ouvido. |

### Cores determinísticas

```rust
fn sample_color(path: &str) -> u8    // FNV-1a do path (case-insensitive)
fn pattern_color(name: &str) -> u8    // FNV-1a do nome do padrão
```

Mesmo arquivo → mesma cor, estável entre projetos.

### Fábricas

```rust
Clip::new(id, track_id, name, kind, start_beat, length_beats)
Clip::midi(id, track_id, name, start_beat, length_beats)
Clip::audio(id, track_id, name, start_beat, length_beats)
```

## Track

```rust
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
    pub color: u8,
    pub mixer: MixerChannel,
}
```

### TrackKind

| Kind | MIDI? | Nome base |
|---|---|---|
| `Audio` | não | "Audio" |
| `Midi` | sim | "MIDI" |
| `Instrument` | sim | "Instrument" |
| `Drum` | sim | "Drums" |
| `Fx` | não | "FX" |
| `Bus` | não | "Bus" |
| `Master` | não | "Master" |

## Mixer

```rust
pub struct MixerChannel {
    pub volume: f32,    // 0..1 (default 1.0 = unity)
    pub pan: f32,       // 0..1 (default 0.5 = centro)
    pub muted: bool,
    pub soloed: bool,
    pub armed: bool,
    pub inserts: Vec<InsertSlot>,
}

pub struct InsertSlot {
    pub name: String,
    pub enabled: bool,
}

pub enum MixerParam { Volume, Pan, Muted, Soloed, Armed }
```

## History

```rust
pub struct History {
    undo_stack: Vec<DocumentEvent>,
    redo_stack: Vec<DocumentEvent>,
}
```

Cada mutação registra um `DocumentEvent` que carrega os dados de antes/depois. Undo reverte, redo reaplica.

## Persistência (daw-project)

**Fase futura** — placeholder. O crate `daw-project` está vazio. O formato de arquivo do projeto ainda não foi definido.

## Integração

- `Project::subscribe()` permite que a UI mantenha modelos Slint sincronizados
- `DocumentEvent` carrega dados suficientes para undo/reprojetar
- `app/src/playback.rs` consome `Transport` e `Project` para reprodução
- `app/src/timeline_slint.rs` projeta clips/tracks na UI

## Testes

- `daw-core/src/project.rs`: testes de undo/redo, find_free_slot, track_with_room
- `daw-core/src/clip.rs`: 5 testes (end_beat, overlaps, beats_eq, sample_color)
- `daw-core/src/track.rs`: 2 testes (midi_kinds, track_is_midi)
- `daw-core/src/mixer.rs`: 4 testes (defaults, set_clamp, from_str, inserts)
- `daw-core/src/history.rs`: testes de pilha undo/redo

## Links

- [[MIDI e Sequenciador]]
- [[Timeline e Transporte]]
- [[Engine de Áudio]]
- [[Plugins e Extensibilidade]]
