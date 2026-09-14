//! Prova de conceito da arquitetura do AGENTS.md:
//!
//!   1. O app é dono do loop winit e do dispositivo wgpu (via
//!      `timeline_render::App`), renderizando a timeline direto na surface.
//!   2. A UI Slint é renderizada por software (offscreen) e composta por cima
//!      da cena wgpu como um overlay RGBA (pass de overlay no `App::render`).
//!
//! Rodar: `cargo run --bin overlay-demo`
//!
//! Controles:
//!   - Espaço:            play/pause do playhead (timeline, via wgpu)
//!   - Clique na UI:      toggle play (o botão vive NO overlay Slint)
//!   - Demais cliques:    vão para a timeline (arrastar clip, etc.)
//!   - ESC:               fecha

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use slint::SharedString;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

#[path = "../slint_host.rs"]
mod slint_host;

use slint_host::SlintOverlay;
use timeline_render::app::App;

slint::slint! {
    import { Button } from "std-widgets.slint";

    export component DemoUi inherits Window {
        background: transparent;
        in-out property <string> status: "paused";
        callback toggled;
        Rectangle {
            height: 56px;
            width: 100%;
            background: #17181cd8;
            HorizontalLayout {
                spacing: 10px;
                padding: 10px;
                Button {
                    text: "play / pause";
                    clicked => { root.toggled(); }
                }
                Text {
                    text: "overlay Slint :: " + root.status;
                    color: #e8e8e8;
                }
            }
        }
    }
}

struct Handler {
    app: Option<App>,
    overlay: Option<SlintOverlay>,
    ui: Option<DemoUi>,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    play_pending: Rc<Cell<bool>>,
    scale_factor: f64,
    last_frame: Instant,
}

impl Handler {
    fn new() -> Self {
        Self {
            app: None,
            overlay: None,
            ui: None,
            window: None,
            window_id: None,
            play_pending: Rc::new(Cell::new(false)),
            scale_factor: 1.0,
            last_frame: Instant::now(),
        }
    }
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Ruana Studio — wgpu + Slint overlay")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = event_loop
            .create_window(attrs)
            .expect("failed to create window");
        let window_id = window.id();
        let inner = window.inner_size();
        let win: Arc<Window> = Arc::new(window);
        self.scale_factor = win.scale_factor();

        // 1. Dispositivo/surface wgpu (donos do loop).
        let app = App::new(Some(win.clone()), (inner.width, inner.height), None)
            .expect("timeline-render init failed");
        app.init_tools();
        self.app = Some(app);

        // 2. Host offscreen do Slint + componente. A ordem importa: a
        //    plataforma (que fornece a janela offscreen) antes do componente.
        let overlay = SlintOverlay::new().expect("slint platform (já setada?)");
        overlay.resize(inner.width, inner.height, self.scale_factor as f32);
        let ui = DemoUi::new().expect("component init failed");

        // Botão da UI: marca um toggle para o handler aplicar no playhead.
        let play_pending = Rc::clone(&self.play_pending);
        let weak = ui.as_weak();
        ui.on_toggled(move || {
            let was = play_pending.get();
            play_pending.set(!was);
            if let Some(ui) = weak.upgrade() {
                ui.set_status(if was { "paused" } else { "playing" }.into());
                // O overlay precisa re-renderizar: o Slint marca redraw
                // sozinho, mas forçamos para o próximo frame sempre pegar.
                ui.window().request_redraw();
            }
        });

