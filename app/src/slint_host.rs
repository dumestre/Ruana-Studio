//! Host offscreen do Slint (software renderer) para composição por cima do
//! wgpu do `timeline-render`: a UI é desenhada num buffer RGBA
//! PREMULTIPLICADO (formato nativo do software renderer) que o
//! `App::set_overlay` envia ao pass de overlay do wgpu (que usa blend
//! premultiplicado — `BlendState::PREMULTIPLIED_ALPHA_BLENDING`).
//!
//! Sequência de uso (uma vez por processo, ANTES de criar qualquer
//! componente):
//! ```no_run
//! let mut overlay = slint_host::SlintOverlay::new()?;
//! let ui = MyComponent::new()?; // cria a janela via este host
//! overlay.resize(width, height, scale_factor);
//! if let Some((rgba, w, h)) = overlay.draw() { app.set_overlay(&rgba, w, h); }
//! ```

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, SoftwareRenderer,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::PhysicalSize;

/// Implementação mínima de [`Platform`]: todas as janelas Slint desta
/// aplicação caem numa única janela offscreen (sem janela nativa).
struct OffscreenPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

/// Estado de um overlay Slint composto sobre o wgpu.
pub struct SlintOverlay {
    pub window: Rc<MinimalSoftwareWindow>,
    buffer: Vec<PremultipliedRgbaColor>,
    size: Cell<(u32, u32)>,
}

impl SlintOverlay {
    /// Instala a plataforma Slint offscreen e devolve o host. Deve rodar uma
    /// única vez por processo e antes de criar qualquer componente.
    pub fn new() -> Result<Self, slint::platform::SetPlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(OffscreenPlatform {
            window: window.clone(),
        }))?;
        window.window().request_redraw();
        Ok(Self {
            window,
            buffer: Vec::new(),
            size: Cell::new((0, 0)),
        })
    }

    /// Despacha um evento (posições em pixels LÓGICOS) para a UI Slint.
    pub fn dispatch(&self, event: slint::platform::WindowEvent) {
        self.window.window().dispatch_event(event);
    }

    pub fn request_redraw(&self) {
        self.window.window().request_redraw();
    }

    /// Ajusta o tamanho físico da janela de UI (px) e a escala de DPI. Chamar
    /// na criação e em todo resize/DPI change.
    pub fn resize(&self, width: u32, height: u32, scale_factor: f32) {
        self.size.set((width, height));
        self.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor,
            });
        self.window.set_size(PhysicalSize::new(width, height));
        self.window.window().request_redraw();
    }

    /// Avança timers/animações do Slint. Chamar no início de cada iteração do
    /// loop de eventos (antes de processar input/redesenhar).
    pub fn update_timers_and_animations() {
        slint::platform::update_timers_and_animations();
    }

    /// Renderiza a UI na CPU apenas se houver redraw pendente e devolve os
    /// pixels RGBA premultiplicados (linha 0 = topo) prontos para
    /// `App::set_overlay`. `None` se nada mudou neste frame.
    pub fn draw(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = self.size.get();
        if w == 0 || h == 0 {
            return None;
        }
        let pixels = (w as usize).saturating_mul(h as usize);
        if self.buffer.len() != pixels {
            self.buffer = vec![
                PremultipliedRgbaColor {
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 0,
                };
                pixels
            ];
        }
        let window = Rc::clone(&self.window);
        let buffer: &mut [PremultipliedRgbaColor] = &mut self.buffer;
        let stride = w as usize;
        let rendered = window.draw_if_needed(|renderer: &SoftwareRenderer| {
            renderer.render(buffer, stride);
        });
        rendered.then(|| (bytemuck::cast_slice(buffer).to_vec(), w, h))
    }
}