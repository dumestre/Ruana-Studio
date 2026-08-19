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
}