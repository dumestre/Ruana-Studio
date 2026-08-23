//! Buffers de áudio decodificados, prontos para a thread RT.
//!
//! Decisões que importam para o som:
//!
//! - **Stereo de verdade.** O buffer guarda L e R separados. Somar
//!   os canais em mono cancelava conteúdo fora de fase e achatava a
//!   imagem — o sample não saía igual ao original.
//! - **Resample no carregamento**, uma vez, para o sample rate do
//!   dispositivo. Assim a leitura em tempo real é 1:1 e não precisa
//!   converter taxa por frame.
//! - **Indexado por path** (não por clip): o mesmo sample usado em
//!   vinte clips decodifica uma vez só, e as linhas do beat
//!   sequencer compartilham o mesmo buffer.
//! - **`Arc`**: a thread de áudio recebe um ponteiro clonado junto
//!   com o snapshot. Ela nunca trava o cache nem toca em disco.

use std::collections::HashMap;
use std::sync::Arc;

use daw_media::waveform::decode_audio;

use crate::resample::resample_sinc;

/// Rampa anti-clique aplicada ao fim de qualquer buffer, em frames.
///
/// Um sample cortado no meio de um ciclo produz um degrau — e degrau
/// é clique. ~1ms de fade resolve sem alterar o ataque.
const TAIL_FADE: usize = 48;

/// Buffer stereo já no sample rate de saída.
pub struct AudioBuffer {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioBuffer {
    /// Frames (samples por canal).
    #[inline]
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    /// Lê um frame com interpolação linear.
    ///
    /// A posição é `f64` de propósito: o início de um clip raramente
    /// cai num sample inteiro, e ler com `round()` reintroduziria a
    /// quantização que deixava o áudio granulado.
    #[inline]
    pub fn read(&self, pos: f64) -> (f32, f32) {
        if pos < 0.0 {
            return (0.0, 0.0);
        }
        let idx = pos as usize;
        if idx + 1 >= self.left.len() {
            // Último sample: sem próximo para interpolar.
            if idx < self.left.len() {
                return (self.left[idx], self.right[idx]);
            }
            return (0.0, 0.0);
        }
        let frac = (pos - idx as f64) as f32;
        let l = self.left[idx] + (self.left[idx + 1] - self.left[idx]) * frac;
        let r = self.right[idx] + (self.right[idx + 1] - self.right[idx]) * frac;
        (l, r)
    }

    /// Aplica a rampa anti-clique no fim do buffer.
    fn fade_tail(&mut self) {
        let n = TAIL_FADE.min(self.frames());
        if n == 0 {
            return;
        }
        let start = self.frames() - n;
        for i in 0..n {
            let g = 1.0 - (i as f32 + 1.0) / n as f32;
            self.left[start + i] *= g;
            self.right[start + i] *= g;
        }
    }
}

/// Cache de buffers por path do arquivo.
///
/// Guarda também as falhas (`None`): sem isso, um path inválido
/// tentaria decodificar de novo a cada sincronização de snapshot.
pub struct BufferCache {
    cache: HashMap<String, Option<Arc<AudioBuffer>>>,
    output_sample_rate: u32,
}

impl BufferCache {
    pub fn new(output_sample_rate: u32) -> Self {
        Self {
            cache: HashMap::new(),
            output_sample_rate,
        }
    }

    pub fn output_sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    /// Troca o sample rate de saída e invalida o cache.
    ///
    /// Os buffers guardados estão resampleados para a taxa antiga;
    /// mantê-los tocaria tudo com pitch errado.
    pub fn set_output_sample_rate(&mut self, rate: u32) {
        if rate != self.output_sample_rate {
            self.output_sample_rate = rate;
            self.cache.clear();
        }
    }

