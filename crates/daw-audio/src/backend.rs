use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

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
            buffer_size: 512,
            channels: 2,
        }
    }
}

/// Backend CPAL — abre a stream de saída de áudio do sistema.
pub struct CpalBackend {
    _stream: Stream,
    config: AudioConfig,
}

impl CpalBackend {
    /// Cria e inicia o backend CPAL com o callback dado.
    /// O callback recebe um slice f32 stereo intercalado (L, R, L, R, ...).
    pub fn new(
        mut callback: impl FnMut(&mut [f32]) + Send + 'static,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDefaultDevice)?;

        let supported = device
            .default_output_config()
            .map_err(|_| AudioError::NoSupportedConfig)?;

        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();

        let sample_rate = config.sample_rate;
        let channels = config.channels as u16;
        let buffer_size = match &config.buffer_size {
            cpal::BufferSize::Default => 512,
            cpal::BufferSize::Fixed(n) => *n,
        };

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
