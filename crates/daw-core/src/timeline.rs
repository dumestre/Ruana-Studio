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

    /// Faixa de BPM que o timewarp aceita (a mesma do campo de BPM da UI).
    ///
    /// Fora dela a rampa exigida seria absurda (ou infinita) — melhor recusar
    /// o gesto do que gravar um tempo impraticável no documento.
    pub const BPM_MIN: f32 = 20.0;
    pub const BPM_MAX: f32 = 300.0;

    /// TIME WARP (Cubase): faz a posição musical `musical_beat` soar no
    /// instante em que HOJE soa a posição `target_beat`.
    ///
    /// É o gesto da ferramenta Time Warp: você arrasta a linha do compasso 5
    /// até um evento do áudio e o Cubase descobre o BPM que faz o compasso 5
    /// cair exatamente ali. O que muda é o TEMPO, não o áudio: os clips em
    /// modo musical (ver `TimeMode`) acompanham, porque são reesticados para
    /// o novo span.
    ///
    /// Como funciona:
    ///
    /// - `t_target` = `beats_to_seconds(target_beat)` no mapa ATUAL — o
    ///   instante em que o alvo soa agora;
    /// - o trecho afetado é `[prev, musical_beat]`, onde `prev` é a última
    ///   mudança de tempo ANTES do ponto arrastado (`prev` continua soando no
    ///   mesmo instante: nada antes dele é tocado);
    /// - o BPM desse trecho passa a ser o que faz `musical_beat` cair em
    ///   `t_target`;
    /// - uma mudança no próprio `musical_beat` preserva o BPM que existia
    ///   depois dele — o tempo da música da frente não muda.
    ///
    /// Recusa (devolve `false`) quando:
    ///
    /// - o alvo não é posterior a `prev` (não existe BPM finito para isso);
    /// - o BPM necessário sai de [`Self::BPM_MIN`]..=[`Self::BPM_MAX`].
    ///
    /// Nenhuma outra mudança de tempo corre risco: por construção `prev` é a
    /// ÚLTIMA antes do ponto arrastado, então tudo o que o usuário já tinha
    /// ficou ANTES do trecho reescalado e não se move.
    pub fn warp_beat_to_target(&mut self, musical_beat: f32, target_beat: f32) -> bool {
        if !musical_beat.is_finite() || !target_beat.is_finite() {
            return false;
        }

        if musical_beat <= 0.001 {
            return false;
        }

        // A mudança que governa o trecho que termina em `musical_beat`.
        let prev_index = self
            .changes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.beat < musical_beat - 0.001)
            .map(|(i, _)| i)
            .next_back();

        let (prev_beat, prev_bpm_after) = match prev_index {
            Some(i) => (self.changes[i].beat, self.bpm_at(musical_beat)),
            // Sem nenhuma mudança antes do ponto: a rampa começa no zero.
            None => (0.0, self.bpm_at(musical_beat)),
        };

        let t_prev = self.beats_to_seconds(prev_beat);
        let t_target = self.beats_to_seconds(target_beat);
        let seconds = t_target - t_prev;

        if !seconds.is_finite() || seconds <= 1e-6 {
            return false;
        }

        let beats = (musical_beat - prev_beat) as f64;

        if beats <= 0.0 {
            return false;
        }

        let bpm = (beats * 60.0 / seconds) as f32;

        if !bpm.is_finite() || !(Self::BPM_MIN..=Self::BPM_MAX).contains(&bpm) {
            return false;
        }

        // O BPM que valia DEPOIS do ponto arrastado é preservado: só o trecho
        // que termina nele é reescalado.
        self.set_change(prev_beat, bpm);
        self.set_change(musical_beat, prev_bpm_after);

        true
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
#[cfg(test)]
mod timewarp_tool_tests {
    use super::*;

