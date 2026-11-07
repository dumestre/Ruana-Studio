# Referência symphonia 0.6

> Referência do symphonia usado no RuanaStudio (0.6).

---

## Fontes oficiais

- **symphonia docs**: https://docs.rs/symphonia/
- **symphonia GitHub**: https://github.com/pdeljanov/Symphonia

## Visão geral

Symphonia é uma biblioteca de decodificação de áudio pura-Rust, sem dependências de sistema. Suporta múltiplos formatos e codecs.

## Formatos suportados (features do projeto)

```toml
symphonia = { version = "0.6", features = ["wav", "mp3", "flac", "ogg", "pcm"] }
```

| Feature | Formatos/Codecs |
|---|---|
| `wav` | WAV (PCM, IEEE float) |
| `mp3` | MP3 (MPEG-1/2/2.5 Layer III) |
| `flac` | FLAC |
| `ogg` | Ogg (Vorbis, Opus, FLAC) |
| `pcm` | PCM raw |

## API principal

```rust
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// Abrir arquivo
let file = std::fs::File::open("track.wav").unwrap();
let mss = MediaSourceStream::new(Box::new(file), Default::default());

// Hint para formato (opcional, acelera detecção)
let mut hint = Hint::new();
hint.with_extension("wav");

// Probe
let probed = symphonia::default::get_probe()
    .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
    .unwrap();

let mut format = probed.format;

// Decoder
let track = format.default_track().unwrap();
let mut decoder = symphonia::default::get_codecs()
    .make(&track.codec_params, &DecoderOptions::default())
    .unwrap();

// Decodificar
let mut sample_buf = SampleBuffer::<f32>::new(
    track.codec_params.n_frames.unwrap_or(0) as u64,
    track.codec_params.channels.unwrap().into(),
);

loop {
    let packet = match format.next_packet() {
        Ok(packet) => packet,
        Err(_) => break,  // EOF
    };
    
    match decoder.decode(&packet) {
        Ok(decoded) => {
            sample_buf.copy_interleaved_ref(decoded);
            // sample_buf.samples() -> &[f32] interleaved
        }
        Err(_) => continue,
    }
}
```

## Integração com o projeto

`daw-media` usa symphonia para decodificar arquivos de áudio para PCM `f32`, que são então:
1. Armazenados em `AudioBuffer` (memmap para arquivos grandes)
2. Passados por `timestretch` para varispeed/keylock
3. Misturados no callback de áudio do `daw-audio`

## Padrões

### Decodificação para buffer

```rust
pub fn decode_file(path: &Path) -> AudioResult<AudioBuffer> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    
    let probed = get_probe().format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or(AudioError::NoTrack)?;
    
    let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    
    let mut samples: Vec<f32> = Vec::new();
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if let Ok(decoded) = decoder.decode(&packet) {
            let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, decoded.spec().channels);
            buf.copy_interleaved_ref(decoded);
            samples.extend_from_slice(buf.samples());
        }
    }
    
    Ok(AudioBuffer::new(samples, sample_rate, channels))
}
```

## Links

- [[Referência Rust]]
- [[Referência cpal 0.18]]
- [[Referência timestretch 0.15]]
- [[Arquitetura Geral]]
