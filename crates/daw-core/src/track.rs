//! Tipos de trilha da DAW.

/// Tipo de uma trilha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Trilha de áudio.
    Audio,
    /// Trilha MIDI.
    Midi,
}

/// Uma trilha da DAW.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
}

impl Track {
    /// Cria uma nova trilha.
    pub fn new(id: u64, name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
        }
    }

    /// True se a trilha é MIDI.
    pub fn is_midi(&self) -> bool {
        self.kind == TrackKind::Midi
    }
}
