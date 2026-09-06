//! Timeline: matemática de tempo musical e grids de edição.
//!
//! Independente de UI/Slint. Produz as grades e réguas usadas
//! pelos editores (arrange / piano roll) e, no futuro, pelo
//! tempo musical real (TempoMap, PPQ).

/// Tick da régua de compassos (nível 0 = barra, maior = subdivisão).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RulerTick {
    pub bar: i32,
    pub pos: f32,
    pub level: i32,
}

/// Uma mudança de tempo no mapa de tempo (time-warp).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoChange {
    /// Posição em beats onde esta mudança de BPM entra em vigor.
    pub beat: f32,
    /// BPM a partir deste beat.
    pub bpm: f32,
}

/// Mapa de tempo: lista ordenada de mudanças de BPM ao longo da timeline.
///
/// Permite variações de tempo (time-warp / stretch) — o mesmo
/// intervalo de beats dura mais ou menos segundos dependendo do
/// BPM ativo em cada segmento.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TempoMap {
    /// Mudanças de tempo, ordenadas por beat crescente.
    /// Sempre começa com uma entrada no beat 0.
    pub changes: Vec<TempoChange>,
}

impl TempoMap {
    /// Cria um mapa de tempo constante com o BPM dado.
    pub fn from_bpm(bpm: f32) -> Self {
        Self {
            changes: vec![TempoChange { beat: 0.0, bpm }],
        }
    }

    /// BPM no beat especificado (última mudança até ou antes do beat).
    pub fn bpm_at(&self, beat: f32) -> f32 {
        self.changes
            .iter()
            .rev()
            .find(|c| c.beat <= beat)
            .map(|c| c.bpm)
            .unwrap_or(120.0)
    }

    /// Converte beats → segundos, respeitando variações de tempo.
    pub fn beats_to_seconds(&self, beat: f32) -> f64 {
        if self.changes.is_empty() {
            return beat as f64 * 60.0 / 120.0;
        }
        let mut seconds = 0.0;
        let mut current_beat = 0.0;
        let mut current_bpm = self.changes[0].bpm;
        for change in &self.changes {
            if change.beat >= beat {
                break;
            }
            let seg = (change.beat - current_beat).max(0.0);
            seconds += seg as f64 * 60.0 / current_bpm as f64;
            current_beat = change.beat;
            current_bpm = change.bpm;
        }
        let remaining = (beat - current_beat).max(0.0);
        seconds + remaining as f64 * 60.0 / current_bpm as f64
    }

    /// Converte segundos → beats, respeitando variações de tempo.
    pub fn seconds_to_beats(&self, seconds: f64) -> f32 {
        if self.changes.is_empty() || seconds <= 0.0 {
            let bpm = self.changes.first().map(|c| c.bpm).unwrap_or(120.0);
            return (seconds * bpm as f64 / 60.0) as f32;
        }
        let mut elapsed = 0.0_f64;
        let mut current_beat = 0.0_f32;
        let mut current_bpm = self.changes[0].bpm;
        for change in &self.changes {
            let seg_secs =
                (change.beat - current_beat).max(0.0) as f64 * 60.0 / current_bpm as f64;
            if elapsed + seg_secs >= seconds {
                let remaining = (seconds - elapsed).max(0.0);
                return current_beat + remaining as f32 * current_bpm / 60.0;
            }
            elapsed += seg_secs;
            current_beat = change.beat;
            current_bpm = change.bpm;
        }
        let remaining = (seconds - elapsed).max(0.0);
        current_beat + remaining as f32 * current_bpm / 60.0
    }

    /// Adiciona ou atualiza uma mudança de tempo no beat especificado.
    pub fn set_change(&mut self, beat: f32, bpm: f32) {
        let bpm = bpm.max(1.0).min(999.0);
        if let Some(pos) = self.changes.iter().position(|c| (c.beat - beat).abs() < 0.001) {
            self.changes[pos].bpm = bpm;
        } else {
            self.changes.push(TempoChange { beat, bpm });
            self.changes.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
        }
    }

    /// Remove uma mudança de tempo (exceto a do beat 0).
    pub fn remove_change(&mut self, beat: f32) -> bool {
        if beat.abs() < 0.001 {
            return false;
        }
        if let Some(pos) = self.changes.iter().position(|c| (c.beat - beat).abs() < 0.001) {
            self.changes.remove(pos);
            true
        } else {
            false
        }
    }
}


