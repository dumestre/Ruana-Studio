//! daw-core — núcleo independente da interface (Fase 2).
//! Contém o modelo conceitual da DAW. Nenhuma dependência de UI/Slint.

pub mod clip;
pub mod history;
pub mod mixer;
pub mod project;
pub mod timeline;
pub mod track;
pub mod transport;

pub use clip::{Clip, ClipKind};
pub use history::History;
pub use mixer::{MixerChannel, MixerParam};
pub use project::{ChangeKind, DocumentEvent, Project};
pub use timeline::{Grid, RulerTick};
pub use track::{Track, TrackKind};
pub use transport::Transport;
