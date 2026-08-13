//! daw-core — núcleo independente da interface (Fase 2).
//! Contém o modelo conceitual da DAW. Nenhuma dependência de UI/Slint.

pub mod clip;
pub mod project;
pub mod track;

pub use clip::{Clip, ClipKind};
pub use project::Project;
pub use track::{Track, TrackKind};
