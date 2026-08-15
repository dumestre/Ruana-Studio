//! Projeto: o modelo completo da DAW (trilhas + clips).
//!
//! O `Project` é a fonte de verdade do documento e **emite
//! eventos** a cada mutação. A camada de UI assina esses eventos
//! e mantém seus modelos sincronizados; o histórico de
//! undo/redo vive aqui também (UI-agnostic).
//!
//! Fluxo: UI emite intenção (callback) → `Project` muta e emite
//! `DocumentEvent` → assinante (glue) reprojeta para a UI.

use std::cell::RefCell;

use crate::clip::{Clip, ClipKind};
use crate::history::History;
use crate::mixer::{MixerChannel, MixerParam};
use crate::track::{Track, TrackKind};
use daw_midi::{BeatPatterns, DrumPattern};

/// Tipo de mudança de um evento do documento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Mutação aplicada (ou reaplicada no redo).
    Apply,
    /// Mutação revertida (undo).
    Undo,
}

/// Evento emitido a cada mutação do documento.
///
/// Carrega os dados antes/depois necessários tanto para a UI se
/// atualizar quanto para undo/redo.
#[derive(Debug, Clone)]
pub enum DocumentEvent {
    TrackAdded(Track),
    ClipAdded(Clip),
    ClipSliced {
        /// Clip antes do corte (para restaurar no undo).
        original: Clip,
        /// Clip esquerdo após o corte (estado atual).
        left: Clip,
        /// Clip direito criado pelo corte.
        right: Clip,
    },
    ClipMoved {
        /// Estado antes do movimento.
        old: Clip,
        /// Estado após o movimento.
        new: Clip,
    },
    ClipDeleted(Clip),
}

/// Um projeto da DAW.
pub struct Project {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    /// Canal master do mixer.
    pub master_mixer: MixerChannel,
    /// Padrões de bateria / step sequencer.
    pub beat_patterns: BeatPatterns,
    history: History,
    subscribers: RefCell<Vec<Option<Box<dyn Fn(&DocumentEvent, ChangeKind)>>>>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            clips: Vec::new(),
            master_mixer: MixerChannel::default(),
            beat_patterns: BeatPatterns::new(),
            history: History::default(),
            subscribers: RefCell::new(Vec::new()),
        }
    }
}

impl Project {
    /// Projeto placeholder com dados fictícios.
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

