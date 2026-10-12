use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

/// Erros do audio backend.
#[derive(Debug)]
pub enum AudioError {
    NoDefaultDevice,
    NoSupportedConfig,
    BuildStream(cpal::Error),
    PlayStream(cpal::Error),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDefaultDevice => write!(f, "nenhum dispositivo de áudio encontrado"),
            AudioError::NoSupportedConfig => write!(f, "configuração de áudio não suportada"),
            AudioError::BuildStream(e) => write!(f, "erro ao criar stream: {}", e),
            AudioError::PlayStream(e) => write!(f, "erro ao iniciar stream: {}", e),
        }
    }
}

impl std::error::Error for AudioError {}

/// Configuração do audio backend.
pub struct AudioConfig {
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            buffer_size: 2048,
            channels: 2,
        }
    }
}

/// Buffers pequenos (< 512) fazem a thread RT de áudio derrubar amostras
/// quando a CPU também está renderizando a UI (Skia + wgpu). O padrão do
/// `BufferSize::Default` varia muito entre hosts (WASAPI costuma dar ~10ms,
/// mas hosts "low-latency" podem dar menos que 2ms) e o `cpal` não expõe o
/// valor real — daí a escolha explícita de um buffer grande aqui.
const MIN_BUFFER_FRAMES: u32 = 1024;

/// Envia a configuração preferida para o log de diagnóstico (só com
/// `RUANA_LOG` definido).
fn log_chosen_config(sample_rate: u32, buffer_size: u32, channels: u16) {
    if std::env::var("RUANA_LOG").is_ok() {
        let ms = buffer_size as f64 / sample_rate as f64 * 1000.0;
        eprintln!(
            "[backend] usando {} Hz / {} frames (~{:.1} ms) / {} canais",
            sample_rate, buffer_size, ms, channels
        );
    }
}

/// Escolhe a melhor configuração de saída suportada pelo dispositivo.
///
/// Estratégia: um buffer de pelo menos [`MIN_BUFFER_FRAMES`] frames é
/// prioridade, porque o app renderiza a UI ao mesmo tempo que toca áudio.
/// Sem isso o RT do cpal estoura ("granulado", "falhas", "buffer baixo").
/// O fallback é o `default_output_config()` — que o `cpal` mantém em
/// `BufferSize::Default` e o `CpalBackend::new` traduz para um valor fixo.
fn pick_supported_config(
    device: &cpal::Device,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), AudioError> {
    let default = device
        .default_output_config()
        .map_err(|_| AudioError::NoSupportedConfig)?;

    // A taxa padrão do dispositivo (o motor assume 48 kHz, mas o cpal
    // pode devolver outra — ex.: 44.1 kHz em drivers baratos).
    let want_rate = default.sample_rate();

    for range in device
        .supported_output_configs()
        .map_err(|_| AudioError::NoSupportedConfig)?
    {
        if range.channels() != default.channels() {
            continue;
        }
        if range.max_sample_rate() < want_rate || range.min_sample_rate() > want_rate {
            continue;
        }
        // O buffer pode ser `Range { min, max }` ou `Unknown`.
        let max_buf = match range.buffer_size() {
            cpal::SupportedBufferSize::Range { max, .. } => *max,
            cpal::SupportedBufferSize::Unknown => 0,
        };
        if max_buf >= MIN_BUFFER_FRAMES {
            let candidate = range.with_sample_rate(want_rate);
            let format = candidate.sample_format();
            return Ok((candidate.into(), format));
        }
    }

    let format = default.sample_format();
    Ok((default.into(), format))
}

/// Backend CPAL — abre a stream de saída de áudio do sistema.
pub struct CpalBackend {
    _stream: Stream,
    config: AudioConfig,
}

impl CpalBackend {
    /// Cria e inicia o backend CPAL com o callback dado.
    /// O callback recebe um slice f32 stereo intercalado (L, R, L, R, ...).
    pub fn new(mut callback: impl FnMut(&mut [f32]) + Send + 'static) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDefaultDevice)?;

        let (config, sample_format) = pick_supported_config(&device)?;

        let sample_rate = config.sample_rate;
        let channels = config.channels as u16;
        let buffer_size = match &config.buffer_size {
            // Hosts que só aceitam `Default` continuam funcionando — o
            // valor é um número RAZOÁVEL (não mais 512 fixos).
            cpal::BufferSize::Default => 1024,
            cpal::BufferSize::Fixed(n) => *n,
        };

        log_chosen_config(sample_rate, buffer_size, channels);

        let config_info = AudioConfig {
            sample_rate,
            buffer_size,
            channels,
        };

        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                config.clone(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    callback(data);
                },
                |err| eprintln!("audio stream error: {}", err),
                None,
            ),
            SampleFormat::I16 => {
                let mut inner_callback = Some(callback);
                device.build_output_stream(
                    config.clone(),
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        let cb = inner_callback.as_mut().unwrap();
                        let mut f32_buf = vec![0.0f32; data.len()];
                        cb(&mut f32_buf);
                        for (out, &s) in data.iter_mut().zip(f32_buf.iter()) {
                            *out = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                    },
                    |err| eprintln!("audio stream error: {}", err),
                    None,
                )
            }
            SampleFormat::U16 => {
                let mut inner_callback = Some(callback);
                device.build_output_stream(
                    config.clone(),
                    move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                        let cb = inner_callback.as_mut().unwrap();
                        let mut f32_buf = vec![0.0f32; data.len()];
                        cb(&mut f32_buf);
                        for (out, &s) in data.iter_mut().zip(f32_buf.iter()) {
                            *out = ((s.clamp(-1.0, 1.0) + 1.0) * 0.5 * u16::MAX as f32) as u16;
                        }
                    },
                    |err| eprintln!("audio stream error: {}", err),
                    None,
                )
            }
            _ => return Err(AudioError::NoSupportedConfig),
        }
        .map_err(AudioError::BuildStream)?;

        stream.play().map_err(AudioError::PlayStream)?;

        Ok(Self {
            _stream: stream,
            config: config_info,
        })
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }
}
