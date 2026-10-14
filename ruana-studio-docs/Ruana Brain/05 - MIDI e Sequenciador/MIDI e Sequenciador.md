# MIDI e Sequenciador

> Modelos MIDI do RuanaStudio: notas, piano roll, padrões de bateria e step sequencer.

---

## Fontes

- `crates/daw-midi/src/notes.rs` — `Note`, `NoteEditor`
- `crates/daw-midi/src/pattern.rs` — `DrumPattern`, `BeatPatterns`, `DrumTrackParams`
- `crates/daw-midi/src/lib.rs` — re-exports

## Nota MIDI

```rust
pub struct Note {
    pub id: i32,
    pub pitch: i32,      // 0..127 (60 = C4)
    pub start: f32,      // posição em beats
    pub length: f32,     // duração em beats
    pub velocity: i32,   // 0..127
}
```

## NoteEditor

Fonte de verdade das edições do piano roll. Vive no core, a UI apenas projeta e injeta edições.

| Método | Descrição |
|---|---|
| `add(pitch, start, length, velocity) -> i32` | Adiciona nota, retorna id |
| `move_note(id, start)` | Move posição inicial |
| `change_note_pitch(id, pitch)` | Altera pitch (clamp 0..127) |
| `resize_note(id, length)` | Redimensiona duração |
| `change_note_velocity(id, velocity)` | Altera velocidade (clamp 0..127) |
| `delete(id)` / `delete_many(ids)` | Remove nota(s) |
| `clear_notes()` | Remove todas |
| `quantize(denom)` / `quantize_selected(denom, ids)` | Quantiza ao grid (1/denom beats) |
| `notes_in_rect(x1, y1, x2, y2, ...)` | Hit-test de retângulo (seleção marquee) |
| `replace_notes(notes)` | Substitui conteúdo preservando IDs |

## Padrão de Bateria (DrumPattern)

```rust
pub struct DrumPattern {
    pub name: String,
    pub grid: Vec<Vec<bool>>,      // tracks × steps
    pub resolution: u32,            // passos por compasso
    pub track_params: Vec<DrumTrackParams>,
    pub swing: f32,                // 0..1
    pub humanize: f32,             // 0..1
}
```

### Constante

```rust
pub const DRUM_PITCH_BASE: i32 = 36;
```

Cada trilha ocupa um pitch consecutivo a partir de 36. O motor de áudio mapeia o pitch de volta para a linha do padrão para obter o sample.

### Trilhas padrão (8)

`Kick`, `Snare`, `Clap`, `Closed Hat`, `Open Hat`, `Tom`, `Cymbal`, `Perc`

### Conversão pattern → notas

```rust
pub fn to_notes(&self) -> Vec<Note>
pub fn cell_note(track, step) -> Option<Note>   // geometria independente do estado on/off
pub fn step_note(track, step) -> Option<Note>   // só para célula LIGADA
```

- `cell_note` existe mesmo quando a célula está desligada (para o undo de pintura saber onde a nota estava)
- `step_note` reutiliza a geometria de `cell_note`
- `to_notes` e `cell_note` produzem as mesmas notas (garantia de que pintar e arrastar soam idênticos)

### Duração do padrão

```rust
pub fn length_beats(&self) -> f32 { resolution / 4.0 }
pub fn step_beats(&self) -> f32 { length_beats / resolution }
```

16 passos = 1 compasso = 4 beats. Um padrão de N passos dura N/4 beats na timeline.

### Swing

Atrasa passos ímpares: `swing * 0.5 * step_beats`. Máximo 0.5 * step (swing=1.0).

## BeatPatterns (Song Mode)

```rust
pub struct BeatPatterns {
    pub patterns: Vec<DrumPattern>,
    pub current: usize,
}
```

| Método | Descrição |
|---|---|
| `set_current(idx)` | Seleciona padrão |
| `add_pattern(pattern)` | Adiciona |
| `duplicate_current()` | Duplica padrão atual |
| `delete_current()` | Remove atual (mínimo 1) |
| `current_pattern() -> Option<&DrumPattern>` | Acesso ao atual |

## Integração com o projeto

- `Project` carrega `beat_patterns: BeatPatterns`
- `Clip` MIDI guarda `notes: Vec<Note>` (resultado de `to_notes()` ou pintura passo a passo)
- O beat sequencer toca o pattern ativo com o motor de áudio
- Clips MIDI na timeline são independentes do pattern (podem ser arrastados, cortados, editados)

## Testes

- `daw-midi/src/notes.rs`: 9 testes (IDs, move/resize, clamp, quantize, hit-test)
- `daw-midi/src/pattern.rs`: 8 testes (toggle, bounds, length, to_notes, swing, beat_patterns, cell_note)

## Links

- [[Workspace Rust e Crates]]
- [[Timeline e Transporte]]
- [[Projeto e Persistência]]
