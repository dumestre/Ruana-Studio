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
use daw_midi::{BeatPatterns, DrumPattern, DrumTrackParams};

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
    ClipsGlued {
        /// Clip da esquerda antes da fusão (para o undo restaurar o comprimento).
        left_before: Clip,
        /// Clip da esquerda após a fusão (estado atual, comprimento estendido).
        left: Clip,
        /// Clip da direita que foi fundido e removido.
        right: Clip,
    },
    /// Redimensionamento (alargar/encurtar) de clip via handles das bordas.
    ClipResized {
        old: Clip,
        new: Clip,
    },
    /// Mudança de nome do clip (double-click renomear).
    ClipRenamed {
        old: Clip,
        new: Clip,
    },
    /// Mudança de cor (paleta) do clip.
    ClipColorChanged {
        old: Clip,
        new: Clip,
    },
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
    /// Projeto inicial da aplicação: dez trilhas vazias, prontas para uso.
    pub fn startup() -> Self {
        let mut project = Self::default();
        for i in 0..10u64 {
            project.tracks.push(Track::new(i, format!("Track {}", i + 1), TrackKind::Audio));
        }
        project
    }

    /// Projeto placeholder com dados fictícios.
    ///
    /// Representa o arranjo mockado usado para testar o fluxo:
    /// trilhas Drums/Bass/Vocal/Synth com clips de exemplo.
    pub fn demo() -> Self {
        let mut tracks = vec![
            Track::new(0, "Drums", TrackKind::Midi),
            Track::new(1, "Bass", TrackKind::Midi),
            Track::new(2, "Vocal", TrackKind::Audio),
            Track::new(3, "Synth", TrackKind::Midi),
        ];
        tracks[0].mixer.add_insert("Kick Pro");
        tracks[0].mixer.add_insert("Drum Comp");
        tracks[1].mixer.add_insert("Bass Amp");
        tracks[1].mixer.add_insert("Saturator");
        tracks[2].mixer.add_insert("Vocal Chain");
        tracks[3].mixer.add_insert("Delay");
        tracks[3].mixer.add_insert("Chorus");
        tracks[3].mixer.add_insert("Filter");
        let mut master_mixer = MixerChannel::default();
        master_mixer.add_insert("Master Limiter");

        let clips = vec![
            Clip::midi(0, 0, "Drums", 0.0, 4.0),
            Clip::midi(100, 0, "Drums 02", 8.0, 4.0),
            Clip::midi(1, 1, "Bass", 4.0, 6.0),
            Clip::audio(2, 2, "Vocal", 8.0, 8.0),
            Clip::audio(202, 2, "Vocal Take", 0.0, 10.0),
            Clip::midi(3, 3, "Synth", 2.0, 5.0),
        ];

        Self {
            tracks,
            clips,
            master_mixer,
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
            DocumentEvent::ClipsGlued { left, right, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == left.id) {
                    *existing = left.clone();
                }
                self.clips.retain(|c| c.id != right.id);
            }
            DocumentEvent::ClipResized { new, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == new.id) {
                    *existing = new.clone();
                }
            }
            DocumentEvent::ClipRenamed { new, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == new.id) {
                    *existing = new.clone();
                }
            }
            DocumentEvent::ClipColorChanged { new, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == new.id) {
                    *existing = new.clone();
                }
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
            DocumentEvent::ClipsGlued {
                left_before,
                right,
                ..
            } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == left_before.id) {
                    *existing = left_before.clone();
                }
                self.clips.push(right.clone());
            }
            DocumentEvent::ClipResized { old, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == old.id) {
                    *existing = old.clone();
                }
            }
            DocumentEvent::ClipRenamed { old, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == old.id) {
                    *existing = old.clone();
                }
            }
            DocumentEvent::ClipColorChanged { old, .. } => {
                if let Some(existing) = self.clips.iter_mut().find(|c| c.id == old.id) {
                    *existing = old.clone();
                }
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
        clips.sort_by(|a, b| a.start_beat.total_cmp(&b.start_beat));
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

    /// Clip de uma trilha que contém o beat informado (inclusive), se houver.
    pub fn clip_at_beat(&self, track_id: u64, beat: f32) -> Option<&Clip> {
        self.clips
            .iter()
            .find(|c| c.track_id == track_id && beat >= c.start_beat && beat < c.end_beat())
    }

    /// Primeiro beat livre numa trilha para um clip de `length` beats.
    ///
    /// Se `desired` já está livre, devolve `desired` — a posição pedida
    /// pelo usuário é sempre a primeira escolha. Se está ocupado,
    /// caminha para a direita pelo FIM de cada clip que atravessa,
    /// então o novo clip encosta no vizinho em vez de deixar buraco.
    ///
    /// Importar áudio nunca deve empilhar um clip em cima do outro: o
    /// que fica embaixo vira inaudível e invisível.
    pub fn find_free_slot(&self, track_id: u64, desired: f32, length: f32) -> f32 {
        let length = length.max(crate::clip::BEAT_EPSILON);
        let mut start = desired.max(0.0);

        // Cada iteração salta o clip que bloqueia. Um clip só pode
        // bloquear uma vez (o salto é sempre para o fim dele), então o
        // laço termina em no máximo `clips.len()` passos.
        for _ in 0..=self.clips.len() {
            let blocker = self
                .clips
                .iter()
                .filter(|c| c.track_id == track_id)
                .filter(|c| c.overlaps(start, start + length))
                .max_by(|a, b| a.end_beat().total_cmp(&b.end_beat()));

            match blocker {
                Some(c) => start = c.end_beat(),
                None => return start,
            }
        }
        start
    }

    /// Trilha com espaço livre no intervalo pedido.
    ///
    /// Começa na trilha `preferred` e desce a partir dela, dando a volta
    /// no fim da lista. Descer é o que importar vários arquivos de uma
    /// vez espera: o segundo arquivo cai na trilha de baixo, não numa
    /// trilha aleatória acima de onde o usuário soltou.
    ///
    /// `None` = nenhuma trilha existente cabe; quem chama decide (criar
    /// trilha ou deslocar no tempo).
    pub fn track_with_room(&self, preferred: u64, start: f32, length: f32) -> Option<u64> {
        let end = start + length;
        let free = |id: u64| {
            !self
                .clips
                .iter()
                .any(|c| c.track_id == id && c.overlaps(start, end))
        };

        let usable = |t: &Track| t.kind == TrackKind::Audio || t.kind == TrackKind::Midi;
        if self.track(preferred).is_some_and(usable) && free(preferred) {
            return Some(preferred);
        }

        // Ponto de partida da varredura. Se a preferida existe ela já
        // foi testada acima, então começa na seguinte; se não existe,
        // varre a lista desde o começo.
        let (from, first) = match self.tracks.iter().position(|t| t.id == preferred) {
            Some(i) => (i, 1),
            None => (0, 0),
        };
        let count = self.tracks.len();
        (first..first + count)
            .map(|offset| &self.tracks[(from + offset) % count])
            .filter(|t| usable(t))
            .map(|t| t.id)
            .find(|&id| free(id))
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
        start_beat: f32,
        length_beats: f32,
    ) -> Option<u64> {
        self.add_clip_with_path(track_id, kind, name, start_beat, length_beats, None)
    }

    /// Adiciona um clip na trilha informada, com caminho de arquivo opcional.
    pub fn add_clip_with_path(
        &mut self,
        track_id: u64,
        kind: ClipKind,
        name: impl Into<String>,
        start_beat: f32,
        length_beats: f32,
        file_path: Option<String>,
    ) -> Option<u64> {
        if self.track(track_id).is_none() || length_beats <= crate::clip::BEAT_EPSILON {
            return None;
        }
        let id = self.next_clip_id();
        let mut clip = Clip::new(
            id,
            track_id,
            name,
            kind,
            start_beat.max(0.0),
            length_beats,
        );
        clip.file_path = file_path;
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
    pub fn slice_clip(&mut self, clip_id: u64, at_beat: f32) -> Option<u64> {
        let index = self.clips.iter().position(|c| c.id == clip_id)?;
        let clip = &self.clips[index];

        // Cortar rente à borda não gera clip de tamanho zero.
        if at_beat <= clip.start_beat + crate::clip::BEAT_EPSILON
            || at_beat >= clip.end_beat() - crate::clip::BEAT_EPSILON
        {
            return None;
        }

        let original = clip.clone();
        let left = {
            let mut left = original.clone();
            left.length_beats = at_beat - original.start_beat;
            left
        };
        let right_id = self.next_clip_id();
        let mut right = Clip::new(
            right_id,
            original.track_id,
            original.name.clone(),
            original.kind,
            at_beat,
            original.end_beat() - at_beat,
        );
        right.file_path = original.file_path.clone();
        right.source_start_beats = original.source_start_beats
            + (at_beat - original.start_beat);

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

    /// Funde um clip com o clip adjacente imediatamente à direita na mesma
    /// trilha (mesmo tipo de conteúdo), se existir.
    ///
    /// O clip da esquerda fica com o comprimento somado e o da direita é
    /// removido. Retorna o id do clip removido, ou `None` se não há vizinho.
    pub fn glue_clip(&mut self, clip_id: u64) -> Option<u64> {
        let index = self.clips.iter().position(|c| c.id == clip_id)?;
        let left = self.clips[index].clone();
        let right_index = self
            .clips
            .iter()
            .position(|c| {
                c.track_id == left.track_id
                    && c.kind == left.kind
                    && crate::clip::beats_eq(c.start_beat, left.end_beat())
            })?;
        let right = self.clips[right_index].clone();

        let left_before = left.clone();
        let mut left = left;
        left.length_beats += right.length_beats;

        self.clips[index] = left.clone();
        self.clips.remove(right_index);

        let removed_id = right.id;
        let event = DocumentEvent::ClipsGlued {
            left_before,
            left,
            right,
        };
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        Some(removed_id)
    }

    /// Move um clip para outra trilha/posição.
    ///
    /// Retorna `false` se o clip ou a trilha de destino não existem.
    pub fn move_clip(&mut self, clip_id: u64, new_track_id: u64, new_beat: f32) -> bool {
        if self.track(new_track_id).is_none() {
            return false;
        }
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let old = self.clips[index].clone();
        self.clips[index].track_id = new_track_id;
        self.clips[index].start_beat = new_beat.max(0.0);
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

    /// Redimensiona (alarga/encurta) um clip.
    ///
    /// Muda o `start_beat` e/ou `length_beats`. Impede valores inválidos
    /// (comprimento 0 ou start negativo). Retorna `false` se clip não existe.
    pub fn resize_clip(&mut self, clip_id: u64, new_start: f32, new_length: f32) -> bool {
        if new_length <= crate::clip::BEAT_EPSILON {
            return false;
        }
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let old = self.clips[index].clone();
        self.clips[index].start_beat = new_start.max(0.0);
        self.clips[index].length_beats = new_length;
        // Ao redimensionar pela borda esquerda, a janela visual e a janela
        // de áudio devem avançar/recuar juntas. A borda direita só altera
        // a duração e mantém o ponto de origem.
        let delta_start = self.clips[index].start_beat - old.start_beat;
        self.clips[index].source_start_beats =
            (old.source_start_beats + delta_start).max(0.0);
        let new = self.clips[index].clone();

        let event = DocumentEvent::ClipResized { old, new };
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        true
    }

    /// Renomeia um clip. Retorna `false` se não existe.
    pub fn rename_clip(&mut self, clip_id: u64, new_name: impl Into<String>) -> bool {
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let old = self.clips[index].clone();
        self.clips[index].name = new_name.into();
        let new = self.clips[index].clone();

        let event = DocumentEvent::ClipRenamed { old, new };
        self.emit(&event, ChangeKind::Apply);
        self.history.record(event);
        true
    }

    /// Atualiza o índice de cor (paleta) de um clip. Retorna `false` se não existe.
    pub fn set_clip_color(&mut self, clip_id: u64, color: u8) -> bool {
        let Some(index) = self.clips.iter().position(|c| c.id == clip_id) else {
            return false;
        };
        let old = self.clips[index].clone();
        self.clips[index].color = color;
        let new = self.clips[index].clone();

        let event = DocumentEvent::ClipColorChanged { old, new };
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

    /// Adiciona um insert ao canal do mixer.
    ///
    /// Endereçamento igual a [`set_mixer_param`]. Retorna `false` se o
    /// canal não existe.
    pub fn add_mixer_insert(&mut self, channel: usize) -> bool {
        let target: &mut MixerChannel = if channel < self.tracks.len() {
            &mut self.tracks[channel].mixer
        } else if channel == self.tracks.len() {
            &mut self.master_mixer
        } else {
            return false;
        };
        target.add_insert(format!("Insert {}", target.inserts.len() + 1));
        true
    }

    /// Remove um insert do canal do mixer. Retorna `false` se o canal ou
    /// o slot não existem.
    pub fn remove_mixer_insert(&mut self, channel: usize, index: usize) -> bool {
        let target: &mut MixerChannel = if channel < self.tracks.len() {
            &mut self.tracks[channel].mixer
        } else if channel == self.tracks.len() {
            &mut self.master_mixer
        } else {
            return false;
        };
        target.remove_insert(index)
    }

    /// Alterna o bypass de um insert. Retorna `false` se o canal ou o slot
    /// não existem.
    pub fn toggle_mixer_insert(&mut self, channel: usize, index: usize) -> bool {
        let target: &mut MixerChannel = if channel < self.tracks.len() {
            &mut self.tracks[channel].mixer
        } else if channel == self.tracks.len() {
            &mut self.master_mixer
        } else {
            return false;
        };
        target.toggle_insert(index)
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

    /// Define o estado de um passo no padrão atual. Retorna `true` se gravou.
    pub fn set_beat_step(&mut self, track: usize, step: usize, on: bool) -> bool {
        let Some(p) = self.current_beat_pattern_mut() else {
            return false;
        };
        p.set_step(track, step, on)
    }

    /// Redefine a resolução do padrão atual (preserva passos gravados).
    pub fn set_beat_resolution(&mut self, resolution: u32) -> bool {
        let Some(p) = self.current_beat_pattern_mut() else {
            return false;
        };
        p.set_resolution(resolution);
        true
    }

    /// Insere passos vazios no início de todas as trilhas do padrão atual
    /// (desloca os gravados para a direita).
    pub fn insert_beat_steps(&mut self, count: usize) -> bool {
        let Some(p) = self.current_beat_pattern_mut() else {
            return false;
        };
        p.insert_steps(count);
        true
    }

    /// Parâmetros de uma trilha do padrão atual (mute/solo/pan/volume/
    /// canal/sample/nome), para edição direta pelo bridge.
    pub fn beat_track_params_mut(&mut self, track: usize) -> Option<&mut DrumTrackParams> {
        self.current_beat_pattern_mut()?
            .track_params
            .get_mut(track)
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
        assert_eq!(drums[0].start_beat, 0.0);
        assert_eq!(drums[1].start_beat, 8.0);
    }

    #[test]
    fn clip_at_beat_finds_containing_clip() {
        let project = demo();
        // Clip 1 (Bass) ocupa beats 4..10 na trilha 1.
        assert_eq!(project.clip_at_beat(1, 4.0).map(|c| c.id), Some(1));
        assert_eq!(project.clip_at_beat(1, 9.0).map(|c| c.id), Some(1));
        // Limites: início incluso, fim exclusivo.
        assert!(project.clip_at_beat(1, 3.0).is_none());
        assert!(project.clip_at_beat(1, 10.0).is_none());
        // Trilha errada → None.
        assert!(project.clip_at_beat(0, 4.0).is_none());
        // Overlap entre trilhas não vaza.
        assert_eq!(project.clip_at_beat(2, 8.0).map(|c| c.id), Some(2));
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
        let id = project.add_clip(0, ClipKind::Midi, "Kick", 4.0, 2.0).unwrap();
        assert_eq!(id, 203);
        let clip = project.clip(id).unwrap();
        assert_eq!(clip.track_id, 0);
        assert_eq!(clip.start_beat, 4.0);
        assert_eq!(clip.length_beats, 2.0);
    }

    #[test]
    fn add_clip_rejects_missing_track_and_zero_len() {
        let mut project = demo();
        assert_eq!(project.add_clip(999, ClipKind::Midi, "X", 0.0, 4.0), None);
        assert_eq!(project.add_clip(0, ClipKind::Midi, "X", 0.0, 0.0), None);
    }

    #[test]
    fn slice_clip_splits_and_shrinks() {
        let mut project = demo();
        let new_id = project.slice_clip(1, 6.0).unwrap();
        assert_eq!(new_id, 203);

        let left = project.clip(1).unwrap();
        assert_eq!(left.start_beat, 4.0);
        assert_eq!(left.length_beats, 2.0);
        assert_eq!(left.source_start_beats, 0.0);

        let right = project.clip(new_id).unwrap();
        assert_eq!(right.start_beat, 6.0);
        assert_eq!(right.length_beats, 4.0);
        assert_eq!(right.track_id, 1);
        assert_eq!(right.source_start_beats, 2.0);
    }

    #[test]
    fn slice_clip_rejects_invalid_cuts() {
        let mut project = demo();
        assert_eq!(project.slice_clip(999, 4.0), None);
        assert_eq!(project.slice_clip(1, 4.0), None);
        assert_eq!(project.slice_clip(1, 10.0), None);
    }

    #[test]
    fn glue_clip_merges_adjacent() {
        let mut project = demo();
        let right_id = project.slice_clip(0, 2.0).unwrap();

        let removed = project.glue_clip(0).unwrap();
        assert_eq!(removed, right_id);

        let left = project.clip(0).unwrap();
        assert_eq!(left.start_beat, 0.0);
        assert_eq!(left.length_beats, 4.0);
        assert!(project.clip(right_id).is_none());
        assert_eq!(project.clips.len(), 6);
    }

    #[test]
    fn glue_clip_rejects_non_adjacent() {
        let mut project = demo();
        assert_eq!(project.glue_clip(1), None);
        assert_eq!(project.clip(1).unwrap().length_beats, 6.0);

        let gap = project.add_clip(0, ClipKind::Midi, "Far", 12.0, 2.0).unwrap();
        assert_eq!(project.glue_clip(0), None);
        assert!(project.clip(gap).is_some());
    }

    #[test]
    fn glue_clip_rejects_wrong_kind_or_track() {
        let mut project = demo();
        project.add_clip(1, ClipKind::Audio, "Audio", 10.0, 2.0);
        assert_eq!(project.glue_clip(1), None);

        let other_track = project.add_clip(3, ClipKind::Midi, "Synth 02", 10.0, 2.0).unwrap();
        assert_eq!(project.glue_clip(other_track), None);
    }

    #[test]
    fn glue_clip_rejects_missing_clip() {
        let mut project = demo();
        assert_eq!(project.glue_clip(999), None);
    }

    #[test]
    fn glue_clip_emits_event() {
        let mut project = demo();
        let events: Rc<RefCell<Vec<DocumentEvent>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let events = events.clone();
            project.subscribe(move |event, _| {
                events.borrow_mut().push(event.clone());
            });
        }

        project.slice_clip(0, 2.0);
        project.glue_clip(0);

        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[1], DocumentEvent::ClipsGlued { .. }));
    }

    #[test]
    fn undo_redo_glue_restores_clips() {
        let mut project = demo();
        let right_id = project.slice_clip(0, 2.0).unwrap();
        project.glue_clip(0);

        project.undo();
        let left = project.clip(0).unwrap();
        assert_eq!(left.length_beats, 2.0);
        assert!(project.clip(right_id).is_some());
        assert_eq!(project.clips.len(), 7);

        project.redo();
        let left = project.clip(0).unwrap();
        assert_eq!(left.length_beats, 4.0);
        assert!(project.clip(right_id).is_none());
        assert_eq!(project.clips.len(), 6);
    }

    #[test]
    fn move_clip_relocates() {
        let mut project = demo();
        assert!(project.move_clip(1, 3, 0.0));
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 3);
        assert_eq!(clip.start_beat, 0.0);
    }

    #[test]
    fn move_clip_rejects_missing_targets() {
        let mut project = demo();
        assert!(!project.move_clip(999, 0, 0.0));
        assert!(!project.move_clip(1, 999, 0.0));
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
        project.add_clip(0, ClipKind::Midi, "Kick", 0.0, 1.0);
        project.move_clip(1, 2, 0.0);
        project.slice_clip(1, 5.0);
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
        project.add_clip(0, ClipKind::Midi, "A", 0.0, 1.0);
        project.unsubscribe(handle);
        project.add_clip(0, ClipKind::Midi, "B", 1.0, 1.0);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn undo_redo_add_clip() {
        let mut project = demo();
        let before = project.clips.len();
        project.add_clip(0, ClipKind::Midi, "Undo", 20.0, 2.0);
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
        project.slice_clip(1, 6.0);

        project.undo();
        let bass = project.clip(1).unwrap();
        assert_eq!(bass.length_beats, 6.0);
        assert_eq!(project.clips.len(), 6);

        project.redo();
        let left = project.clip(1).unwrap();
        assert_eq!(left.length_beats, 2.0);
        assert_eq!(project.clips.len(), 7);
    }

    #[test]
    fn undo_redo_move_restores_position() {
        let mut project = demo();
        project.move_clip(1, 3, 0.0);

        project.undo();
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 1);
        assert_eq!(clip.start_beat, 4.0);

        project.redo();
        let clip = project.clip(1).unwrap();
        assert_eq!(clip.track_id, 3);
        assert_eq!(clip.start_beat, 0.0);
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
        project.add_clip(0, ClipKind::Midi, "A", 0.0, 1.0);
        project.undo();
        assert!(project.can_redo());

        project.add_clip(0, ClipKind::Midi, "B", 1.0, 1.0);
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

        project.add_clip(0, ClipKind::Midi, "A", 0.0, 1.0);
        project.undo();
        project.redo();

        let kinds = kinds.borrow();
        assert_eq!(kinds[0], ChangeKind::Apply);
        assert_eq!(kinds[1], ChangeKind::Undo);
        assert_eq!(kinds[2], ChangeKind::Apply);
    }

    #[test]
    fn demo_seeds_inserts() {
        let project = demo();
        assert_eq!(project.tracks[0].mixer.inserts.len(), 2);
        assert_eq!(project.tracks[3].mixer.inserts.len(), 3);
        assert_eq!(project.master_mixer.inserts.len(), 1);
        assert_eq!(project.master_mixer.inserts[0].name, "Master Limiter");
    }

    #[test]
    fn mixer_insert_mutations() {
        let mut project = demo();

        assert!(project.add_mixer_insert(1));
        assert_eq!(project.tracks[1].mixer.inserts.len(), 3);
        assert_eq!(project.tracks[1].mixer.inserts[2].name, "Insert 3");

        assert!(project.toggle_mixer_insert(1, 2));
        assert!(!project.tracks[1].mixer.inserts[2].enabled);

        assert!(project.remove_mixer_insert(1, 2));
        assert_eq!(project.tracks[1].mixer.inserts.len(), 2);

        assert!(project.add_mixer_insert(project.tracks.len()));
        assert_eq!(project.master_mixer.inserts.len(), 2);

        assert!(!project.add_mixer_insert(999));
        assert!(!project.remove_mixer_insert(1, 99));
        assert!(!project.toggle_mixer_insert(1, 99));
    }

    // =====================================================
    // COLOCAÇÃO SEM SOBREPOSIÇÃO
    // =====================================================

    /// Projeto com duas trilhas de áudio vazias. Devolve os ids delas.
    fn two_audio_tracks() -> (Project, u64, u64) {
        let mut project = Project::default();
        let a = project.add_track(TrackKind::Audio, "A");
        let b = project.add_track(TrackKind::Audio, "B");
        (project, a, b)
    }

    #[test]
    fn free_slot_keeps_desired_position_when_empty() {
        let (project, a, _) = two_audio_tracks();
        assert_eq!(project.find_free_slot(a, 3.5, 2.0), 3.5);
    }

    #[test]
    fn free_slot_clamps_negative_desired() {
        let (project, a, _) = two_audio_tracks();
        assert_eq!(project.find_free_slot(a, -8.0, 2.0), 0.0);
    }

    #[test]
    fn free_slot_lands_right_after_the_blocker() {
        let (mut project, a, _) = two_audio_tracks();
        project.add_clip(a, ClipKind::Audio, "um", 0.0, 3.5);

        // Pediu 1.0, mas 0.0..3.5 está ocupado: encosta em 3.5.
        assert_eq!(project.find_free_slot(a, 1.0, 2.0), 3.5);
        // Pediu 3.5, que já está livre: respeita o pedido.
        assert_eq!(project.find_free_slot(a, 3.5, 2.0), 3.5);
    }

    #[test]
    fn free_slot_walks_over_a_chain_of_clips() {
        let (mut project, a, _) = two_audio_tracks();
        project.add_clip(a, ClipKind::Audio, "um", 0.0, 2.0);
        project.add_clip(a, ClipKind::Audio, "dois", 2.0, 2.0);
        project.add_clip(a, ClipKind::Audio, "tres", 4.0, 1.5);

        // Três clips encostados: o quarto vai para depois do último.
        assert_eq!(project.find_free_slot(a, 0.0, 2.0), 5.5);
    }

    #[test]
    fn free_slot_fills_a_gap_between_clips() {
        let (mut project, a, _) = two_audio_tracks();
        project.add_clip(a, ClipKind::Audio, "um", 0.0, 2.0);
        project.add_clip(a, ClipKind::Audio, "dois", 6.0, 2.0);

        // Cabe no buraco 2.0..6.0.
        assert_eq!(project.find_free_slot(a, 2.0, 4.0), 2.0);
        // Não cabe no buraco: pula o segundo clip.
        assert_eq!(project.find_free_slot(a, 2.0, 5.0), 8.0);
    }

    #[test]
    fn free_slot_ignores_other_tracks() {
        let (mut project, a, b) = two_audio_tracks();
        project.add_clip(b, ClipKind::Audio, "outra", 0.0, 8.0);

        assert_eq!(project.find_free_slot(a, 0.0, 4.0), 0.0);
    }

    #[test]
    fn track_with_room_prefers_the_requested_track() {
        let (mut project, a, b) = two_audio_tracks();
        project.add_clip(b, ClipKind::Audio, "outra", 0.0, 8.0);

        assert_eq!(project.track_with_room(a, 0.0, 4.0), Some(a));
    }

    #[test]
    fn track_with_room_falls_back_to_a_free_track() {
        let (mut project, a, b) = two_audio_tracks();
        project.add_clip(a, ClipKind::Audio, "ocupa", 0.0, 8.0);

        assert_eq!(project.track_with_room(a, 0.0, 4.0), Some(b));
    }

    #[test]
    fn track_with_room_returns_none_when_all_busy() {
        let (mut project, a, b) = two_audio_tracks();
        project.add_clip(a, ClipKind::Audio, "a", 0.0, 8.0);
        project.add_clip(b, ClipKind::Audio, "b", 0.0, 8.0);

        assert_eq!(project.track_with_room(a, 0.0, 4.0), None);
        // Depois do fim dos dois, a preferida volta a servir.
        assert_eq!(project.track_with_room(a, 8.0, 4.0), Some(a));
    }

    #[test]
    fn track_with_room_ignores_unknown_preferred() {
        let (project, a, _) = two_audio_tracks();
        assert_eq!(project.track_with_room(999, 0.0, 4.0), Some(a));
    }

    #[test]
    fn track_with_room_searches_downwards_then_wraps() {
        let mut project = Project::default();
        let a = project.add_track(TrackKind::Audio, "A");
        let b = project.add_track(TrackKind::Audio, "B");
        let c = project.add_track(TrackKind::Audio, "C");
        project.add_clip(b, ClipKind::Audio, "ocupa b", 0.0, 4.0);

        // Soltou em B (ocupada) → desce para C, não sobe para A.
        assert_eq!(project.track_with_room(b, 0.0, 4.0), Some(c));

        // Com C também ocupada, dá a volta e chega em A.
        project.add_clip(c, ClipKind::Audio, "ocupa c", 0.0, 4.0);
        assert_eq!(project.track_with_room(b, 0.0, 4.0), Some(a));
    }
}