    const EPS: f64 = 1e-3;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn esticar_compasso_para_um_evento_mais_a_frente() {
        // 120 BPM constante. Arrastar o beat 4 ate onde HOJE soa o beat 6
        // (3.0 s): o trecho 0..4 passa a durar 3.0 s -> 80 BPM.
        let mut map = TempoMap::from_bpm(120.0);

        assert!(close(map.beats_to_seconds(6.0), 3.0));

        assert!(map.warp_beat_to_target(4.0, 6.0));

        // INVARIANTE: o beat arrastado soa exatamente onde o alvo soava.
        assert!(
            close(map.beats_to_seconds(4.0), 3.0),
            "beat 4 em {} s",
            map.beats_to_seconds(4.0)
        );

        assert!(close(map.bpm_at(0.0) as f64, 80.0), "bpm {}", map.bpm_at(0.0));

        // O tempo DEPOIS do ponto arrastado nao muda: 2 beats a 120 BPM.
        assert!(
            close(map.beats_to_seconds(6.0) - map.beats_to_seconds(4.0), 1.0),
            "depois do warp: {} s",
            map.beats_to_seconds(6.0) - map.beats_to_seconds(4.0)
        );
    }

    #[test]
    fn comprimir_arrastando_para_tras() {
        // Arrastar o beat 8 para onde hoje soa o beat 4 (2.0 s): o trecho
        // 0..8 passa a durar 2.0 s -> 240 BPM.
        let mut map = TempoMap::from_bpm(120.0);

        assert!(map.warp_beat_to_target(8.0, 4.0));

        assert!(close(map.beats_to_seconds(8.0), 2.0), "{}", map.beats_to_seconds(8.0));
        assert!(close(map.bpm_at(0.0) as f64, 240.0), "bpm {}", map.bpm_at(0.0));
    }

    #[test]
    fn rampa_usa_o_ponto_de_tempo_anterior() {
        // Tempo: 120 BPM ate o beat 8, 60 BPM depois. Arrastar o beat 12
        // (que hoje soa em 4.0 + 4*1.0 = ... ver abaixo) para o beat 10.
        let mut map = TempoMap::from_bpm(120.0);
        map.set_change(8.0, 60.0);

        // t(8) = 4.0 s (8 beats a 120) ; t(10) = 4.0 + 2 beats a 60 = 6.0 s.
        assert!(close(map.beats_to_seconds(8.0), 4.0));
        assert!(close(map.beats_to_seconds(10.0), 6.0));

        // Arrastar o beat 12 ate onde o beat 10 soa hoje (6.0 s).
        assert!(map.warp_beat_to_target(12.0, 10.0));

        assert!(close(map.beats_to_seconds(12.0), 6.0), "{}", map.beats_to_seconds(12.0));

        // O trecho afetado e [8, 12]: 4 beats em 2.0 s -> 120 BPM de novo.
        assert!(close(map.bpm_at(8.0) as f64, 120.0), "bpm {}", map.bpm_at(8.0));

        // O que vinha ANTES do ponto anterior nao se move.
        assert!(close(map.beats_to_seconds(8.0), 4.0));

        // E o tempo DEPOIS do ponto arrastado e o que ja valia ali (60 BPM):
        // 4 beats = 4.0 s. O ajuste nao vaza para a musica da frente.
        assert!(
            close(map.beats_to_seconds(16.0) - map.beats_to_seconds(12.0), 4.0),
            "depois: {}",
            map.beats_to_seconds(16.0) - map.beats_to_seconds(12.0)
        );
        assert!(close(map.bpm_at(12.0) as f64, 60.0), "bpm em 12: {}", map.bpm_at(12.0));
    }

    #[test]
    fn recusa_bpm_fora_da_faixa() {
        let mut map = TempoMap::from_bpm(120.0);

        // Trazer o beat 40 para 1 s exigiria 2400 BPM.
        assert!(!map.warp_beat_to_target(40.0, 2.0));
        // Estado intacto.
        assert!(close(map.beats_to_seconds(40.0), 20.0));

        // Empurrar o beat 4 para 100 s exigiria ~1.9 BPM.
        assert!(!map.warp_beat_to_target(4.0, 400.0));
        assert!(close(map.beats_to_seconds(4.0), 2.0));
    }

