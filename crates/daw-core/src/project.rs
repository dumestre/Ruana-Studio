//! Projeto: o modelo completo da DAW (trilhas + clips).

use crate::clip::{Clip, ClipKind};
use crate::track::{Track, TrackKind};

/// Um projeto da DAW.
#[derive(Debug, Clone, Default)]
pub struct Project {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
}

impl Project {
    /// Projeto placeholder com dados fictícios (Fase 1).
    ///
    /// Representa o arranjo mockado usado para testar o fluxo:
    /// trilhas Drums/Bass/Vocal/Synth com clips de exemplo.
    pub fn demo() -> Self {
        let tracks = vec![
            Track::new(0, "Drums", TrackKind::Midi),
            Track::new(1, "Bass", TrackKind::Midi),
            Track::new(2, "Vocal", TrackKind::Audio),
            Track::new(3, "Synth", TrackKind::Midi),
        ];

        let clips = vec![
            Clip::midi(0, 0, "Drums", 0, 4),
            Clip::midi(100, 0, "Drums 02", 8, 4),
            Clip::midi(1, 1, "Bass", 4, 6),
            Clip::audio(2, 2, "Vocal", 8, 8),
            Clip::audio(202, 2, "Vocal Take", 0, 10),
            Clip::midi(3, 3, "Synth", 2, 5),
        ];

        Self { tracks, clips }
    }

    /// Retorna os clips de uma trilha, ordenados por posição.
    pub fn clips_for_track(&self, track_id: u64) -> Vec<&Clip> {
        let mut clips: Vec<&Clip> = self
            .clips
            .iter()
            .filter(|c| c.track_id == track_id)
            .collect();
        clips.sort_by_key(|c| c.start_beat);
        clips
    }

    /// Todos os clips de áudio.
    pub fn audio_clips(&self) -> Vec<&Clip> {
        self.clips
            .iter()
            .filter(|c| c.kind == ClipKind::Audio)
            .collect()
    }

    /// Todos os clips MIDI.
    pub fn midi_clips(&self) -> Vec<&Clip> {
        self.clips
            .iter()
            .filter(|c| c.kind == ClipKind::Midi)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_has_four_tracks() {
        let project = Project::demo();
        assert_eq!(project.tracks.len(), 4);
    }

    #[test]
    fn demo_clips_belong_to_existing_tracks() {
        let project = Project::demo();
        for clip in &project.clips {
            assert!(
                project.tracks.iter().any(|t| t.id == clip.track_id),
                "clip {} em trilha inexistente {}",
                clip.name,
                clip.track_id
            );
        }
    }

    #[test]
    fn demo_track_kinds() {
        let project = Project::demo();
        assert!(project.tracks[0].is_midi());
        assert!(project.tracks[1].is_midi());
        assert!(!project.tracks[2].is_midi());
        assert!(project.tracks[3].is_midi());
    }

    #[test]
    fn clips_for_track_filters_and_sorts() {
        let project = Project::demo();
        let drums = project.clips_for_track(0);
        assert_eq!(drums.len(), 2);
        assert_eq!(drums[0].start_beat, 0);
        assert_eq!(drums[1].start_beat, 8);
    }

    #[test]
    fn demo_audio_clips() {
        let project = Project::demo();
        assert_eq!(project.audio_clips().len(), 2);
    }

    #[test]
    fn demo_midi_clips() {
        let project = Project::demo();
        assert_eq!(project.midi_clips().len(), 4);
    }
}