    /// Decodifica sob demanda e cacheia. `None` = falhou.
    ///
    /// Roda na thread principal (decodificar pode levar centenas de
    /// milissegundos) — NUNCA chamar da thread de áudio.
    pub fn load(&mut self, path: &str) -> Option<Arc<AudioBuffer>> {
        if let Some(hit) = self.cache.get(path) {
            return hit.clone();
        }

        let built = Self::decode(path, self.output_sample_rate);
        if built.is_none() {
            eprintln!("audio: falha ao carregar {}", path);
        }
        self.cache.insert(path.to_string(), built.clone());
        built
    }

    /// Só consulta — não decodifica.
    pub fn get(&self, path: &str) -> Option<Arc<AudioBuffer>> {
        self.cache.get(path).cloned().flatten()
    }

    fn decode(path: &str, output_rate: u32) -> Option<Arc<AudioBuffer>> {
        let data = decode_audio(path).ok()?;
        if data.interleaved.is_empty() {
            return None;
        }

        let src_left = data.channel(0);
        // Mono: o mesmo conteúdo nos dois lados (centro real, sem
        // metade do ganho de um lado só).
        let src_right = if data.channels <= 1 {
            src_left.clone()
        } else {
            data.channel(1)
        };

        let mut buffer = AudioBuffer {
            left: resample_sinc(&src_left, data.sample_rate, output_rate),
            right: resample_sinc(&src_right, data.sample_rate, output_rate),
            sample_rate: output_rate,
        };

        // Os dois canais precisam ter exatamente o mesmo tamanho:
        // a leitura indexa os dois com o mesmo cursor.
        let len = buffer.left.len().min(buffer.right.len());
        buffer.left.truncate(len);
        buffer.right.truncate(len);
        if len == 0 {
            return None;
        }

        buffer.fade_tail();
        Some(Arc::new(buffer))
    }

    /// Remove um buffer do cache (arquivo trocado em disco).
    pub fn evict(&mut self, path: &str) {
        self.cache.remove(path);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(left: Vec<f32>, right: Vec<f32>) -> AudioBuffer {
        AudioBuffer {
            left,
            right,
            sample_rate: 48000,
        }
    }

    #[test]
    fn read_interpolates_between_samples() {
        let b = buf(vec![0.0, 1.0], vec![0.0, -1.0]);
        let (l, r) = b.read(0.5);
        assert!((l - 0.5).abs() < 1e-6);
        assert!((r + 0.5).abs() < 1e-6);
    }

    #[test]
    fn read_out_of_bounds_is_silent() {
        let b = buf(vec![0.5, 0.5], vec![0.5, 0.5]);
        assert_eq!(b.read(-1.0), (0.0, 0.0));
        assert_eq!(b.read(9.0), (0.0, 0.0));
        // Último sample válido: devolve o valor, sem interpolar.
        assert_eq!(b.read(1.0), (0.5, 0.5));
    }

    #[test]
    fn channels_stay_independent() {
        let b = buf(vec![1.0, 1.0], vec![-1.0, -1.0]);
        let (l, r) = b.read(0.0);
        assert_eq!(l, 1.0);
        assert_eq!(r, -1.0);
    }

    #[test]
    fn fade_tail_ends_at_zero() {
        let n = TAIL_FADE + 10;
        let mut b = buf(vec![1.0; n], vec![1.0; n]);
        b.fade_tail();
        assert_eq!(b.left[0], 1.0);
        assert!(*b.left.last().unwrap() < 0.05);
        assert!(*b.right.last().unwrap() < 0.05);
    }

    #[test]
    fn cache_starts_empty_and_evict_is_safe() {
        let mut cache = BufferCache::new(48000);
        assert!(cache.is_empty());
        cache.evict("nao/existe.wav");
        assert_eq!(cache.len(), 0);
        assert!(cache.get("nao/existe.wav").is_none());
    }

    #[test]
    fn rate_change_invalidates() {
        let mut cache = BufferCache::new(48000);
        cache
            .cache
            .insert("x.wav".into(), Some(Arc::new(buf(vec![0.0], vec![0.0]))));
        assert_eq!(cache.len(), 1);
        cache.set_output_sample_rate(44100);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.output_sample_rate(), 44100);
    }
}
