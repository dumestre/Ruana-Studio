//! Transporte: estado de reprodução do documento.
//!
//! O playhead vive aqui (no core), não na UI. O engine de áudio
//! futuro (daw-audio) consome e avança este estado; a UI apenas
//! projeta a posição.

/// Estado de transporte da DAW.
#[derive(Debug, Clone)]
pub struct Transport {
    pub playing: bool,
    /// Posição do playhead em ticks (PPQ).
    pub position_ticks: u64,
    /// Batidas por minuto.
    pub bpm: u32,
    /// Pulsos por batida (resolução).
    pub ppq: u32,
    /// Batidas por compasso.
    pub beats_per_bar: u32,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            playing: false,
            position_ticks: 0,
            bpm: 120,
            ppq: 960,
            beats_per_bar: 4,
        }
    }
}

impl Transport {
    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    /// Posiciona o playhead em ticks.
    pub fn seek(&mut self, ticks: u64) {
        self.position_ticks = ticks;
    }

    /// Avança o playhead `delta` ticks (só quando tocando).
    pub fn advance(&mut self, delta_ticks: u64) {
        if self.playing {
            self.position_ticks += delta_ticks;
        }
    }

    /// Posição em batidas (fração).
    pub fn position_beats(&self) -> f32 {
        self.position_ticks as f32 / self.ppq as f32
    }

    /// Ticks por compasso.
    pub fn ticks_per_bar(&self) -> u64 {
        self.ppq as u64 * self.beats_per_bar as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_daw() {
        let t = Transport::default();
        assert!(!t.playing);
        assert_eq!(t.position_ticks, 0);
        assert_eq!(t.bpm, 120);
        assert_eq!(t.ppq, 960);
        assert_eq!(t.beats_per_bar, 4);
    }

    #[test]
    fn play_pause_toggle() {
        let mut t = Transport::default();
        t.play();
        assert!(t.playing);
        t.pause();
        assert!(!t.playing);
        t.toggle();
        assert!(t.playing);
        t.toggle();
        assert!(!t.playing);
    }

    #[test]
    fn advance_only_while_playing() {
        let mut t = Transport::default();
        t.advance(480);
        assert_eq!(t.position_ticks, 0);

        t.play();
        t.advance(480);
        t.advance(480);
        assert_eq!(t.position_ticks, 960);
    }

    #[test]
    fn seek_and_position_beats() {
        let mut t = Transport::default();
        t.seek(960);
        assert_eq!(t.position_ticks, 960);
        assert_eq!(t.position_beats(), 1.0);
        assert_eq!(t.ticks_per_bar(), 3840);
    }
}
