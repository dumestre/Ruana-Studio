//! Piano roll: editor de notas MIDI como grade (notas como retângulos
//! posicionados por pitch × tempo). Dados vêm de `daw_midi::NoteEditor`
//! (a mesma fonte de verdade usada pelo Slint); este módulo apenas projeta
//! e injeta edições via renderização wgpu.
//!
//! Submódulos:
//! - [`data`]: tipos e geometria (`PianoRollMetrics`, notas, grid helper).
//! - [`render`]: geração de instâncias (grade, teclado, notas, preview, seleção, splitter).
//! - [`interaction`]: hover, arrasto de notas, marquee, desenho contínuo, splitter.

mod data;
mod interaction;
mod render;

pub use data::{PianoRollMetrics, PianoRollNote};
pub use interaction::PianoRollInteraction;
pub use render::{build_piano_roll_instances, build_splitter_instances, NOTE_HEIGHT};