        Self {
            tracks,
            clips,
            beat_patterns: {
                let mut bp = BeatPatterns::new();
                bp.add_pattern(DrumPattern::new(
                    "BEAT SEQUENCER",
                    8,
                    16,
                ));
                bp
            },
            ..Default::default()
        }
    }

    // =====================================================
    // EVENTOS
    // =====================================================

    /// Assina as mudanças do documento. Retorna um handle para
    /// `unsubscribe`.
    pub fn subscribe(&self, f: impl Fn(&DocumentEvent, ChangeKind) + 'static) -> usize {
        let mut subscribers = self.subscribers.borrow_mut();
        if let Some(index) = subscribers.iter().position(|s| s.is_none()) {
            subscribers[index] = Some(Box::new(f));
            return index;
        }
        subscribers.push(Some(Box::new(f)));
        subscribers.len() - 1
    }

    /// Para de receber eventos. Id seguro de chamar mais de uma vez.
    pub fn unsubscribe(&self, handle: usize) {
        if let Some(slot) = self.subscribers.borrow_mut().get_mut(handle) {
            *slot = None;
        }
    }

    fn emit(&self, event: &DocumentEvent, kind: ChangeKind) {
        let subscribers = self.subscribers.borrow();
        for slot in subscribers.iter().flatten() {
            slot(event, kind);
        }
    }

    // =====================================================
    // HISTÓRICO
    // =====================================================

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Desfaz a última mutação.
    pub fn undo(&mut self) -> bool {
        let Some(event) = self.history.pop_undo() else {
            return false;
        };
        self.apply_undo(&event);
        self.history.push_redo(event);
        true
    }

    /// Refaz a última mutação desfeita.
    pub fn redo(&mut self) -> bool {
        let Some(event) = self.history.pop_redo() else {
            return false;
        };
        self.apply_event(&event);
        self.history.push_undo(event);
        true
    }

    /// Reaplica uma mutação (redo / aplicação direta).
    fn apply_event(&mut self, event: &DocumentEvent) {
        match event {
            DocumentEvent::TrackAdded(track) => {
                self.tracks.push(track.clone());
            }
            DocumentEvent::ClipAdded(clip) => {
                self.clips.push(clip.clone());
            }
            DocumentEvent::ClipSliced { left, right, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == left.id) {
                    *existing = left.clone();
                }
                self.clips.push(right.clone());
            }
            DocumentEvent::ClipMoved { new, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == new.id) {
                    *existing = new.clone();
                }
            }
            DocumentEvent::ClipDeleted(clip) => {
                self.clips.retain(|c| c.id != clip.id);
            }
        }
        self.emit(event, ChangeKind::Apply);
    }

    /// Reverte uma mutação (undo).
    fn apply_undo(&mut self, event: &DocumentEvent) {
        match event {
            DocumentEvent::TrackAdded(track) => {
                self.tracks.retain(|t| t.id != track.id);
            }
            DocumentEvent::ClipAdded(clip) => {
                self.clips.retain(|c| c.id != clip.id);
            }
            DocumentEvent::ClipSliced {
                original, right, ..
            } => {
                self.clips.retain(|c| c.id != right.id);
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == original.id) {
                    *existing = original.clone();
                }
            }
            DocumentEvent::ClipMoved { old, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == old.id) {
                    *existing = old.clone();
                }
            }
            DocumentEvent::ClipDeleted(clip) => {
                self.clips.push(clip.clone());
            }
        }
        self.emit(event, ChangeKind::Undo);
    }

    // =====================================================
    // LEITURA
    // =====================================================

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

    // =====================================================
    // MUTAÇÕES
    //
    // Toda mutação valida antes de aplicar, emite um
    // `DocumentEvent` e registra no histórico.
    // =====================================================

    /// Adiciona uma trilha e retorna o novo id.
    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> u64 {
        let id = self.next_track_id();
        let track = Track::new(id, name, kind);
        self.tracks.push(track.clone());

        let event = DocumentEvent::TrackAdded(track);
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
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
        let clip = Clip::new(id, track_id, name, kind, start_beat, length_beats);
        self.clips.push(clip.clone());

        let event = DocumentEvent::ClipAdded(clip);
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
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

        let original = clip.clone();
        let left = {
            let mut left = original.clone();
            left.length_beats = at_beat - original.start_beat;
            left
        };
        let right_id = self.next_clip_id();
        let right = Clip::new(
            right_id,
            original.track_id,
            original.name.clone(),
            original.kind,
            at_beat,
            original.start_beat + original.length_beats - at_beat,
        );

        self.clips[index] = left.clone();
        self.clips.push(right.clone());

        let event = DocumentEvent::ClipSliced {
            original,
            left,
            right,
        };
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        Some(right_id)
    }

    /// Move um clip para outra trilha/posição.
    ///
    /// Retorna `false` se o clip ou a trilha de destino não existem.
    pub fn move_clip(&mut self, clip_id: u64, new_track_id: u64, new_beat: u32) -> bool {
        if self.track(new_track_id).is_none() {
            return false;
        }
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let old = self.clips[index].clone();
        self.clips[index].track_id = new_track_id;
        self.clips[index].start_beat = new_beat;
        let new = self.clips[index].clone();

        let event = DocumentEvent::ClipMoved { old, new };
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        true
    }

    /// Remove um clip. Retorna `false` se não existe.
    pub fn delete_clip(&mut self, clip_id: u64) -> bool {
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let clip = self.clips.remove(index);

        let event = DocumentEvent::ClipDeleted(clip);
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        true
    }

    /// Altera um parâmetro do mixer.
    ///
    /// `channel`: 0..tracks.len() = trilhas, tracks.len() = master.
    /// Retorna `false` se o canal não existe.
    pub fn set_mixer_param(
        &mut self,
        channel: usize,
        param: MixerParam,
        value: f32,
    ) -> bool {
        let target: &mut MixerChannel = if channel < self.tracks.len() {
            &mut self.tracks[channel].mixer
        } else if channel == self.tracks.len() {
            &mut self.master_mixer
        } else {
            return false;
        };
        target.set(param, value);
        true
    }

    // =====================================================
    // BEAT PATTERNS
    // =====================================================

    /// Referência ao padrão atual (se houver).
    pub fn current_beat_pattern(&self) -> Option<&DrumPattern> {
        self.beat_patterns.current_pattern()
    }

    /// Referência mutável ao padrão atual.
    pub fn current_beat_pattern_mut(&mut self) -> Option<&mut DrumPattern> {
        self.beat_patterns.current_pattern_mut()
    }

    /// Alterna um passo no padrão atual.
    ///
    /// Retorna o novo estado do passo (true = ligado), ou None se não houver padrão.
    pub fn toggle_beat_step(&mut self, track: usize, step: usize) -> Option<bool> {
        self.current_beat_pattern_mut()
            .map(|p| p.toggle_step(track, step))
    }

    /// Limpa uma trilha no padrão atual.
    pub fn clear_beat_track(&mut self, track: usize) -> bool {
        if let Some(p) = self.current_beat_pattern_mut() {
            if track < p.num_tracks() {
                p.clear_track(track);
                return true;
            }
        }
        false
    }

    /// Copia uma trilha para outra no padrão atual.
    pub fn copy_beat_track(&mut self, src: usize, dst: usize) -> bool {
        if let Some(p) = self.current_beat_pattern_mut() {
            if src < p.num_tracks() && dst < p.num_tracks() {
                p.copy_track(src, dst);
                return true;
            }
        }
        false
    }

    /// Duplica uma trilha no padrão atual.
    pub fn duplicate_beat_track(&mut self, track: usize) -> bool {
        if let Some(p) = self.current_beat_pattern_mut() {
            if track < p.num_tracks() {
                p.duplicate_track(track);
                return true;
            }
        }
        false
    }

    /// Remove uma trilha do padrão atual.
    pub fn delete_beat_track(&mut self, track: usize) -> bool {
        if let Some(p) = self.current_beat_pattern_mut() {
            p.delete_track(track)
        } else {
            false
        }
    }

    /// Limpa todo o padrão atual.
    pub fn clear_beat_pattern(&mut self) -> bool {
        if let Some(p) = self.current_beat_pattern_mut() {
            p.clear();
            true
        } else {
            false
        }
    }

    /// Duplica o padrão atual (novo padrão no container).
    pub fn duplicate_beat_pattern(&mut self) {
        self.beat_patterns.duplicate_current();
    }

    /// Remove o padrão atual.
    pub fn delete_beat_pattern(&mut self) -> bool {
        self.beat_patterns.delete_current()
    }

    /// Seleciona um padrão por índice.
    pub fn select_beat_pattern(&mut self, index: usize) -> bool {
        self.beat_patterns.set_current(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

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
        assert_eq!(project.slice_clip(999, 4), None);
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

    #[test]
    fn mutations_emit_events() {
        let mut project = demo();
        let events: Rc<RefCell<Vec<DocumentEvent>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let events = events.clone();
            project.subscribe(move |event, _| {
                events.borrow_mut().push(event.clone());
            });
        }

        project.add_track(TrackKind::Audio, "Vocal 2");
        project.add_clip(0, ClipKind::Midi, "Kick", 0, 1);
        project.move_clip(1, 2, 0);
        project.slice_clip(1, 5);
        project.delete_clip(2);

        let events = events.borrow();
        assert_eq!(events.len(), 5);
        assert!(matches!(events[0], DocumentEvent::TrackAdded(_)));
        assert!(matches!(events[1], DocumentEvent::ClipAdded(_)));
        assert!(matches!(events[2], DocumentEvent::ClipMoved { .. }));
        assert!(matches!(events[3], DocumentEvent::ClipSliced { .. }));
        assert!(matches!(events[4], DocumentEvent::ClipDeleted(_)));
    }

    #[test]
    fn unsubscribe_stops_events() {
        let mut project = demo();
        let count = Rc::new(RefCell::new(0));
        let handle = {
            let count = count.clone();
            project.subscribe(move |_, _| *count.borrow_mut() += 1)
        };
        project.add_clip(0, ClipKind::Midi, "A", 0, 1);
        project.unsubscribe(handle);
        project.add_clip(0, ClipKind::Midi, "B", 1, 1);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn undo_redo_add_clip() {
        let mut project = demo();
        let before = project.clips.len();
        project.add_clip(0, ClipKind::Midi, "Undo", 20, 2);
        assert_eq!(project.clips.len(), before + 1);

        assert!(project.can_undo());
        assert!(project.undo());
        assert_eq!(project.clips.len(), before);
        assert!(project.clip(203).is_none());

        assert!(project.can_redo());
        assert!(project.redo());
        assert_eq!(project.clips.len(), before + 1);
        assert_eq!(project.clip(203).unwrap().name, "Undo");
    }

    #[test]
    fn undo_redo_slice_restores_length() {
        let mut project = demo();
        project.slice_clip(1, 6);

        project.undo();
        let bass = project.clip(1).unwrap();
        assert_eq!(bass.length_beats, 6);
        assert_eq!(project.clips.len(), 6);

        project.redo();
        let left = project.clip(1).unwrap();
        assert_eq!(left.length_beats, 2);
        assert_eq!(project.clips.len(), 7);
    }

    #[test]
    fn undo_redo_move_restores_position() {
        let mut project = demo();
        project.move_clip(1, 3, 0);

        project.undo();
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 1);
        assert_eq!(clip.start_beat, 4);

        project.redo();
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 3);
        assert_eq!(clip.start_beat, 0);
    }

    #[test]
    fn undo_redo_delete_restores_clip() {
        let mut project = demo();
        project.delete_clip(1);

        assert!(project.clip(1).is_none());
        project.undo();
        assert!(project.clip(1).is_some());
        project.redo();
        assert!(project.clip(1).is_none());
    }

    #[test]
    fn undo_redo_add_track() {
        let mut project = demo();
        let id = project.add_track(TrackKind::Drum, "Perc");
        assert_eq!(project.tracks.len(), 5);

        project.undo();
        assert_eq!(project.tracks.len(), 4);
        assert!(project.track(id).is_none());

        project.redo();
        assert_eq!(project.tracks.len(), 5);
        assert!(project.track(id).is_some());
    }

    #[test]
    fn new_mutation_clears_redo_stack() {
        let mut project = demo();
        project.add_clip(0, ClipKind::Midi, "A", 0, 1);
        project.undo();
        assert!(project.can_redo());

        project.add_clip(0, ClipKind::Midi, "B", 1, 1);
        assert!(!project.can_redo());
    }

    #[test]
    fn undo_redo_emit_events() {
        let mut project = demo();
        let kinds: Rc<RefCell<Vec<ChangeKind>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let kinds = kinds.clone();
            project.subscribe(move |_, kind| kinds.borrow_mut().push(kind));
        }

        project.add_clip(0, ClipKind::Midi, "A", 0, 1);
        project.undo();
        project.redo();

        let kinds = kinds.borrow();
        assert_eq!(kinds[0], ChangeKind::Apply);
        assert_eq!(kinds[1], ChangeKind::Undo);
        assert_eq!(kinds[2], ChangeKind::Apply);
    }
}
