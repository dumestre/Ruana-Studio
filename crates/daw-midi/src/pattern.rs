//! Padrões de bateria / step sequencer (MIDI).
//!
//! Estado autoritativo do beat sequencer: vive no core, UI projeta.

use crate::notes::Note;

/// Pitch base dos clips MIDI gerados por um padrão de bateria.
///
/// Cada trilha do padrão ocupa um pitch consecutivo a partir daqui; o
/// motor de áudio mapeia o pitch de volta para a linha do padrão (36 +
/// índice da trilha) para obter o sample daquela linha.
pub const DRUM_PITCH_BASE: i32 = 36;

/// Parâmetros de uma trilha do beat sequencer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrumTrackParams {
    /// Nome da trilha (ex.: "Kick", "Snare").
    pub name: String,
    pub active: bool,
    pub solo: bool,
    pub pan: f32,
    pub volume: f32,
    pub mixer_channel: i32,
    /// Sample atribuído à trilha via drag & drop (path do arquivo).
    pub sample_path: Option<String>,
}

impl Default for DrumTrackParams {
    fn default() -> Self {
        Self {
            name: String::new(),
            active: true,
            solo: false,
            pan: 0.0,
            volume: 1.0,
            mixer_channel: 0,
            sample_path: None,
        }
    }
}

/// Um padrão de bateria (matriz de passos: tracks x steps).
#[derive(Debug, Clone, PartialEq)]
pub struct DrumPattern {
    /// Nome do padrão.
    pub name: String,
    /// Grade: tracks x steps (bool = note on/off).
    pub grid: Vec<Vec<bool>>,
    /// Resolução (passos por compasso).
    pub resolution: u32,
    /// Parâmetros por trilha.
    pub track_params: Vec<DrumTrackParams>,
    /// Swing 0..1.
    pub swing: f32,
    /// Humanize 0..1.
    pub humanize: f32,
}

impl Default for DrumPattern {
    fn default() -> Self {
        Self::new("DEFAULT", 8, 16)
    }
}

impl DrumPattern {
    /// Nomes padrão de trilha (kit de bateria).
    const DEFAULT_TRACK_NAMES: [&'static str; 8] = [
        "Kick",
        "Snare",
        "Clap",
        "Closed Hat",
        "Open Hat",
        "Tom",
        "Cymbal",
        "Perc",
    ];

    /// Cria um padrão vazio com N trilhas e resolução.
    pub fn new(name: impl Into<String>, num_tracks: usize, resolution: u32) -> Self {
        let grid = vec![vec![false; resolution as usize]; num_tracks];
        let track_params = (0..num_tracks)
            .map(|i| DrumTrackParams {
                name: Self::DEFAULT_TRACK_NAMES
                    .get(i)
                    .map(|s| (*s).to_string())
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                ..Default::default()
            })
            .collect();
        Self {
            name: name.into(),
            grid,
            resolution,
            track_params,
            swing: 0.0,
            humanize: 0.0,
        }
    }

    /// Número de trilhas.
    pub fn num_tracks(&self) -> usize {
        self.grid.len()
    }

    /// Número de passos (resolução).
    pub fn steps(&self) -> usize {
        self.resolution as usize
    }

    /// Duração do padrão em beats da timeline.
    ///
    /// `resolution` é passos por COMPASSO (16 = 1 compasso de 4/4,
    /// como no channel rack do FL Studio). Um padrão de N passos dura
    /// N/4 beats — é isto que dá a "duração correspondente" quando o
    /// padrão é arrastado do Clip Pool para a timeline.
    pub fn length_beats(&self) -> f32 {
        (self.resolution as f32) / 4.0
    }

    /// Alterna um passo (track, step).
    pub fn toggle_step(&mut self, track: usize, step: usize) -> bool {
        if track < self.grid.len() && step < self.grid[track].len() {
            self.grid[track][step] = !self.grid[track][step];
            self.grid[track][step]
        } else {
            false
        }
    }

