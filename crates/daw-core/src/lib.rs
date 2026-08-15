//! daw-core — núcleo independente da interface (Fase 2).
//! Contém o modelo conceitual da DAW. Nenhuma dependência de UI/Slint.

pub mod clip;
pub mod project;
pub mod track;
pub mod transport;

pub use clip::{Clip, ClipKind};
pub use project::Project;
pub use track::{Track, TrackKind};
pub use transport::Transport;
