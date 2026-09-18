//! Host offscreen do Slint com renderer Skia/wgpu-30 desenhando DIRETO numa
//! textura wgpu da própria engine do `timeline-render`.
//!
//! Diferente da abordagem antiga (FemtoVG-GL numa janela oculta + readback
//! `glReadPixels` + premultiply em CPU + upload), aqui a UI e a timeline
//! compartilham a MESMA `Instance`/`Adapter`/`Device`/`Queue`. A cada `draw()`
//! o Skia rasteriza a cena na textura de overlay da engine (GPU→GPU); o pass
//! de composição do wgpu apenas amostra essa textura. Zero cópia de pixels.

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::skia_renderer::SkiaWGPU30Renderer;
use slint::platform::{Platform, PlatformError, Renderer, WindowAdapter};
use slint::PhysicalSize;

/// Janela Slint customizada: toda a UI do app cai nesta única janela virtual
/// (sem janela nativa). O renderer é o Skia sobre wgpu-30.
pub struct OverlayWindowAdapter {
    window: slint::Window,
    renderer: SkiaWGPU30Renderer,
    size: Cell<PhysicalSize>,
    scale_factor: Cell<f32>,
    /// Slint pediu um novo frame (dirty). Só re-renderiza quando `true`.
    redraw: Cell<bool>,
}

impl WindowAdapter for OverlayWindowAdapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn request_redraw(&self) {
        self.redraw.set(true);
    }

    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

/// Implementação de [`Platform`]: devolve sempre a única janela do overlay.
struct OverlayPlatform {
    adapter: Rc<OverlayWindowAdapter>,
}

impl Platform for OverlayPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.adapter.clone())
    }
}

/// Estado do overlay Slint composto sobre o wgpu.
pub struct SlintOverlay {
    pub window: Rc<OverlayWindowAdapter>,
}

impl SlintOverlay {
    /// Cria o renderer Skia/wgpu-30 sobre os recursos da engine, instala a
    /// plataforma Slint offscreen e devolve o host. Deve rodar UMA vez por
    /// processo. `instance`/`adapter`/`device`/`queue` precisam ser os mesmos
    /// usados pela engine que vai compor o overlay.
    pub fn new(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, PlatformError> {
        let renderer = SkiaWGPU30Renderer::new(instance, adapter, device, queue)
            .map_err(|e| format!("slint_host: SkiaWGPU30Renderer: {e}"))?;

        let adapter: Rc<OverlayWindowAdapter> = Rc::new_cyclic(|weak| OverlayWindowAdapter {
            window: slint::Window::new(weak.clone() as _),
            renderer,
            size: Cell::new(PhysicalSize::new(0, 0)),
            scale_factor: Cell::new(1.0),
            redraw: Cell::new(true),
        });

        match slint::platform::set_platform(Box::new(OverlayPlatform {
            adapter: adapter.clone(),
        })) {
            Ok(()) => {}
            Err(_) => {
                return Err(PlatformError::Other(
                    "slint_host: plataforma Slint já instalada".to_string(),
                ));
            }
        }

        Ok(Self { window: adapter })
    }

    /// Despacha sem segurar o overlay/borrow do host (para reentrância:
    /// os callbacks da UI podem chamar de volta o bridge da engine).
    pub fn dispatch_to(window: &Rc<OverlayWindowAdapter>, event: slint::platform::WindowEvent) {
        window.window.dispatch_event(event);
    }

    pub fn request_redraw(&self) {
        self.window.window.request_redraw();
    }

    /// Tamanho físico (px) atual da UI.
    pub fn size(&self) -> (u32, u32) {
        let s = self.window.size.get();
        (s.width, s.height)
    }

    /// Ajusta o tamanho físico da janela de UI (px) e a escala de DPI. Chamar
    /// na criação e em todo resize/DPI change.
    pub fn resize(&self, width: u32, height: u32, scale_factor: f32) {
        self.window.size.set(PhysicalSize::new(width, height));
        self.window.scale_factor.set(scale_factor);
        self.window.redraw.set(true);
        self.window
            .window
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor });
        self.window
            .window
            .dispatch_event(slint::platform::WindowEvent::Resized {
                size: slint::LogicalSize::new(
                    width as f32 / scale_factor,
                    height as f32 / scale_factor,
                ),
            });
        self.window.window.request_redraw();
    }

    /// Avança timers/animações do Slint. Chamar no início de cada iteração do
    /// loop de eventos (antes de processar input/redesenhar).
    pub fn update_timers_and_animations() {
        slint::platform::update_timers_and_animations();
    }

    /// Desenha a UI na textura de overlay da engine. Só re-renderiza quando o
    /// Slint pediu (redraw pendente); caso contrário a textura mantém o último
    /// frame e a composição segue de graça. `false` = falhou (fica sujo para
    /// tentar de novo no próximo frame).
    pub fn draw(&self, texture: &wgpu::Texture) -> bool {
        if !self.window.redraw.replace(false) {
            return true;
        }
        match self.window.renderer.render_to_texture(texture) {
            Ok(()) => true,
            Err(e) => {
                self.window.redraw.set(true);
                eprintln!("[overlay] render_to_texture falhou: {e}");
                false
            }
        }
    }
}
