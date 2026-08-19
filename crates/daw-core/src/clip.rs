//! Clips da DAW.

/// Tipo de conteúdo de um clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// Clip de áudio.
    Audio,
    /// Clip MIDI.
    Midi,
}

/// Um clip posicionado na timeline.
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: u64,
    /// Trilha à qual o clip pertence.
    pub track_id: u64,
    pub name: String,
    pub kind: ClipKind,
    /// Posição inicial em beats na timeline.
    pub start_beat: u32,
    /// Duração em beats.
    pub length_beats: u32,
    /// Índice de cor (paleta na UI). 0 = padrão MIDI/Audio.
    pub color: u8,
    /// Caminho do arquivo de áudio (apenas clips de áudio).
    pub file_path: Option<String>,
}

impl Clip {
    /// Cria um novo clip.
    pub fn new(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        kind: ClipKind,
        start_beat: u32,
        length_beats: u32,
    ) -> Self {
        Self {
            id,
            track_id,
            name: name.into(),
            kind,
            start_beat,
            length_beats,
            color: 0,
            file_path: None,
        }
    }

    /// Cria um clip MIDI.
    pub fn midi(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        start_beat: u32,
        length_beats: u32,
    ) -> Self {
        Self::new(
            id,
            track_id,
            name,
            ClipKind::Midi,
            start_beat,
            length_beats,
        )
    }

    /// Cria um clip de áudio.
    pub fn audio(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        start_beat: u32,
        length_beats: u32,
    ) -> Self {
        Self::new(
            id,
            track_id,
            name,
            ClipKind::Audio,
            start_beat,
            length_beats,
        )
    }

    /// True se o clip é MIDI.
    pub fn is_midi(&self) -> bool {
        self.kind == ClipKind::Midi
    }

    /// Define o caminho do arquivo de áudio (builder pattern).
    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }
}
