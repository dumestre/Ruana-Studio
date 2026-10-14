# Referência winit 0.30

> Referência do winit usado no RuanaStudio (0.30).

---

## Fontes oficiais

- **winit docs**: https://docs.rs/winit/
- **winit GitHub**: https://github.com/rust-windowing/winit
- **winit releases**: https://github.com/rust-windowing/winit/releases

## Visão geral

winit é uma biblioteca de criação e gerenciamento de janelas cross-platform. Pode criar janelas e lidar com eventos (resize, teclado, mouse, etc.).

## Plataformas suportadas

- Desktop: Windows, macOS, Unix (X11, Wayland), Redox
- Mobile: iOS, Android
- Web

## API principal

```rust
use winit::event::{Event, WindowEvent, ElementState, MouseButton, KeyEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

let event_loop = EventLoop::new().unwrap();

event_loop.run(move |event, elwt| {
    match event {
        Event::WindowEvent { event: WindowEvent::RedrawRequested, window_id } => {
            // Renderizar
        }
        Event::WindowEvent { event: WindowEvent::CloseRequested, window_id } => {
            elwt.exit();
        }
        Event::WindowEvent { event: WindowEvent::Resized(size), window_id } => {
            // Resize surface
        }
        Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent { .. }, .. }, .. } => {
            // Tecla
        }
        Event::WindowEvent { event: WindowEvent::MouseInput { state, button, .. }, .. } => {
            // Mouse
        }
        Event::AboutToWait => {
            // Atualizar estado, request redraw
        }
        _ => {}
    }
}).unwrap();
```

## Criação de janela

```rust
use winit::window::WindowAttributes;

let window = elwt.create_window(
    WindowAttributes::default()
        .with_title("Ruana Studio")
        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720))
        .with_min_inner_size(winit::dpi::LogicalSize::new(800, 600))
).unwrap();
```

## Eventos principais

| Evento | Descrição |
|---|---|
| `WindowEvent::Resized` | Janela redimensionada |
| `WindowEvent::Moved` | Janela movida |
| `WindowEvent::CloseRequested` | Usuário pediu para fechar |
| `WindowEvent::KeyboardInput` | Tecla pressionada/solta |
| `WindowEvent::MouseInput` | Botão do mouse pressionado/solto |
| `WindowEvent::CursorMoved` | Cursor movido |
| `WindowEvent::MouseWheel` | Rodinha do mouse |
| `WindowEvent::RedrawRequested` | Frame deve ser renderizado |
| `Event::AboutToWait` | Loop está prestes a esperar |

## Integração com wgpu

```rust
// Criar surface a partir da janela
let surface = instance.create_surface(&window).unwrap();

// Resize
Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
    config.width = size.width;
    config.height = size.height;
    surface.configure(&device, &config);
}
```

## Integração com Slint

```slint
// No loop de eventos
slint_window.process_event(event);
```

## DPI e HiDPI

```rust
let scale_factor = window.scale_factor();
let physical_size = window.inner_size();
let logical_size = physical_size.to_logical(scale_factor);
```

## Features

| Feature | Descrição |
|---|---|
| `rwh_06` | raw-window-handle 0.6 (para wgpu) |
| `x11` | Suporte X11 |
| `wayland` | Suporte Wayland |
| `serde` | Serialização |

## Links

- [[Referência Rust]]
- [[Referência wgpu 30]]
- [[Arquitetura Geral]]
