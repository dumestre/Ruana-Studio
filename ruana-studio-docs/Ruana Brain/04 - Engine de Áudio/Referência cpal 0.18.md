# Referência cpal 0.18

> Referência do cpal usado no RuanaStudio (0.18).

---

## Fontes oficiais

- **cpal docs**: https://docs.rs/cpal/
- **cpal GitHub**: https://github.com/rustaudio/cpal
- **cpal releases**: https://github.com/rustaudio/cpal/releases

## Visão geral

cpal é uma biblioteca de I/O de áudio de baixo nível, cross-platform, escrita em Rust. Para playback e capture de áudio de mais alto nível, considere Rodio ou similar.

## Backends suportados

| Plataforma | Backend |
|---|---|
| Windows | WASAPI, ASIO (via feature) |
| macOS | CoreAudio |
| Linux | ALSA, PulseAudio, PipeWire, JACK (via features) |
| Android | Oboe |
| Web | WASM (audioworklet) |

## Features do projeto

```toml
[features]
default = ["cpal-out"]
cpal-out = ["dep:cpal"]
jack = ["cpal-out", "cpal/jack"]
```

## Conceitos principais

- **Host**: abstração do sistema de áudio
- **Device**: dispositivo de áudio físico ou virtual
- **Stream**: fluxo de áudio ativo (input ou output)
- **Config**: formato do stream (sample rate, channels, buffer size)
- **Callback**: função chamada pela thread de áudio para cada buffer

## API principal

```rust
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// Host padrão
let host = cpal::default_host();

// Device de output padrão
let device = host.default_output_device().expect("no output device");

// Config suportada
let config = device.default_output_config().unwrap();

// Criar stream
let stream = device.build_output_stream(
    &config.config(),
    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        // Callback de áudio — chamado em thread de alta prioridade
        // NÃO bloquear, NÃO alocar, NÃO fazer I/O
        for sample in data.iter_mut() {
            *sample = 0.0;  // Silêncio
        }
    },
    move |err| {
        eprintln!("Audio stream error: {}", err);
    },
    None,
).unwrap();

stream.play().unwrap();
```

## Config de stream

```rust
// Buffer size
let config = cpal::StreamConfig {
    channels: 2,
    sample_rate: cpal::SampleRate(48000),
    buffer_size: cpal::BufferSize::Fixed(1024),  // ou Default
};

// Descobrir configs suportadas
let configs: cpal::SupportedOutputConfigs = device.supported_output_configs().unwrap();
for config in configs {
    println!("{:?}", config);
}
```

## Regras críticas para callback de áudio

1. **NUNCA bloquear** — sem mutex, sem lock, sem channel send bloqueante
2. **NUNCA alocar** — sem `Vec::new()`, sem `String::from()`, sem `Box::new()`
3. **NUNCA fazer I/O** — sem file read/write, sem network
4. **NUNCA fazer logging** — sem `println!`, sem `eprintln!`
5. **Ser determinístico** — mesmo input → mesmo output, sempre

## Padrão do projeto: pick_supported_config

```rust
fn pick_supported_config(device: &cpal::Device) -> cpal::StreamConfig {
    let configs: Vec<_> = device.supported_output_configs()
        .unwrap()
        .collect();
    
    // Escolher buffer >= 1024 para evitar underrun
    let config = configs.iter()
        .find(|c| c.max_sample_rate().0 >= 48000)
        .or_else(|| configs.first())
        .expect("no supported config");
    
    let mut config = config.with_max_sample_rate();
    config.buffer_size = cpal::BufferSize::Fixed(1024);
    config
}
```

## Tratamento de erro

```rust
stream.play().map_err(|e| AudioError::StreamError(e.to_string()))?;

// No callback
move |err| {
    match err {
        cpal::StreamError::DeviceNotAvailable => {
            // Device desconectado
        }
        cpal::StreamError::BackendSpecific { err } => {
            // Erro específico do backend
        }
    }
}
```

## Links

- [[Referência Rust]]
- [[Arquitetura Geral]]
- [[Referência timestretch 0.15]]