        self.overlay = Some(overlay);
        self.ui = Some(ui);
        self.window = Some(win);
        self.window_id = Some(window_id);
        self.last_frame = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }
        let app = self.app.as_mut().expect("app not created");
        let scale = self.scale_factor as f32;

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                app.resize((size.width, size.height));
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.resize(size.width, size.height, scale);
                }
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: sf, ..
            } => {
                self.scale_factor = sf;
                if let (Some(win), Some(overlay)) = (&self.window, self.overlay.as_ref()) {
                    let size = win.inner_size();
                    overlay.resize(size.width, size.height, sf as f32);
                }
            }
            WindowEvent::RedrawRequested => {
                // Avança o transporte em modo play para o playhead andar.
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f64();
                self.last_frame = now;
                if app.playing {
                    if let Ok(mut t) = app.transport.lock() {
                        let ppq = t.ppq as f64;
                        let bpm = app.bpm as f64;
                        t.advance((dt * ppq * bpm / 60.0) as u64);
                    }
                }

                // Botão da UI pediu toggle do play.
                if self.play_pending.get() {
                    self.play_pending.set(false);
                    app.last_play_head_update = Instant::now();
                    app.playing = !app.playing;
                    if let Ok(mut t) = app.transport.lock() {
                        t.playing = app.playing;
                    }
                }

                // A UI itera sozinha (animations/timers do Slint).
                SlintOverlay::update_timers_and_animations();
                app.update_playhead();

                // Compõe o overlay (se o Slint redesenhou algo) e apresenta.
                if let Some(overlay) = self.overlay.as_mut() {
                    if let Some((rgba, w, h)) = overlay.draw() {
                        app.set_overlay(&rgba, w, h);
                    }
                }
                app.render();
            }
            WindowEvent::CursorMoved { position, .. } => {
                app.handle_cursor_moved((position.x as f32, position.y as f32));
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.dispatch(slint::platform::WindowEvent::PointerMoved {
                        position: logical_pos(position.x, position.y, scale),
                    });
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.dispatch(slint::platform::WindowEvent::PointerExited);
                }
            }
            WindowEvent::MouseInput {
                state, button, ..
            } => {
                let pos = app.last_cursor;
                app.handle_mouse_button(state.into(), Some(pos));
                let slint_button = match button {
                    winit::event::MouseButton::Left => {
                        slint::platform::PointerEventButton::Left
                    }
                    winit::event::MouseButton::Right => {
                        slint::platform::PointerEventButton::Right
                    }
                    _ => slint::platform::PointerEventButton::Middle,
                };
                let event = match state {
                    ElementState::Pressed => slint::platform::WindowEvent::PointerPressed {
                        position: logical_pos(pos.0 as f64, pos.1 as f64, scale),
                        button: slint_button,
                    },
                    ElementState::Released => slint::platform::WindowEvent::PointerReleased {
                        position: logical_pos(pos.0 as f64, pos.1 as f64, scale),
                        button: slint_button,
                    },
                };
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.dispatch(event);
                }
                app.request_redraw();
            }
            WindowEvent::MouseWheel {
                delta, phase: _, ..
            } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                app.handle_scroll(dx, dy, app.ctrl_pressed, app.alt_pressed);
                let pos = app.last_cursor;
                if let Some(overlay) = self.overlay.as_ref() {
                    overlay.dispatch(slint::platform::WindowEvent::PointerScrolled {
                        position: logical_pos(pos.0 as f64, pos.1 as f64, scale),
                        delta_x: dx / scale,
                        delta_y: dy / scale,
                    });
                }
                app.request_redraw();
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                if let Some(overlay) = self.overlay.as_ref() {
                    forward_key(overlay, &key);
                }
                if key.state == ElementState::Pressed {
                    if key.logical_key == Key::Named(NamedKey::Space) {
                        app.playing = !app.playing;
                        if let Ok(mut t) = app.transport.lock() {
                            t.playing = app.playing;
                        }
                        app.last_play_head_update = Instant::now();
                        app.request_redraw();
                    } else if key.logical_key == Key::Named(NamedKey::Escape) {
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Loop contínuo de redraw (playhead, animações da UI, etc.).
        if let Some(app) = &mut self.app {
            if app.playing || (app.clip_interaction.is_dragging()) {
                app.request_redraw();
            }
        }
        if let Some(overlay) = &self.overlay {
            if self.ui.is_some() {
                overlay.request_redraw();
            }
        }
    }
}

// Dispatch de teclado para o Slint: mapeia `logical_key` do winit para o
// texto de `WindowEvent::KeyPressed/Released` do Slint.
fn forward_key(overlay: &SlintOverlay, event: &KeyEvent) {
    let text: Option<SharedString> = match &event.logical_key {
        Key::Character(s) => Some(SharedString::from(s.as_str())),
        Key::Named(named) => named_key_text(*named),
        Key::Unidentified(_) | Key::Dead(_) => None,
    };
    let Some(text) = text else { return };
    let ev = match event.state {
        ElementState::Pressed => slint::platform::WindowEvent::KeyPressed { text },
        ElementState::Released => slint::platform::WindowEvent::KeyReleased { text },
    };
    overlay.dispatch(ev);
}

/// Converte posição em px físicos para px lógicos do Slint.
fn logical_pos(x: f64, y: f64, scale_factor: f32) -> slint::LogicalPosition {
    slint::LogicalPosition::new(x as f32 / scale_factor, y as f32 / scale_factor)
}

/// Mapeia teclas nomeadas do winit para os nomes que o Slint entende.
fn named_key_text(key: NamedKey) -> Option<SharedString> {
    let name: Option<&'static str> = match key {
        NamedKey::Enter => Some("Return"),
        NamedKey::Tab => Some("Tab"),
        NamedKey::Space => Some(" "),
        NamedKey::Backspace => Some("Backspace"),
        NamedKey::Delete => Some("Delete"),
        NamedKey::Escape => Some("Escape"),
        NamedKey::ArrowUp => Some("Up"),
        NamedKey::ArrowDown => Some("Down"),
        NamedKey::ArrowLeft => Some("Left"),
        NamedKey::ArrowRight => Some("Right"),
        NamedKey::Home => Some("Home"),
        NamedKey::End => Some("End"),
        NamedKey::PageUp => Some("PageUp"),
        NamedKey::PageDown => Some("PageDown"),
        NamedKey::Shift => Some("Shift"),
        NamedKey::Control => Some("Control"),
        NamedKey::Alt => Some("Alt"),
        NamedKey::Meta => Some("Meta"),
        NamedKey::CapsLock => Some("CapsLock"),
        _ => None,
    };
    name.map(Into::into)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut handler = Handler::new();
    event_loop.run_app(&mut handler)?;
    Ok(())
}