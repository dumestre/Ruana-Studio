//! Tipos de trilha da DAW.

use crate::mixer::MixerChannel;

/// Tipo de uma trilha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Trilha de áudio.
    Audio,
    /// Trilha MIDI.
    Midi,
    /// Trilha de instrumento (MIDI + synth).
    Instrument,
    /// Trilha de bateria (MIDI/step).
    Drum,
    /// Trilha de efeitos (processamento).
    Fx,
    /// Trilha barramento (routing).
    Bus,
    /// Trilha master.
    Master,
}

impl TrackKind {
    /// True se a trilha carrega eventos MIDI.
    pub fn is_midi(self) -> bool {
        matches!(self, TrackKind::Midi | TrackKind::Instrument | TrackKind::Drum)
    }

    /// Nome base legível usado para nomear novas trilhas.
    pub fn base_name(self) -> &'static str {
        match self {
            TrackKind::Audio => "Audio",
            TrackKind::Midi => "MIDI",
            TrackKind::Instrument => "Instrument",
            TrackKind::Drum => "Drums",
            TrackKind::Fx => "FX",
            TrackKind::Bus => "Bus",
            TrackKind::Master => "Master",
        }
    }
}

/// Uma trilha da DAW.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
    /// Índice de cor na paleta da UI (0 = padrão por tipo).
    pub color: u8,
    /// Parâmetros do canal no mixer.
    pub mixer: MixerChannel,
}

impl Track {
    /// Cria uma nova trilha.
    pub fn new(id: u64, name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            color: ((id % 19) + 1) as u8,
            mixer: MixerChannel::default(),
        }
    }

    /// True se a trilha é MIDI.
    pub fn is_midi(&self) -> bool {
        self.kind.is_midi()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_kinds() {
        assert!(TrackKind::Midi.is_midi());
        assert!(TrackKind::Instrument.is_midi());
        assert!(TrackKind::Drum.is_midi());
        assert!(!TrackKind::Audio.is_midi());
        assert!(!TrackKind::Fx.is_midi());
        assert!(!TrackKind::Bus.is_midi());
        assert!(!TrackKind::Master.is_midi());
    }

    #[test]
    fn track_is_midi_reflects_kind() {
        let midi = Track::new(0, "Bass", TrackKind::Midi);
        let audio = Track::new(1, "Vocal", TrackKind::Audio);
        assert!(midi.is_midi());
        assert!(!audio.is_midi());
    }
}
