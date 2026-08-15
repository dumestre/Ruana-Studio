//! Projeto: o modelo completo da DAW (trilhas + clips).
//!
//! O `Project` é a fonte de verdade do documento. Todas as
//! mutações passam por aqui (UI-agnostic): a camada de UI emite
//! intenções (callbacks), que viram chamadas a estes métodos.

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

    // =====================================================
    // MUTAÇÕES
    //
    // Toda mutação valida antes de aplicar e retorna um
    // resultado utilizável pela camada de UI.
    // =====================================================

    /// Próximo ID de trilha (único no documento).
    pub fn next_track_id(&self) -> u64 {
        self.tracks.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    /// Próximo ID de clip (único no documento).
    pub fn next_clip_id(&self) -> u64 {
        self.clips.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }

    /// Localiza uma trilha por id.
    pub fn track(&self, track_id: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == track_id)
    }

    /// Localiza um clip por id.
    pub fn clip(&self, clip_id: u64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == clip_id)
    }

    /// Adiciona uma trilha e retorna o novo id.
    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> u64 {
        let id = self.next_track_id();
        self.tracks.push(Track::new(id, name, kind));
        id
    }

    /// Adiciona um clip na trilha informada.
    ///
    /// Retorna `None` se a trilha não existe ou o tamanho é 0.
    pub fn add_clip(
        &mut self,
        track_id: u64,
        kind: ClipKind,
        name: impl Into<String>,
        start_beat: u32,
        length_beats: u32,
    ) -> Option<u64> {
        if self.track(track_id).is_none() || length_beats == 0 {
            return None;
        }
        let id = self.next_clip_id();
        self.clips.push(Clip::new(
            id,
            track_id,
            name,
            kind,
            start_beat,
            length_beats,
        ));
        Some(id)
    }

    /// Divide um clip em dois no beat informado.
    ///
    /// O clip original encolhe até `at_beat`; o clip da direita
    /// é criado com o restante. Retorna o id do novo clip, ou
    /// `None` se o clip não existe / o corte é inválido.
    pub fn slice_clip(&mut self, clip_id: u64, at_beat: u32) -> Option<u64> {
        let index = self.clips.iter().position(|c| c.id == clip_id)?;
        let clip = &self.clips[index];

        if at_beat <= clip.start_beat || at_beat >= clip.start_beat + clip.length_beats {
            return None;
        }

        let right_len = clip.start_beat + clip.length_beats - at_beat;
        let right_id = self.next_clip_id();
        let right = Clip::new(
            right_id,
            clip.track_id,
            clip.name.clone(),
            clip.kind,
            at_beat,
            right_len,
        );

        self.clips[index].length_beats = at_beat - clip.start_beat;
        self.clips.push(right);
        Some(right_id)
    }

    /// Move um clip para outra trilha/posição.
    ///
    /// Retorna `false` se o clip ou a trilha de destino não existem.
    pub fn move_clip(&mut self, clip_id: u64, new_track_id: u64, new_beat: u32) -> bool {
        if self.track(new_track_id).is_none() {
            return false;
        }
        let clip = match self.clips.iter_mut().find(|c| c.id == clip_id) {
            Some(c) => c,
            None => return false,
        };
        clip.track_id = new_track_id;
        clip.start_beat = new_beat;
        true
    }

    /// Remove um clip. Retorna `false` se não existe.
    pub fn delete_clip(&mut self, clip_id: u64) -> bool {
        let before = self.clips.len();
        self.clips.retain(|c| c.id != clip_id);
        self.clips.len() < before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        Project::demo()
    }

    #[test]
    fn demo_has_four_tracks() {
        let project = demo();
        assert_eq!(project.tracks.len(), 4);
    }

    #[test]
    fn demo_clips_belong_to_existing_tracks() {
        let project = demo();
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
        let project = demo();
        assert!(project.tracks[0].is_midi());
        assert!(project.tracks[1].is_midi());
        assert!(!project.tracks[2].is_midi());
        assert!(project.tracks[3].is_midi());
    }

    #[test]
    fn clips_for_track_filters_and_sorts() {
        let project = demo();
        let drums = project.clips_for_track(0);
        assert_eq!(drums.len(), 2);
        assert_eq!(drums[0].start_beat, 0);
        assert_eq!(drums[1].start_beat, 8);
    }

    #[test]
    fn demo_audio_clips() {
        let project = demo();
        assert_eq!(project.audio_clips().len(), 2);
    }

    #[test]
    fn demo_midi_clips() {
        let project = demo();
        assert_eq!(project.midi_clips().len(), 4);
    }

    #[test]
    fn next_ids_are_unique() {
        let project = demo();
        assert_eq!(project.next_track_id(), 4);
        assert_eq!(project.next_clip_id(), 203);
    }

    #[test]
    fn add_track_assigns_new_id_and_keeps_order() {
        let mut project = demo();
        let id = project.add_track(TrackKind::Drum, "Brass");
        assert_eq!(id, 4);
        assert_eq!(project.tracks.len(), 5);
        assert!(project.track(id).unwrap().is_midi());
    }

    #[test]
    fn add_clip_to_existing_track() {
        let mut project = demo();
        let id = project.add_clip(0, ClipKind::Midi, "Kick", 4, 2).unwrap();
        assert_eq!(id, 203);
        let clip = project.clip(id).unwrap();
        assert_eq!(clip.track_id, 0);
        assert_eq!(clip.start_beat, 4);
        assert_eq!(clip.length_beats, 2);
    }

    #[test]
    fn add_clip_rejects_missing_track_and_zero_len() {
        let mut project = demo();
        assert_eq!(project.add_clip(999, ClipKind::Midi, "X", 0, 4), None);
        assert_eq!(project.add_clip(0, ClipKind::Midi, "X", 0, 0), None);
    }

    #[test]
    fn slice_clip_splits_and_shrinks() {
        let mut project = demo();
        // Bass (id 1) começa no beat 4, len 6 -> corte em 6.
        let new_id = project.slice_clip(1, 6).unwrap();
        assert_eq!(new_id, 203);

        let left = project.clip(1).unwrap();
        assert_eq!(left.start_beat, 4);
        assert_eq!(left.length_beats, 2);

        let right = project.clip(new_id).unwrap();
        assert_eq!(right.start_beat, 6);
        assert_eq!(right.length_beats, 4);
        assert_eq!(right.track_id, 1);
    }

    #[test]
    fn slice_clip_rejects_invalid_cuts() {
        let mut project = demo();
        // id 999 não existe.
        assert_eq!(project.slice_clip(999, 4), None);
        // corte no início/fim é inválido.
        assert_eq!(project.slice_clip(1, 4), None);
        assert_eq!(project.slice_clip(1, 10), None);
    }

    #[test]
    fn move_clip_relocates() {
        let mut project = demo();
        assert!(project.move_clip(1, 3, 0));
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 3);
        assert_eq!(clip.start_beat, 0);
    }

    #[test]
    fn move_clip_rejects_missing_targets() {
        let mut project = demo();
        assert!(!project.move_clip(999, 0, 0));
        assert!(!project.move_clip(1, 999, 0));
    }

    #[test]
    fn delete_clip_removes() {
        let mut project = demo();
        let before = project.clips.len();
        assert!(project.delete_clip(1));
        assert_eq!(project.clips.len(), before - 1);
        assert!(project.clip(1).is_none());
        assert!(!project.delete_clip(1));
    }
}
