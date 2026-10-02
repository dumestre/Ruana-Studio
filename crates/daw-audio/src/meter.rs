//! Medição de nível para os VU meters.
//!
//! # O que a engine mede e o que a UI mede
//!
//! A engine publica **só o pico do buffer** (L e R), por canal. Isso é
//! medição, não apresentação: a barra do VU, o peak-hold e a escala em
//! dB dependem da taxa de atualização da tela, não da taxa de áudio —
//! fazer isso aqui significaria encolher o smooth em cima de uma taxa de
//! 48 kHz e o resultado seria liso e sem vida.
//!
//! Já a curva em dB, o decay e o peak-hold ficam em `app/src/meters.rs`,
//! na thread da UI. Um pico por buffer de áudio é a MEIA da taxa de
//! atualização do medidor: se a UI lê mais rápido que isso, repete o
//! mesmo valor, e o ataque instantâneo do VU já absorve a diferença.
//!
//! # Por que atômicos de tamanho fixo
//!
//! O banco é alocado uma vez, com [`MAX_METER_CHANNELS`] canais, e
//! nunca cresce nem encolhe. A thread RT escreve por índice com
//! `Ordering::Relaxed` (a mesma disciplina de `engine.rs` para a taxa
//! de amostragem) e a thread da UI lê. Se o banco crescesse a partir de
//! um `Vec`, um `resize` na UI reallocaria o buffer por baixo da RT —
//! data race, não um bug "raro". Capacidade fixa + índice com guarda
//! elimina a classe inteira de problema.
//!
//! O latch de clip é um bit por canal, ligado com `fetch_or` e
//! desligado por [`MeterBank::clear_clip`]. A engine nunca o desliga:
//! um clip que se apaga sozinho não serve para nada — o usuário precisa
//! ver o que estourou até clicar nele.

use std::sync::atomic::{AtomicU32, Ordering};

/// Capacidade de canais medidos (trilhas + master). Fixa: ver módulo.
pub const MAX_METER_CHANNELS: usize = 128;

/// Floats por canal no banco: pico L, pico R.
pub const FLOATS_PER_CHANNEL: usize = 2;

/// Bit de clip do canal esquerdo.
pub const CLIP_L: u32 = 0b01;
/// Bit de clip do canal direito.
pub const CLIP_R: u32 = 0b10;

/// Leitura de um canal, já desbitada.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    /// Pico absoluto do lado esquerdo no último buffer renderizado.
    pub peak_left: f32,
    /// Pico absoluto do lado direito no último buffer renderizado.
    pub peak_right: f32,
    pub clipped_left: bool,
    pub clipped_right: bool,
}

/// Ponte de medição entre a thread RT e a thread da UI.
pub struct MeterBank {
    /// `FLOATS_PER_CHANNEL` floats por canal, em `f32::to_bits`.
    values: Box<[AtomicU32]>,
    /// Bitmask de clip por canal (`CLIP_L`/`CLIP_R`).
    clipped: Box<[AtomicU32]>,
    /// Canais válidos (trilhas + master). Escrito só pela UI.
    count: AtomicU32,
}

impl MeterBank {
    pub fn new() -> Self {
        let values: Vec<AtomicU32> = (0..MAX_METER_CHANNELS * FLOATS_PER_CHANNEL)
            .map(|_| AtomicU32::new(0))
            .collect();
        let clipped: Vec<AtomicU32> = (0..MAX_METER_CHANNELS).map(|_| AtomicU32::new(0)).collect();
        Self {
            values: values.into_boxed_slice(),
            clipped: clipped.into_boxed_slice(),
            count: AtomicU32::new(0),
        }
    }

    /// Declara quantos canais existem (trilhas + master).
    ///
    /// Só a contagem muda; a capacidade é fixa. Canais além do
    /// declarado não são lidos nem publicados.
    pub fn set_channel_count(&self, channels: usize) {
        self.count.store(channels.min(MAX_METER_CHANNELS) as u32, Ordering::Relaxed);
    }

    /// Canais válidos no momento.
    pub fn channel_count(&self) -> usize {
        self.count.load(Ordering::Relaxed) as usize
    }

