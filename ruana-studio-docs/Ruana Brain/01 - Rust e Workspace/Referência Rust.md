# Referência Rust

> Referência rápida do Rust usado no RuanaStudio (edition 2024).

---

## Fontes oficiais

- **The Rust Book**: https://doc.rust-lang.org/stable/book/
- **The Rust Reference**: https://doc.rust-lang.org/stable/reference/
- **Standard Library**: https://doc.rust-lang.org/stable/std/
- **Rust by Example**: https://doc.rust-lang.org/stable/rust-by-example/
- **Cargo Book**: https://doc.rust-lang.org/cargo/
- **Edition Guide (2024)**: https://doc.rust-lang.org/edition-guide/

## Edition 2024 — pontos relevantes

O workspace usa `edition = "2024"`. Mudanças que afetam o código:

- **`gen` keyword reservado** — blocos `gen` agora são `gen {}` (experimental)
- **`unsafe` em traits** — `unsafe trait` e `unsafe fn` em traits agora explícitos
- **Lifetime capture** — closures capturam lifetimes por padrão (não mais `move` implícito)
- **`impl Trait` em return position** — captura lifetimes do escopo
- **Macro `unsafe_attr`** — atributos `unsafe` em itens

## Padrões do projeto

### Ownership e borrowing

```rust
// Preferir referências sobre ownership quando possível
fn process_clip(clip: &Clip) -> f32 { /* ... */ }

// Clonar apenas quando necessário
let clip_copy = clip.clone();

// Rc/RefCell para estado compartilhado UI <-> engine
let state: Rc<RefCell<PlayheadState>> = Rc::new(RefCell::new(PlayheadState::default()));
```

### Concronência

```rust
// Canais para comunicação entre threads
let (tx, rx) = std::sync::mpsc::channel::<AudioCommand>();

// Mutex para estado compartilhado
let shared: Arc<Mutex<Project>> = Arc::new(Mutex::new(project));

// Atomic para flags simples
let playing: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
```

### Error handling

```rust
// Result com thiserror ou enum customizado
#[derive(Debug)]
pub enum AudioError {
    DeviceNotFound,
    StreamError(String),
    BufferUnderrun,
}

pub type AudioResult<T> = Result<T, AudioError>;

// ? operator para propagação
fn setup_stream() -> AudioResult<Stream> {
    let device = default_device().ok_or(AudioError::DeviceNotFound)?;
    // ...
}
```

### Traits e generics

```rust
pub trait AudioSource {
    fn next_frame(&mut self) -> f32;
    fn sample_rate(&self) -> u32;
}

// Generic sobre trait
pub fn mix<S: AudioSource>(sources: &mut [S], output: &mut [f32]) {
    // ...
}
```

## Crates externos principais

| Crate | Uso no projeto | Versão |
|---|---|---|
| `wgpu` | Renderização GPU | 30 |
| `winit` | Janelas e eventos | 0.30 |
| `slint` | UI declarativa | 1.18 |
| `cpal` | I/O de áudio | 0.18 |
| `symphonia` | Decodificação | 0.6 |
| `timestretch` | Time-stretch | 0.15 |
| `hound` | WAV I/O | 3.5 |
| `bytemuck` | Cast de tipos para GPU | 1.14 |
| `pollster` | Block on async | 0.4/1.0 |
| `raw-window-handle` | Handle de janela p/ GPU | 0.6 |
| `ab_glyph` | Fonte para texto GPU | 0.2 |
| `libloading` | Carregamento dinâmico de plugins | 0.9 |
| `serde` + `serde_json` | Serialização | 1 |
| `sha2` | Hash de plugins | 0.10 |
| `zip` | Pacotes de plugins | 2.4 |
| `reqwest` | HTTP (ruana-hub) | 0.12 |

## Links

- [[Workspace Rust e Crates]]
- [[Referência wgpu 30]]
- [[Referência Slint 1.18]]
