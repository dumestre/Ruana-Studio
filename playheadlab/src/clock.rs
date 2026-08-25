//! Clock simulado do transporte + seek throttled.
//!
//! Réplica do que a DAW tem (VisualClock + transport), mas sem
//! áudio: o "motor" é só um f64. Serve pra medir e provar que
//! o scrub é fluido antes de portar pro app real.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Clock visual: âncora `(segundos, Instant)` projetada no tempo.
pub struct VisualClock {
    anchor_secs: f64,
    anchor_at: Instant,
    playing: bool,
}

impl VisualClock {
    pub fn new() -> Self {
        Self {
            anchor_secs: 0.0,
            anchor_at: Instant::now(),
            playing: false,
        }
    }

    pub fn set_playing(&mut self, on: bool) {
        if on == self.playing {
            return;
        }
        if on {
            self.anchor_at = Instant::now();
        } else {
            self.anchor_secs = self.projected();
        }
        self.playing = on;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Posição projetada agora.
    pub fn projected(&self) -> f64 {
        if self.playing {
            self.anchor_secs + self.anchor_at.elapsed().as_secs_f64()
        } else {
            self.anchor_secs
        }
    }

    /// Re-ancora (seek): agulha salta imediatamente, sem drift.
    pub fn seek(&mut self, secs: f64) {
        self.anchor_secs = secs.max(0.0);
        self.anchor_at = Instant::now();
    }
}

/// Throttler de seek: agrupa N pedidos num por frame (~30 Hz).
/// É o que elimina o "ímã": a UI nunca espera o motor.
pub struct SeekThrottle {
    pending: Cell<Option<f64>>,
    last_sent: Cell<Instant>,
    min_interval: Duration,
}

impl SeekThrottle {
    pub fn new() -> Self {
        Self {
            pending: Cell::new(None),
            last_sent: Cell::new(Instant::now() - Duration::from_secs(1)),
            min_interval: Duration::from_millis(33),
        }
    }

    /// Registra um pedido de seek vindo da UI.
    pub fn request(&self, secs: f64) {
        self.pending.set(Some(secs));
    }

    /// Retorna o seek a enviar agora (se o intervalo passou).
    pub fn due(&self) -> Option<f64> {
        let p = self.pending.take()?;
        if self.last_sent.get().elapsed() >= self.min_interval || !p.is_finite() {
            self.last_sent.set(Instant::now());
            Some(p)
        } else {
            // Ainda cedo: devolve pra próxima rodada.
            self.pending.set(Some(p));
            None
        }
    }

    /// Força envio imediato (usado no release do mouse).
    pub fn flush(&self) -> Option<f64> {
        self.pending.take()
    }
}
