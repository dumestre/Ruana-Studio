use std::collections::HashMap;

use daw_core::Clip;
use daw_media::waveform::decode_audio;

use crate::resample::resample_linear;

/// Buffer mono de samples para um clip, resampled ao sample rate de saída.
pub struct SampleBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl SampleBuffer {
    /// Lê um sample na posição dada (interpolação linear entre samples).
    pub fn read(&self, position: u64) -> f32 {
        let idx = position as usize;
        if idx >= self.samples.len() {
            return 0.0;
        }
        self.samples[idx]
    }

    /// Quantidade total de samples neste buffer.
    pub fn len_samples(&self) -> u64 {
        self.samples.len() as u64
    }
}

/// Cache lazy de buffers de áudio por clip.
/// Decodifica e resample sob demanda, cacheia o resultado.
pub struct BufferCache {
    cache: HashMap<u64, SampleBuffer>,
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

    /// Decodifica + resample sob demanda. Retorna referência cacheada.
    pub fn get_or_load(&mut self, clip: &Clip) -> &SampleBuffer {
        let clip_id = clip.id;
        let output_rate = self.output_sample_rate;

        self.cache.entry(clip_id).or_insert_with(|| {
            let path = clip
                .file_path
                .as_ref()
                .expect("audio clip sem file_path");

            let data = decode_audio(path)
                .unwrap_or_else(|e| panic!("falha ao decodificar {:?}: {}", path, e));

            let resampled =
                resample_linear(&data.samples, data.sample_rate, output_rate);

            SampleBuffer {
                samples: resampled,
                sample_rate: output_rate,
            }
        })
    }

    /// Retorna referência a um buffer já cacheado pelo clip ID.
    /// Não decodifica — retorna None se não estiver em cache.
    pub fn get_by_id(&self, clip_id: u64) -> Option<&SampleBuffer> {
        self.cache.get(&clip_id)
    }

    /// Remove o buffer de um clip (quando deletado do projeto).
    pub fn evict(&mut self, clip_id: u64) {
        self.cache.remove(&clip_id);
    }

    /// Limpa todo o cache.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Retorna a quantidade de clips em cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Clip;

    #[test]
    fn buffer_cache_evict() {
        let mut cache = BufferCache::new(48000);
        assert_eq!(cache.len(), 0);
        cache.evict(999);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn sample_buffer_read_out_of_bounds() {
        let buf = SampleBuffer {
            samples: vec![0.5, -0.5],
            sample_rate: 48000,
        };
        assert_eq!(buf.read(0), 0.5);
        assert_eq!(buf.read(1), -0.5);
        assert_eq!(buf.read(2), 0.0);
        assert_eq!(buf.len_samples(), 2);
    }
}