    /// Define o estado de um passo (track, step). Retorna `true` se existia.
    pub fn set_step(&mut self, track: usize, step: usize, on: bool) -> bool {
        let Some(row) = self.grid.get_mut(track) else {
            return false;
        };
        let Some(cell) = row.get_mut(step) else {
            return false;
        };
        *cell = on;
        true
    }

    /// Redefine a resolução, preservando os passos já gravados.
    pub fn set_resolution(&mut self, resolution: u32) {
        let resolution = resolution as usize;
        self.resolution = resolution as u32;
        for row in &mut self.grid {
            row.resize(resolution, false);
        }
    }

    /// Insere `count` passos vazios no início de todas as trilhas,
    /// deslocando os passos gravados para a direita (os últimos caem fora).
    pub fn insert_steps(&mut self, count: usize) {
        let steps = self.steps();
        let shift = count.min(steps);
        for row in &mut self.grid {
            row.rotate_right(shift);
            for cell in row.iter_mut().take(shift) {
                *cell = false;
            }
        }
    }

    /// Limpa todos os passos de uma trilha.
    pub fn clear_track(&mut self, track: usize) {
        if track < self.grid.len() {
            self.grid[track].fill(false);
        }
    }

    /// Copia uma trilha para outra.
    pub fn copy_track(&mut self, src: usize, dst: usize) {
        if src < self.grid.len() && dst < self.grid.len() {
            self.grid[dst] = self.grid[src].clone();
        }
    }

    /// Duplica uma trilha (copia para próxima vaga ou empilha).
    pub fn duplicate_track(&mut self, track: usize) {
        if track < self.grid.len() {
            let new_grid = self.grid[track].clone();
            let new_params = self.track_params[track].clone();
            self.grid.push(new_grid);
            self.track_params.push(new_params);
        }
    }

    /// Remove uma trilha.
    pub fn delete_track(&mut self, track: usize) -> bool {
        if track < self.grid.len() && self.grid.len() > 1 {
            self.grid.remove(track);
            self.track_params.remove(track);
            true
        } else {
            false
        }
    }

    /// Limpa todo o padrão.
    pub fn clear(&mut self) {
        for row in &mut self.grid {
            row.fill(false);
        }
    }

    /// Converte a grade em notas MIDI (para arrastar o padrão pra timeline).
    ///
    /// Cada trilha vira um pitch fixo (`DRUM_PITCH_BASE + track`): o
    /// motor de áudio usa esse pitch pra mapear a nota de volta pra
    /// linha do pattern e tocar o sample da linha. O swing atrasa os
    /// passos ímpares em até meio passo, igual ao scheduler RT do beat.
    pub fn to_notes(&self) -> Vec<Note> {
        let steps = self.steps();
        let step_beats = if steps > 0 {
            self.length_beats() / steps as f32
        } else {
            0.25
        };
        let mut notes = Vec::new();
        let mut id: i32 = 1;
        for (track, row) in self.grid.iter().enumerate() {
            let pitch = DRUM_PITCH_BASE + track as i32;
            for (step, on) in row.iter().enumerate() {
                if !*on {
                    continue;
                }
                let swing_delay = if step.rem_euclid(2) == 1 {
                    self.swing.clamp(0.0, 1.0) * 0.5 * step_beats
                } else {
                    0.0
                };
                notes.push(Note {
                    id,
                    pitch,
                    start: step as f32 * step_beats + swing_delay,
                    length: step_beats,
                    velocity: 100,
                });
                id += 1;
            }
        }
        notes
    }
}

/// Container de múltiplos padrões (song mode).
#[derive(Debug, Clone, Default)]
pub struct BeatPatterns {
    pub patterns: Vec<DrumPattern>,
    pub current: usize,
}

