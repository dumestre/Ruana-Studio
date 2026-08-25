//! Clips da DAW.

/// Tolerância para comparar posições em beats.
///
/// Beats são `f32`, então "encostado" nunca é igualdade exata: um clip
/// cortado em 3.7 e recolado precisa reconhecer o vizinho mesmo com o
/// resíduo do arredondamento. 1/1000 de beat é ~0.5 ms em 120 BPM —
/// abaixo de qualquer coisa audível ou clicável.
pub const BEAT_EPSILON: f32 = 0.001;

/// Duas posições em beats são a mesma?
#[inline]
pub fn beats_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= BEAT_EPSILON
}

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
    ///
    /// Fracionário: a duração de um arquivo de áudio quase nunca cai
    /// numa batida inteira, e forçar inteiro esticava ou achatava a
    /// waveform para caber.
    pub start_beat: f32,
    /// Duração em beats.
    pub length_beats: f32,
    /// Posição inicial dentro do arquivo de áudio, em beats do projeto.
    /// Clips MIDI ignoram este campo.
    pub source_start_beats: f32,
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
        start_beat: f32,
        length_beats: f32,
    ) -> Self {
        Self {
            id,
            track_id,
            name: name.into(),
            kind,
            start_beat,
            length_beats,
            source_start_beats: 0.0,
            color: 0,
            file_path: None,
        }
    }

    /// Cria um clip MIDI.
    pub fn midi(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        start_beat: f32,
        length_beats: f32,
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
        start_beat: f32,
        length_beats: f32,
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

    /// Beat onde o clip termina.
    #[inline]
    pub fn end_beat(&self) -> f32 {
        self.start_beat + self.length_beats
    }

    /// O clip cobre o intervalo `[start, end)`?
    ///
    /// Encostar não é sobrepor: um clip que termina exatamente onde o
    /// outro começa é adjacente, e adjacente é legal.
    pub fn overlaps(&self, start: f32, end: f32) -> bool {
        start < self.end_beat() - BEAT_EPSILON && end > self.start_beat + BEAT_EPSILON
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

#[cfg(test)]
mod tests {
    use super::*;

    fn clip_at(start: f32, len: f32) -> Clip {
        Clip::audio(1, 0, "x", start, len)
    }

    #[test]
    fn end_beat_is_start_plus_length() {
        assert_eq!(clip_at(2.5, 1.25).end_beat(), 3.75);
    }

    #[test]
    fn adjacent_clips_do_not_overlap() {
        let c = clip_at(4.0, 2.0);
        // Termina em 6.0: começar em 6.0 é encostar, não sobrepor.
        assert!(!c.overlaps(6.0, 8.0));
        assert!(!c.overlaps(0.0, 4.0));
    }

    #[test]
    fn partial_and_total_overlap_detected() {
        let c = clip_at(4.0, 2.0);
        assert!(c.overlaps(5.0, 7.0)); // entra pela direita
        assert!(c.overlaps(3.0, 5.0)); // entra pela esquerda
        assert!(c.overlaps(0.0, 16.0)); // engole
        assert!(c.overlaps(4.5, 5.5)); // dentro
    }

    #[test]
    fn beats_eq_tolerates_float_residue() {
        assert!(beats_eq(3.7, 3.7 + BEAT_EPSILON * 0.5));
        assert!(!beats_eq(3.7, 3.8));
    }
}
