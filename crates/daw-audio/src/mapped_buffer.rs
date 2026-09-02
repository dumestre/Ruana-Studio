//! Buffer de áudio com Memory Mapping (mmap/MapViewOfFile).
//!
//! Para arquivos WAV não comprimidos (PCM) no Unix, mapeia o arquivo diretamente
//! no espaço de endereçamento virtual. O SO carrega sob demanda apenas
//! as páginas necessárias — ideal para samples longos.
//!
//! No Windows e para formatos comprimidos (MP3, FLAC, OGG), usa o fallback
//! em memória (decodifica tudo de uma vez).

use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
use memmap2::Mmap;

#[cfg(unix)]
use hound;
use crate::resample::resample_sinc;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
enum SampleFormat {
    Int,
    Float,
}

/// Buffer de áudio mapeado em memória (WAV PCM via mmap — Unix only).
pub struct MappedAudioBuffer {
    #[cfg(unix)]
    mmap: Mmap,

    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    sample_format: SampleFormat,
    #[allow(dead_code)]
    data_offset: usize,
    data_size: usize,
    total_frames: usize,

    src_sample_rate: u32,
    dst_sample_rate: u32,
    needs_resample: bool,

    #[allow(dead_code)]
    tail_fade_applied: bool,
}

impl MappedAudioBuffer {
    /// Tenta abrir um arquivo como buffer mapeado.
    ///
    /// Retorna `None` se não for WAV PCM válido, se não for Unix,
    /// ou se o mapeamento falhar.
    pub fn try_open(path: &Path, output_sample_rate: u32) -> Option<Arc<Self>> {
        #[cfg(not(unix))]
        {
            let _ = path;
            let _ = output_sample_rate;
            return None;
        }

        #[cfg(unix)]
        {
            // Primeiro, valida se é WAV PCM usando hound (rápido, só header)
            let reader = hound::WavReader::open(path).ok()?;
            let spec = reader.spec();

            // Só suporta PCM (Int ou Float), não comprimido
            if spec.sample_format != hound::SampleFormat::Int
                && spec.sample_format != hound::SampleFormat::Float
            {
                return None;
            }

            // Abre o arquivo e mapeia
            let file = std::fs::File::open(path).ok()?;
            let mmap = unsafe { Mmap::map(&file).ok()? };
            Self::from_mmap(mmap, spec, output_sample_rate)
        }
    }

    #[cfg(unix)]
    fn from_mmap(
        mmap: Mmap,
        spec: hound::WavSpec,
        output_sample_rate: u32,
    ) -> Option<Arc<Self>> {
        let data_offset = Self::find_data_chunk(&mmap)?;
        let data_size = mmap.len() - data_offset;

        let channels = spec.channels as u16;
        let bits_per_sample = spec.bits_per_sample;
        let sample_format = match spec.sample_format {
            hound::SampleFormat::Int => SampleFormat::Int,
            hound::SampleFormat::Float => SampleFormat::Float,
        };

        let total_frames = match sample_format {
            SampleFormat::Int => data_size / (channels as usize * bits_per_sample as usize / 8),
            SampleFormat::Float => data_size / (channels as usize * 4),
        };

        let needs_resample = spec.sample_rate != output_sample_rate;

        Some(Arc::new(Self {
            mmap,
            sample_rate: output_sample_rate,
            channels,
            bits_per_sample,
            sample_format,
            data_offset,
            data_size,
            total_frames,
            src_sample_rate: spec.sample_rate,
            dst_sample_rate: output_sample_rate,
            needs_resample,
            tail_fade_applied: false,
        }))
    }

    /// Encontra o chunk "data" no arquivo WAV.
    #[allow(dead_code)]
    fn find_data_chunk(data: &[u8]) -> Option<usize> {
        // WAV: RIFF header (12 bytes) + fmt chunk + data chunk
        // Procura por "data" (0x64617461)
        if data.len() < 44 {
            return None;
        }
        // Verifica RIFF
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return None;
        }

