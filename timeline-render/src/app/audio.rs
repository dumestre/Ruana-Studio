//! Áudio: carregamento de arquivos, construção do `Project` e sincronização
//! com o motor de áudio.

use daw_core::{Clip as CoreClip, ClipKind, Project};
use std::path::Path;

use crate::app::{WAVE_PPB, App};
use crate::clips;
use crate::fades;
use crate::waveform;

impl App {
    /// Constrói um `daw_core::Project` a partir dos clips da timeline.
    pub fn build_project(&self) -> Project {
        use daw_core::{MixerChannel, Track, TrackKind};

        let track_count = self.clips.iter()
            .map(|c| c.track_index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1)
            .max(1);

        let mut tracks = Vec::with_capacity(track_count);
        for i in 0..track_count {
            tracks.push(Track {
                id: i as u64,
                name: format!("Track {}", i + 1),
                kind: TrackKind::Audio,
                color: 0,
                mixer: MixerChannel::default(),
            });
        }

        let mut clips = Vec::with_capacity(self.clips.len());
        for (idx, clip) in self.clips.iter().enumerate() {
            let core_clip = CoreClip {
                id: idx as u64,
                track_id: clip.track_index as u64,
                name: clip.name.clone().unwrap_or_else(|| format!("Clip {}", idx + 1)),
                kind: if clip.is_midi { ClipKind::Midi } else { ClipKind::Audio },
                start_beat: clip.beat,
                length_beats: clip.len,
                source_start_beats: clip.source_start_beats,
                color: clip.color as u8,
                color_override: false,
                file_path: clip.file_path.clone(),
                fade_in_beats: self.fades.get(idx).map(|f| f.in_len).unwrap_or(0.0),
                fade_out_beats: self.fades.get(idx).map(|f| f.out_len).unwrap_or(0.0),
                gain_db: self.fades.get(idx).map(|f| f.gain_db).unwrap_or(0.0),
                notes: Vec::new(),
            };
            clips.push(core_clip);
        }

        let mut project = Project::default();
        project.tracks = tracks;
        project.clips = clips;
        project.master_mixer = MixerChannel::default();
        project.beat_patterns = daw_midi::BeatPatterns::default();
        project
    }

    /// Sincroniza o projeto atual com o motor de áudio.
    #[cfg(feature = "audio")]
    pub fn sync_audio_engine(&self) {
        if let Some(ref engine) = self.audio_engine {
            let project = self.build_project();
            engine.handle().sync_project(&project);
        }
    }

    /// Importa vários arquivos de áudio, cada um em uma track diferente.
    /// Se o número de arquivos exceder as tracks existentes, cria tracks
    /// adicionais automaticamente. Os clips começam no beat informado.
    pub fn import_audio_files(&mut self, paths: &[String], start_beat: f32) {
        if paths.is_empty() {
            return;
        }

        // A próxima track livre é a que sucede a maior track em uso.
        let start_track = self
            .clips
            .iter()
            .map(|c| c.track_index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);

        // Registra undo UMA vez para o lote inteiro (atomic).
        use daw_media::waveform::decode_audio;
        self.record_undo();

        for (i, path) in paths.iter().enumerate() {
            let track = start_track + i;
            let Ok(data) = decode_audio(Path::new(path)) else {
                eprintln!("falha ao decodificar {}: arquivo inválido", path);
                continue;
            };

            let bpm = self.bpm;
            let beats_per_second = bpm / 60.0;
            let source_length_beats = data.duration_seconds as f32 * beats_per_second;

            let mut clip = clips::ClipDemo {
                id: 0,
                track_index: track,
                beat: start_beat,
                len: source_length_beats,
                is_midi: false,
                color: 1 + (i % 10),
                name: Some(Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Audio")
                    .to_string()),
                seam_beat: None,
                pre_glue_len: None,
                pre_glue_color: None,
                wave: Vec::new(),
                notes: Vec::new(),
                file_path: Some(path.to_string()),
                source_start_beats: 0.0,
                source_length_beats,
                channels: data.channels,
            };

            // Gera a waveform (real ou procedural)
            let n_cols = waveform::wave_cols(clip.len, WAVE_PPB);
            clip.wave = match waveform::decode_real_wave(
                path,
                n_cols,
                clip.source_start_beats,
                clip.source_length_beats,
                bpm,
            ) {
                Ok(w) => w,
                Err(_) => waveform::generate_procedural_wave(&clip, n_cols, self.clips.len() as u64),
            };

            self.clips.push(clip);
            self.fades.push(fades::ClipFade::none());
        }

        self.rebuild_wave_data();
        self.request_redraw();
    }

    /// Importa o lote de arquivos acumulados pelo drag & drop quando há um
    /// período de silêncio (sem novos `DroppedFile`). O winit entrega um evento
    /// por arquivo; aguardar uns milissegundos garante que o lote fique completo.
    pub fn process_pending_drop_import(&mut self) {
        if !self.pending_drop_import {
            return;
        }
        if self.last_drop_instant.elapsed().as_millis() < 80 {
            self.request_redraw();
            return;
        }
        let paths = std::mem::take(&mut self.drop_paths);
        self.pending_drop_import = false;
        if paths.is_empty() {
            return;
        }
        let (beat, _track) = self.cursor_to_snapped_beat_track(self.last_cursor);
        self.import_audio_files(&paths, beat);
    }

    /// Carrega um arquivo de áudio e cria um clip na posição especificada.
    pub fn load_audio_file(&mut self, path: &str, beat: f32, track: usize) {
        // Esta função cria um clip de áudio a partir de um arquivo e garante
        // que o clip tenha um `ClipFade` associado (como os clips placeholder).
        // O fade padrão inclui gain, fade‑in e fade‑out configurados por
        // `ClipFade::default()`, garantindo comportamento visual consistente.

        use daw_media::waveform::decode_audio;

        match decode_audio(Path::new(path)) {
            Ok(data) => {
                self.record_undo();
                let _sample_rate = data.sample_rate;
                let channels = data.channels;
                let bpm = self.bpm;
                let beats_per_second = bpm / 60.0;
                let source_length_beats = data.duration_seconds as f32 * beats_per_second;

                // Cria o clip de áudio
                let mut clip = clips::ClipDemo {
                    id: 0,
                    track_index: track,
                    beat,
                    len: source_length_beats,
                    is_midi: false,
                    color: 0,
                    name: Some(Path::new(path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Audio")
                        .to_string()),
                    seam_beat: None,
                    pre_glue_len: None,
                    pre_glue_color: None,
                    wave: Vec::new(),
                    notes: Vec::new(),
                    file_path: Some(path.to_string()),
                    source_start_beats: 0.0,
                    source_length_beats,
                    channels,
                };
                // Gera a waveform (real ou procedural)
                let n_cols = waveform::wave_cols(clip.len, WAVE_PPB);
                clip.wave = match waveform::decode_real_wave(
                    path,
                    n_cols,
                    clip.source_start_beats,
                    clip.source_length_beats,
                    bpm,
                ) {
                    Ok(w) => w,
                    Err(_) => waveform::generate_procedural_wave(&clip, n_cols, self.clips.len() as u64),
                };
                // Insere o clip
                self.clips.push(clip);
                // Insere um fade zerado (sem fade in/out) para clip importado
                self.fades.push(fades::ClipFade::none());
                // Atualiza buffers de waveform
                self.rebuild_wave_data();
                self.request_redraw();
            },
            Err(e) => {
                eprintln!("falha ao decodificar {}: {}", path, e);
            }
        }
    }
}