impl BeatPatterns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_pattern(&self) -> Option<&DrumPattern> {
        self.patterns.get(self.current)
    }

    pub fn current_pattern_mut(&mut self) -> Option<&mut DrumPattern> {
        self.patterns.get_mut(self.current)
    }

    pub fn set_current(&mut self, idx: usize) -> bool {
        if idx < self.patterns.len() {
            self.current = idx;
            true
        } else {
            false
        }
    }

    pub fn add_pattern(&mut self, pattern: DrumPattern) {
        self.patterns.push(pattern);
    }

    pub fn duplicate_current(&mut self) {
        let Some(p) = self.current_pattern().cloned() else { return };
        let mut new_p = p;
        new_p.name = format!("{} (Copy)", new_p.name);
        self.patterns.push(new_p);
        self.current = self.patterns.len() - 1;
    }

    pub fn delete_current(&mut self) -> bool {
        if self.patterns.len() > 1 {
            self.patterns.remove(self.current);
            if self.current >= self.patterns.len() {
                self.current = self.patterns.len() - 1;
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_toggle_step() {
        let mut p = DrumPattern::new("Test", 2, 16);
        assert!(p.toggle_step(0, 0)); // false -> true, returns new value
        assert!(p.grid[0][0]);
        assert!(!p.toggle_step(0, 0)); // true -> false
        assert!(!p.grid[0][0]);
    }

    #[test]
    fn pattern_bounds() {
        let mut p = DrumPattern::new("Test", 2, 16);
        assert!(!p.toggle_step(5, 0));
        assert!(!p.toggle_step(0, 20));
    }

    #[test]
    fn pattern_length_beats() {
        // 16 passos por compasso = 1 compasso = 4 beats.
        assert_eq!(DrumPattern::new("A", 2, 16).length_beats(), 4.0);
        // Meio compasso (8) → 2 beats; duplo (32) → 8 beats.
        assert_eq!(DrumPattern::new("B", 2, 8).length_beats(), 2.0);
        assert_eq!(DrumPattern::new("C", 2, 32).length_beats(), 8.0);
    }

    #[test]
    fn to_notes_maps_tracks_to_pitches() {
        let mut p = DrumPattern::new("Test", 3, 4);
        p.set_step(0, 0, true);
        p.set_step(2, 3, true);
        p.track_params[1].volume = 0.5;
        let notes = p.to_notes();
        // Um passo por trilha → 2 notas com pitch base + índice.
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, DRUM_PITCH_BASE);
        assert_eq!(notes[0].start, 0.0);
        assert_eq!(notes[0].length, 0.25);
        assert_eq!(notes[1].pitch, DRUM_PITCH_BASE + 2);
        // 4 passos = 1 beat; passo 3 começa em 0.75 beats.
        assert_eq!(notes[1].start, 0.75);
        assert_eq!(notes[1].velocity, 100);
    }

    #[test]
    fn to_notes_applies_swing_to_odd_steps() {
        let mut p = DrumPattern::new("Swing", 1, 8);
        p.swing = 0.5;
        p.set_step(0, 1, true);
        p.set_step(0, 2, true);
        let notes = p.to_notes();
        assert_eq!(notes.len(), 2);
        // Passo 1 (ímpar): atraso de swing*0.5*step = 0.5*0.5*0.25.
        assert!((notes[0].start - 0.3125).abs() < 1e-6);
        // Passo 2 (par): sem atraso.
        assert!((notes[1].start - 0.5).abs() < 1e-6);
    }

    #[test]
    fn track_params_default() {
        let p = DrumTrackParams::default();
        assert!(p.active);
        assert!(!p.solo);
        assert_eq!(p.pan, 0.0);
        assert_eq!(p.volume, 1.0);
        assert_eq!(p.mixer_channel, 0);
    }

    #[test]
    fn beat_patterns_duplicate() {
        let mut b = BeatPatterns::new();
        b.add_pattern(DrumPattern::new("A", 2, 16));
        assert_eq!(b.patterns.len(), 1);
        b.duplicate_current();
        assert_eq!(b.patterns.len(), 2);
        assert_eq!(b.current, 1);
    }
}