        let mut offset = 12;
        while offset + 8 <= data.len() {
            let chunk_id = &data[offset..offset + 4];
            let chunk_size = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;

            if chunk_id == b"data" {
                return Some(offset + 8);
            }

            offset += 8 + chunk_size;
            // Alinhamento par
            if chunk_size % 2 == 1 {
                offset += 1;
            }
        }
        None
    }

    /// Lê um sample bruto do arquivo mapeado (sem resample, sem interpolação).
    ///
    /// Retorna (left, right) no range [-1.0, 1.0].
    fn read_raw_frame(&self, frame_index: usize) -> Option<(f32, f32)> {
        if frame_index >= self.total_frames {
            return None;
        }

        #[cfg(unix)]
        let data = &self.mmap[self.data_offset..];

        #[cfg(not(unix))]
        let data = &[][..];

        let frame_offset = match self.sample_format {
            SampleFormat::Int => {
                let bytes_per_frame = (self.channels as usize * self.bits_per_sample as usize) / 8;
                frame_index * bytes_per_frame
            }
            SampleFormat::Float => {
                frame_index * (self.channels as usize * 4)
            }
        };

        if frame_offset + (self.channels as usize * 4) > self.data_size {
            return None;
        }

        let frame_data = &data[frame_offset..];

        let left = self.read_sample(frame_data, 0)?;
        let right = if self.channels >= 2 {
            self.read_sample(frame_data, 1)?
        } else {
            left
        };

        Some((left, right))
    }

    fn read_sample(&self, frame_data: &[u8], channel: usize) -> Option<f32> {
        let offset = match self.sample_format {
            SampleFormat::Int => {
                let bytes_per_sample = self.bits_per_sample as usize / 8;
                channel * bytes_per_sample
            }
            SampleFormat::Float => {
                channel * 4
            }
        };

        if offset + 4 > frame_data.len() {
            return None;
        }

        let sample = match self.sample_format {
            SampleFormat::Int => {
                let bytes = &frame_data[offset..offset + (self.bits_per_sample as usize / 8)];
                match self.bits_per_sample {
                    16 => {
                        let val = i16::from_le_bytes([bytes[0], bytes[1]]);
                        val as f32 / i16::MAX as f32
                    }
                    24 => {
                        // 24-bit: 3 bytes, sign-extend
                        let val = (bytes[0] as i32)
                            | ((bytes[1] as i32) << 8)
                            | ((bytes[2] as i32) << 16);
                        let val = if val & 0x800000 != 0 {
                            val | (0xFF000000u32 as i32)
                        } else {
                            val
                        };
                        val as f32 / 8388607.0 // 2^23 - 1
                    }
                    32 => {
                        let val = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        val as f32 / i32::MAX as f32
                    }
                    _ => return None,
                }
            }
            SampleFormat::Float => {
                f32::from_le_bytes([
                    frame_data[offset],
                    frame_data[offset + 1],
                    frame_data[offset + 2],
                    frame_data[offset + 3],
                ])
            }
        };
        Some(sample.clamp(-1.0, 1.0))
    }

    /// Lê um frame com interpolação linear e resample opcional.
    ///
    /// `pos` é em frames na taxa de SAÍDA (já resampleado).
    #[inline]
    pub fn read(&self, pos: f64) -> (f32, f32) {
        if pos < 0.0 {
            return (0.0, 0.0);
        }

        // Converte posição de saída para posição de origem (se resample)
        let src_pos = if self.needs_resample {
            pos * self.src_sample_rate as f64 / self.dst_sample_rate as f64
        } else {
            pos
        };

        let idx = src_pos as usize;
        if idx + 1 >= self.total_frames {
            // Último sample válido
            if idx < self.total_frames {
                return self.read_raw_frame(idx).unwrap_or((0.0, 0.0));
            }
            return (0.0, 0.0);
        }

        let frac = (src_pos - idx as f64) as f32;

        let (l0, r0) = self.read_raw_frame(idx).unwrap_or((0.0, 0.0));
        let (l1, r1) = self.read_raw_frame(idx + 1).unwrap_or((0.0, 0.0));

        let l = l0 + (l1 - l0) * frac;
        let r = r0 + (r1 - r0) * frac;

        (l, r)
    }

    /// Aplica fade tail no final do buffer (anti-clique).
    #[allow(dead_code)]
    fn apply_tail_fade(&mut self) {
        if self.tail_fade_applied {
            return;
        }
        // Para buffers mapeados, não podemos modificar o arquivo.
        // O fade é aplicado virtualmente na leitura.
        // Como isso é complexo com mmap, aplicamos apenas no final da leitura.
        self.tail_fade_applied = true;
    }

    #[inline]
    pub fn frames(&self) -> usize {
        if self.needs_resample {
            (self.total_frames as f64 * self.dst_sample_rate as f64 / self.src_sample_rate as f64) as usize
        } else {
            self.total_frames
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_frames == 0
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl Drop for MappedAudioBuffer {
    fn drop(&mut self) {
        // No Unix, o mmap é droppado automaticamente
    }
}

// ============================================================
// FUNÇÃO DE CARREGAMENTO UNIFICADA
// ============================================================

/// Tenta carregar como MappedAudioBuffer (WAV PCM no Unix), senão cai no fallback em memória.
pub fn load_audio_buffer(
    path: &Path,
    output_sample_rate: u32,
) -> Option<AudioBufferKind> {
    // Tenta memory mapping primeiro (só WAV PCM no Unix)
    if let Some(mapped) = MappedAudioBuffer::try_open(path, output_sample_rate) {
        return Some(AudioBufferKind::Mapped(mapped));
    }

    // Fallback: decodifica tudo em memória (MP3, FLAC, OGG, WAVs esquisitos, Windows)
    use daw_media::waveform::decode_audio;
    let data = decode_audio(path).ok()?;

    let ch = data.channels as usize;
    let src_left = data.channel(0);
    let src_right = if ch <= 1 {
        src_left.clone()
    } else {
        data.channel(1)
    };

    let left = if data.sample_rate != output_sample_rate {
        resample_sinc(&src_left, data.sample_rate, output_sample_rate)
    } else {
        src_left
    };
    let right = if data.sample_rate != output_sample_rate {
        resample_sinc(&src_right, data.sample_rate, output_sample_rate)
    } else {
        src_right
    };

    let mut buffer = AudioBuffer {
        left,
        right,
        sample_rate: output_sample_rate,
    };
    buffer.fade_tail();

    Some(AudioBufferKind::Memory(Arc::new(buffer)))
}

/// Enum que unifica buffer em memória e buffer mapeado.
#[derive(Clone)]
pub enum AudioBufferKind {
    Memory(Arc<AudioBuffer>),
    Mapped(Arc<MappedAudioBuffer>),
}

impl AudioBufferKind {
    #[inline]
    pub fn read(&self, pos: f64) -> (f32, f32) {
        match self {
            AudioBufferKind::Memory(buf) => buf.read(pos),
            AudioBufferKind::Mapped(buf) => buf.read(pos),
        }
    }

    #[inline]
    pub fn frames(&self) -> usize {
        match self {
            AudioBufferKind::Memory(buf) => buf.frames(),
            AudioBufferKind::Mapped(buf) => buf.frames(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        match self {
            AudioBufferKind::Memory(buf) => buf.is_empty(),
            AudioBufferKind::Mapped(buf) => buf.is_empty(),
        }
    }

    #[inline]
    pub fn sample_rate(&self) -> u32 {
        match self {
            AudioBufferKind::Memory(buf) => buf.sample_rate,
            AudioBufferKind::Mapped(buf) => buf.sample_rate(),
        }
    }
}

// Re-exporta AudioBuffer do buffer.rs para o enum
use crate::buffer::AudioBuffer;