    /// Leitura do canal. Canal fora do range devolve silêncio.
    pub fn read(&self, channel: usize) -> MeterReading {
        if channel >= self.channel_count() {
            return MeterReading::default();
        }
        let base = channel * FLOATS_PER_CHANNEL;
        let mask = self.clipped[channel].load(Ordering::Relaxed);
        MeterReading {
            peak_left: f32::from_bits(self.values[base].load(Ordering::Relaxed)),
            peak_right: f32::from_bits(self.values[base + 1].load(Ordering::Relaxed)),
            clipped_left: mask & CLIP_L != 0,
            clipped_right: mask & CLIP_R != 0,
        }
    }

    /// Limpa o latch de clip de um canal (clique no medidor).
    pub fn clear_clip(&self, channel: usize) {
        if channel < MAX_METER_CHANNELS {
            self.clipped[channel].store(0, Ordering::Relaxed);
        }
    }

    /// Limpa todos os latches (troca de projeto, play novo).
    pub fn clear_all_clips(&self) {
        for cell in self.clipped.iter() {
            cell.store(0, Ordering::Relaxed);
        }
    }

    /// Publica o pico do buffer. **Chamada da thread RT.**
    ///
    /// `clip` é a bitmask latched de clip deste buffer — quem chama
    /// decide se o sinal passou de 1.0, porque o ponto de medição
    /// (antes ou depois do limitador) é escolha da engine, não do banco.
    pub(crate) fn publish(&self, channel: usize, peak_left: f32, peak_right: f32, clip: u32) {
        if channel >= MAX_METER_CHANNELS {
            return;
        }
        let base = channel * FLOATS_PER_CHANNEL;
        self.values[base].store(peak_left.to_bits(), Ordering::Relaxed);
        self.values[base + 1].store(peak_right.to_bits(), Ordering::Relaxed);
        if clip != 0 {
            self.clipped[channel].fetch_or(clip, Ordering::Relaxed);
        }
    }
}

impl Default for MeterBank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_read_roundtrip() {
        let bank = MeterBank::new();
        bank.set_channel_count(2);
        bank.publish(0, 0.5, 0.25, 0);
        let r = bank.read(0);
        assert_eq!(r.peak_left, 0.5);
        assert_eq!(r.peak_right, 0.25);
        assert!(!r.clipped_left && !r.clipped_right);
    }

    #[test]
    fn channels_outside_the_count_are_silent() {
        let bank = MeterBank::new();
        bank.set_channel_count(1);
        bank.publish(1, 1.0, 1.0, 0);
        // O canal 1 foi escrito mas não está declarado: a UI não pode
        // ver valor de um canal que não existe.
        assert_eq!(bank.read(1), MeterReading::default());
    }

    #[test]
    fn clip_latches_until_cleared() {
        let bank = MeterBank::new();
        bank.set_channel_count(1);
        bank.publish(0, 1.2, 0.1, CLIP_L);
        assert!(bank.read(0).clipped_left);
        assert!(!bank.read(0).clipped_right);

        // Continua latched mesmo quando o sinal volta ao normal.
        bank.publish(0, 0.1, 0.1, 0);
        assert!(bank.read(0).clipped_left);

        bank.clear_clip(0);
        assert!(!bank.read(0).clipped_left);
    }

    #[test]
    fn clip_accumulates_both_sides() {
        let bank = MeterBank::new();
        bank.set_channel_count(1);
        bank.publish(0, 2.0, 0.5, CLIP_L);
        bank.publish(0, 0.5, 2.0, CLIP_R);
        let r = bank.read(0);
        assert!(r.clipped_left && r.clipped_right);
    }

    #[test]
    fn write_past_capacity_is_ignored() {
        let bank = MeterBank::new();
        bank.set_channel_count(MAX_METER_CHANNELS);
        bank.publish(MAX_METER_CHANNELS + 5, 1.0, 1.0, CLIP_L);
        assert_eq!(bank.read(MAX_METER_CHANNELS).peak_left, 0.0);
    }

    #[test]
    fn clear_all_wipes_every_channel() {
        let bank = MeterBank::new();
        bank.set_channel_count(3);
        bank.publish(0, 2.0, 2.0, CLIP_L | CLIP_R);
        bank.publish(2, 2.0, 2.0, CLIP_L);
        bank.clear_all_clips();
        assert!(!bank.read(0).clipped_left && !bank.read(2).clipped_right);
    }
}