/// Grade musical derivada de um range de notas e número de compassos.
///
/// Todos os campos são `Vec` planos prontos para a UI projetar
/// (nenhum array hardcoded na camada visual).
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub first_note: i32,
    pub last_note: i32,
    pub bars: i32,
    /// Linhas de notas: 0..=(last-first).
    pub note_lines: Vec<i32>,
    /// Linhas de beats: 0..(bars*16).
    pub beat_lines: Vec<i32>,
    /// Oitavas no range, do agudo para o grave.
    pub octave_range: Vec<i32>,
    /// Compassos: 0..bars.
    pub bar_range: Vec<i32>,
    /// Ticks da régua (16 por compasso, com níveis de subdivisão).
    pub ruler_ticks: Vec<RulerTick>,
}

impl Grid {
    /// Cria a grade para o range de notas e nº de compassos dados.
    pub fn new(first_note: i32, last_note: i32, bars: i32) -> Self {
        let note_lines: Vec<i32> = (0..=(last_note - first_note)).collect();
        let beat_lines: Vec<i32> = (0..(bars * 16)).collect();
        let octave_range: Vec<i32> = (first_note / 12..=last_note / 12).rev().collect();
        let bar_range: Vec<i32> = (0..bars).collect();

        let mut ruler_ticks = Vec::new();
        for bar in 0..bars {
            ruler_ticks.push(RulerTick { bar, pos: 0.0, level: 0 });
            for (pos, level) in [
                (0.25, 1),
                (0.50, 1),
                (0.75, 1),
                (0.125, 2),
                (0.375, 2),
                (0.625, 2),
                (0.875, 2),
                (0.0625, 3),
                (0.1875, 3),
                (0.3125, 3),
                (0.4375, 3),
                (0.5625, 3),
                (0.6875, 3),
                (0.8125, 3),
                (0.9375, 3),
            ] {
                ruler_ticks.push(RulerTick { bar, pos, level });
            }
        }

        Self {
            first_note,
            last_note,
            bars,
            note_lines,
            beat_lines,
            octave_range,
            bar_range,
            ruler_ticks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_ranges() {
        let g = Grid::new(0, 127, 64);
        assert_eq!(g.note_lines.len(), 128);
        assert_eq!(g.beat_lines.len(), 64 * 16);
        assert_eq!(g.bar_range.len(), 64);
        assert_eq!(g.octave_range.len(), 11); // 0..10 inclusive
        assert_eq!(*g.octave_range.first().unwrap(), 10);
        assert_eq!(*g.octave_range.last().unwrap(), 0);
    }

    #[test]
    fn grid_ticks_per_bar() {
        let g = Grid::new(0, 24, 4);
        assert_eq!(g.ruler_ticks.len(), 4 * 16);
        // Primeiro tick de cada compasso é a barra (nível 0).
        for bar in 0..4 {
            let bar_tick = &g.ruler_ticks[bar * 16];
            assert_eq!(bar_tick.bar, bar as i32);
            assert_eq!(bar_tick.pos, 0.0);
            assert_eq!(bar_tick.level, 0);
        }
    }

    #[test]
    fn grid_partial_octave_range() {
        let g = Grid::new(60, 72, 1);
        assert_eq!(g.octave_range, vec![6, 5]); // C4..C5 → oitavas 6 e 5
        assert_eq!(g.note_lines.len(), 13);
    }

    #[test]
    fn tempo_map_constant_bpm() {
        let tm = TempoMap::from_bpm(120.0);
        // 120 BPM: 1 beat = 0.5s
        assert!((tm.beats_to_seconds(2.0) - 1.0).abs() < 1e-9);
        assert!((tm.seconds_to_beats(1.0) - 2.0).abs() < 1e-6);
        assert_eq!(tm.bpm_at(100.0), 120.0);
    }

    #[test]
    fn tempo_map_bpm_change() {
        let mut tm = TempoMap::from_bpm(120.0);
        tm.set_change(8.0, 60.0); // a partir do beat 8, BPM cai para 60
        // beats 0..8 a 120 BPM → 4s; beats 8..12 a 60 BPM → 4s
        assert!((tm.beats_to_seconds(12.0) - 8.0).abs() < 1e-9);
        // Inverso
        assert!((tm.seconds_to_beats(8.0) - 12.0).abs() < 1e-5);
        // BPM antes do beat 8
        assert_eq!(tm.bpm_at(5.0), 120.0);
        // BPM a partir do beat 8
        assert_eq!(tm.bpm_at(10.0), 60.0);
    }

    #[test]
    fn tempo_map_set_and_remove() {
        let mut tm = TempoMap::from_bpm(120.0);
        tm.set_change(4.0, 140.0);
        assert_eq!(tm.changes.len(), 2);
        assert_eq!(tm.bpm_at(4.0), 140.0);
        assert!(tm.remove_change(4.0));
        assert_eq!(tm.changes.len(), 1);
        // Não remove o beat 0
        assert!(!tm.remove_change(0.0));
    }
}