    #[test]
    fn recusa_alvo_antes_do_ponto_anterior() {
        let mut map = TempoMap::from_bpm(120.0);
        map.set_change(8.0, 120.0);

        // Alvo (beat 2 = 1.0 s) e ANTERIOR ao ponto anterior (beat 8 = 4.0 s):
        // nao existe BPM que faca o beat 12 soar antes do beat 8.
        assert!(!map.warp_beat_to_target(12.0, 2.0));
    }

    #[test]
    fn pontos_de_tempo_anteriores_a_prev_sobrevivem() {
        // Tres pontos: 120 em 0, 90 em 4 e 100 em 8. Arrastando o beat 12,
        // `prev` e o ponto de beat 8 - SO ele vira a rampa. O ponto de beat 4
        // (anterior a prev) e o trecho ate ele tem de sair intactos.
        let mut map = TempoMap::from_bpm(120.0);
        map.set_change(4.0, 90.0);
        map.set_change(8.0, 100.0);

        let antes_4 = map.beats_to_seconds(4.0);
        let antes_10 = map.beats_to_seconds(10.0);

        assert!(close(antes_4, 2.0), "t(4) {}", antes_4);

        // Arrastar o beat 12 ate onde o beat 10 soa hoje.
        assert!(map.warp_beat_to_target(12.0, 10.0), "gesto devia ser aceito");

        // INVARIANTE do gesto.
        assert!(
            close(map.beats_to_seconds(12.0), antes_10),
            "beat 12 em {} s, alvo era {} s",
            map.beats_to_seconds(12.0),
            antes_10
        );

        // O que estava ANTES de prev nao se moveu.
        assert!(close(map.bpm_at(4.0) as f64, 90.0), "bpm em 4: {}", map.bpm_at(4.0));
        assert!(close(map.beats_to_seconds(4.0), antes_4), "o trecho antes de prev se moveu");

        // O trecho reescalado e [8, 12]: 4 beats em 1.2 s -> 200 BPM.
        assert!(close(map.bpm_at(8.0) as f64, 200.0), "bpm em 8: {}", map.bpm_at(8.0));

        // E o tempo depois do ponto arrastado continua o de antes (100 BPM).
        assert!(close(map.bpm_at(12.0) as f64, 100.0), "bpm em 12: {}", map.bpm_at(12.0));
    }

    #[test]
    fn recusa_entradas_degeneradas() {
        let mut map = TempoMap::from_bpm(120.0);

        assert!(!map.warp_beat_to_target(0.0, 4.0));
        assert!(!map.warp_beat_to_target(-2.0, 4.0));
        assert!(!map.warp_beat_to_target(f32::NAN, 4.0));
        assert!(!map.warp_beat_to_target(4.0, f32::NAN));
        // Alvo no mesmo instante do ponto anterior.
        assert!(!map.warp_beat_to_target(4.0, 0.0));
    }

    #[test]
    fn warps_sucessivos_continuam_batendo() {
        // Dois gestos encadeados: cada um tem de manter a invariante.
        let mut map = TempoMap::from_bpm(120.0);

        assert!(map.warp_beat_to_target(4.0, 6.0));
        let t4 = map.beats_to_seconds(4.0);
        assert!(close(t4, 3.0));

        // Agora arrasta o beat 8 ate onde o beat 6 soa hoje.
        let t6 = map.beats_to_seconds(6.0);
        assert!(map.warp_beat_to_target(8.0, 6.0));

        assert!(close(map.beats_to_seconds(8.0), t6), "{}", map.beats_to_seconds(8.0));
        // E o beat 4 nao se moveu no segundo gesto.
        assert!(close(map.beats_to_seconds(4.0), t4));
    }
}