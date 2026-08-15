//! Padrões de bateria / step sequencer (MIDI).
//!
//! Estado autoritativo do beat sequencer: vive no core, UI projeta.

/// Parâmetros de uma trilha do beat sequencer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrumTrackParams {
    pub active: bool,
    pub solo: bool,
    pub pan: f32,
    pub volume: f32,
    pub mixer_channel: i32,
}

impl Default for DrumTrackParams {
    fn default() -> Self {
        Self {
            active: true,
            solo: false,
            pan: 0.0,
            volume: 1.0,
            mixer_channel: 0,
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
    /// Cria um padrão vazio com N trilhas e resolução.
    pub fn new(name: impl Into<String>, num_tracks: usize, resolution: u32) -> Self {
        let grid = vec![vec![false; resolution as usize]; num_tracks];
        let track_params = (0..num_tracks).map(|_| DrumTrackParams::default()).collect();
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

    /// Alterna um passo (track, step).
    pub fn toggle_step(&mut self, track: usize, step: usize) -> bool {
        if track < self.grid.len() && step < self.grid[track].len() {
            self.grid[track][step] = !self.grid[track][step];
            self.grid[track][step]
        } else {
            false
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