//! Transporte: estado de reprodução do documento.
//!
//! O playhead vive aqui (no core), não na UI. O engine de áudio
//! futuro (daw-audio) consome e avança este estado; a UI apenas
//! projeta a posição.

/// Estado de transporte da DAW.
#[derive(Debug, Clone)]
pub struct Transport {
    pub playing: bool,
    /// Modo "só beat": o transporte roda e o step sequencer soa e
    /// avança, mas os clips da timeline NÃO tocam nem andam.
    pub beat_only: bool,
    /// Posição do playhead em ticks (PPQ).
    pub position_ticks: u64,
    /// Onde o último play começou. SPACE (stop) devolve a agulha
    /// pra cá — comportamento padrão de DAW.
    pub play_start_ticks: u64,
    /// Batidas por minuto.
    pub bpm: u32,
    /// Pulsos por batida (resolução).
    pub ppq: u32,
    /// Batidas por compasso.
    pub beats_per_bar: u32,
    /// Metrônomo do transporte.
    pub metronome_enabled: bool,
    /// Timbre do metrônomo (0..4).
    pub metronome_sound: u8,
    /// Volume do metrônomo (0..1).
    pub metronome_volume: f32,
    pub loop_enabled: bool,
    pub loop_start_ticks: u64,
    pub loop_end_ticks: u64,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            playing: false,
            beat_only: false,
            position_ticks: 0,
            play_start_ticks: 0,
            bpm: 120,
            ppq: 960,
            beats_per_bar: 4,
            metronome_enabled: false,
            metronome_sound: 0,
            metronome_volume: 0.7,
            loop_enabled: false,
            loop_start_ticks: 0,
            loop_end_ticks: 15360,
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

    /// Posição em segundos (tempo contínuo).
    ///
    /// Fonte da Playhead: a UI converte segundos → pixels via
    /// `pixels_per_second = pixels_per_beat / seconds_per_beat`,
    /// sem quantização para beat/grid.
    pub fn position_seconds(&self) -> f64 {
        self.position_ticks as f64 / self.ppq as f64 * 60.0 / self.bpm as f64
    }

    /// Posiciona o playhead em segundos (tempo contínuo).
    pub fn seek_seconds(&mut self, secs: f64) {
        let beats = (secs.max(0.0) * self.bpm as f64 / 60.0) * self.ppq as f64;
        self.position_ticks = beats as u64;
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

    #[test]
    fn seconds_conversion_is_continuous() {
        // 120 BPM, 960 PPQ → 1 beat = 0.5s.
        let mut t = Transport::default();
        assert_eq!(t.position_seconds(), 0.0);

        t.seek(960);
        assert!((t.position_seconds() - 0.5).abs() < 1e-9);

        // Meio segundo após o seek → 1 beat exato.
        t.seek_seconds(1.0);
        assert_eq!(t.position_ticks, 1920);
        assert!((t.position_seconds() - 1.0).abs() < 1e-9);

        // BPM diferente: mesma posição musical, outro tempo em s.
        t.bpm = 60;
        assert!((t.position_seconds() - 2.0).abs() < 1e-9);

        // Negativo é tratado como zero.
        t.seek_seconds(-3.0);
        assert_eq!(t.position_ticks, 0);
    }
}
