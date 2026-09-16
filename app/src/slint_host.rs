//! Host offscreen do Slint com renderer FemtoVG-GL para composição por cima
//! do wgpu do `timeline-render`.
//!
//! A UI é desenhada num contexto OpenGL numa JANELA WINIT OCULTA criada no
//! MESMO `EventLoop` do app (no Windows, um segundo `EventLoop` não pode ser
//! criado). A cada `draw()`:
//!  1. `FemtoVGRenderer::render()` desenha a cena no backbuffer GL;
//!  2. `Window::take_snapshot()` lê de volta via screenshot do FemtoVG;
//!  3. os pixels saem como RGBA PREMULTIPLICADO (o formato nativo do pass de
//!     overlay do wgpu — `PREMULTIPLIED_ALPHA_BLENDING`), prontos para
//!     `App::set_overlay`.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::rc::Rc;

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::{Display as GlutinDisplay, DisplayApiPreference};
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use i_slint_renderer_femtovg::opengl::OpenGLInterface;
use i_slint_renderer_femtovg::FemtoVGOpenGLRenderer;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use slint::platform::{Platform, PlatformError, Renderer, WindowAdapter};
use slint::{PhysicalSize, SharedPixelBuffer};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window as WinitWindow};

/// Contexto GL da janela oculta, como `OpenGLInterface` do FemtoVG. Espelha o
/// `glcontext.rs` do `i-slint-backend-winit` (glutin 0.32).
struct OverlayGlContext {
    context: glutin::context::PossiblyCurrentContext,
    surface: glutin::surface::Surface<WindowSurface>,
    display: GlutinDisplay,
}

unsafe impl OpenGLInterface for OverlayGlContext {
    fn ensure_current(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.context.is_current() {
            self.context.make_current(&self.surface).map_err(|e| -> PlatformError {
                format!("FemtoVG: make_current falhou: {e}").into()
            })?;
        }
        Ok(())
    }

    fn swap_buffers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.surface.swap_buffers(&self.context).map_err(|e| -> PlatformError {
            format!("FemtoVG: swap_buffers falhou: {e}").into()
        })?;
        Ok(())
    }

    fn resize(
        &self,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_current()?;
        self.surface.resize(&self.context, width, height);
        Ok(())
    }

    fn get_proc_address(&self, name: &std::ffi::CStr) -> *const std::ffi::c_void {
        self.display.get_proc_address(name)
    }
}

/// Janela Slint customizada: toda a UI do app cai nesta única janela (sem
/// janela nativa). O renderer é o FemtoVG-GL.
pub struct OverlayWindowAdapter {
    window: slint::Window,
    renderer: FemtoVGOpenGLRenderer,
    size: Cell<PhysicalSize>,
    scale_factor: Cell<f32>,
    redraw: Cell<bool>,
    /// true quando o último quadrado enviado bate com `size`. Força
    /// re-render até que um snapshot case com o alvo (cobre a corrida de
    /// tamanho do backbuffer GL no startup/resize).
    valid: Cell<bool>,
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
    gl_window: Rc<WinitWindow>,
}

impl SlintOverlay {
    /// Cria a janela winit oculta + contexto GL, instala a plataforma Slint
    /// offscreen e devolve o host. Deve rodar UMA vez por processo e com o
    /// `EventLoop` do app já ativo (a janela precisa viver no mesmo loop).
    pub fn new(active_event_loop: &ActiveEventLoop) -> Result<Self, PlatformError> {
        let gl_window = active_event_loop
            .create_window(
                WinitWindow::default_attributes()
                    .with_title("ruana-ui")
                    .with_visible(false)
                    .with_inner_size(winit::dpi::LogicalSize::new(1.0, 1.0)),
            )
            .map_err(|e| format!("slint_host: winit window (overlay GL): {e}"))?;
        let gl_window = Rc::new(gl_window);

        let raw_display = gl_window
            .display_handle()
            .map_err(|e| format!("slint_host: display handle: {e}"))?
            .as_raw();
        let raw_window = gl_window
            .window_handle()
            .map_err(|e| format!("slint_host: window handle: {e}"))?
            .as_raw();

        let display = unsafe {
            GlutinDisplay::new(raw_display, DisplayApiPreference::WglThenEgl(Some(raw_window)))
                .map_err(|e| format!("slint_host: GlutinDisplay: {e}"))?
        };

        let config = unsafe {
            display
                .find_configs(ConfigTemplateBuilder::new().build())
                .map_err(|e| format!("slint_host: find_configs: {e}"))?
                .next()
                .ok_or_else(|| "slint_host: nenhuma config GL".to_string())?
        };

        let surface = unsafe {
            display
                .create_window_surface(
                    &config,
                    &SurfaceAttributesBuilder::<WindowSurface>::new().build(
                        raw_window,
                        NonZeroU32::new(1).unwrap(),
                        NonZeroU32::new(1).unwrap(),
                    ),
                )
                .map_err(|e| format!("slint_host: surface GL: {e}"))?
        };

        let not_current = unsafe {
            display
                .create_context(
                    &config,
                    &ContextAttributesBuilder::new()
                        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                        .build(Some(raw_window)),
                )
                .map_err(|e| format!("slint_host: contexto GL: {e}"))?
        };
        let context = not_current
            .make_current(&surface)
            .map_err(|e| format!("slint_host: make_current: {e}"))?;

        // A janela é OCULTA (nunca exibida → o DWM não compõe a HWND). Com o
        // swap interval default (vsync) o `SwapBuffers` pode travar por
        // segundos por frame, deixando a UI com ~2-3s de atraso. Como esse
        // buffer só existe para o `glReadPixels` da composição, swap NUNCA
        // deve esperar vblank/vsync aqui.
        let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);

        if display
            .get_proc_address(&std::ffi::CString::new("glCreateShader").unwrap())
            .is_null()
        {
            return Err("slint_host: driver GL sem glCreateShader".to_string().into());
        }

        let renderer = FemtoVGOpenGLRenderer::new(OverlayGlContext {
            context,
            surface,
            display,
        })
        .map_err(|e| format!("slint_host: FemtoVGRenderer: {e}"))?;

        let adapter: Rc<OverlayWindowAdapter> = Rc::new_cyclic(|weak| OverlayWindowAdapter {
            window: slint::Window::new(weak.clone() as _),
            renderer,
            size: Cell::new(PhysicalSize::new(0, 0)),
            scale_factor: Cell::new(1.0),
            redraw: Cell::new(true),
            valid: Cell::new(false),
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

        Ok(Self {
            window: adapter,
            gl_window,
        })
    }

    /// Despacha sem segurar o overlay/borrow do host (para reentrância:
    /// os callbacks da UI podem chamar de volta o bridge da engine).
    pub fn dispatch_to(window: &Rc<OverlayWindowAdapter>, event: slint::platform::WindowEvent) {
        window.window.dispatch_event(event);
    }

    pub fn request_redraw(&self) {
        self.window.window.request_redraw();
    }

    /// Ajusta o tamanho físico da janela de UI (px), a escala de DPI e o
    /// tamanho da janela GL oculta. Chamar na criação e em todo resize/DPI
    /// change.
    pub fn resize(&self, width: u32, height: u32, scale_factor: f32) {
        // A janela GL precisa acompanhar (o backbuffer é lido pixel a pixel).
        let _ = self
            .gl_window
            .request_inner_size(winit::dpi::PhysicalSize::new(width.max(1), height.max(1)));
        self.window.size.set(PhysicalSize::new(width, height));
        self.window.scale_factor.set(scale_factor);
        self.window.redraw.set(true);
        self.window.valid.set(false);
        self.window.window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
            scale_factor,
        });
        self.window.window.dispatch_event(slint::platform::WindowEvent::Resized {
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

    /// Renderiza a UI no GL e devolve os pixels RGBA PREMULTIPLICADOS
    /// (linha 0 = topo) prontos para `App::set_overlay`.
    ///
    /// Só re-renderiza quando o Slint pede (redraw pendente). Exceção: se o
    /// último frame validado não bate com `size` (startup/resize — o
    /// backbuffer GL esvazia em 1×1 e o snapshot volta pequeno), re-renderiza
    /// todo frame até bater; o snapshot miúdo é descartado, então nunca
    /// sobe textura do tamanho errado. `None` = nada a fazer/ainda não pronto.
    pub fn draw(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = {
            let s = self.window.size.get();
            (s.width, s.height)
        };
        if w == 0 || h == 0 {
            return None;
        }

        // A janela GL oculta começa em 1×1 e sobe de tamanho via
        // `request_inner_size` de forma assíncrona. Enquanto o backbuffer real
        // (`inner_size`) não bater com o alvo, renderizar só produz um readback
        // de buffer não inicializado (lixo/zeros) — com DIMENSÕES iguais às
        // alvo, então não dá para detectar pelo tamanho do snapshot. Por isso:
        // se `inner_size` ≠ alvo, descarta e tenta de novo no próximo frame.
        let actual = self.gl_window.inner_size();
        if actual.width != w || actual.height != h || actual.width == 0 || actual.height == 0 {
            self.window.valid.set(false);
            return None;
        }

        if !self.window.redraw.replace(false) && self.window.valid.get() {
            return None;
        }
        self.window.renderer.render().ok()?;
        let pixels: SharedPixelBuffer<slint::Rgba8Pixel> = self.window.window.take_snapshot().ok()?;
        let pw = pixels.width();
        let ph = pixels.height();
        if pw != w || ph != h {
            // Backbuffer ainda não acompanhou o alvo: descarta e tenta de novo.
            self.window.valid.set(false);
            return None;
        }
        // Guard de conteúdo: um readback de FB em transição (resize do SO)
        // volta transparente. A UI real tem fundo opaco — se quase tudo estiver
        // alpha=0, não é um frame legítimo: descarta e tenta de novo.
        if is_transparent_readback(pixels.as_bytes()) {
            self.window.valid.set(false);
            return None;
        }
        self.window.valid.set(true);
        Some((premultiply(pixels.as_bytes()), pw, ph))
    }
}

/// FemtoVG lê o framebuffer em RGBA "straight"; o blend do wgpu espera
/// pré-multiplicado (RGB já multiplicado por A). Converte em CPU — barato,
/// pois só roda quando a UI está suja.
fn premultiply(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        if a == 255 {
            out.extend_from_slice(px);
        } else if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let af = a as u32;
            out.extend_from_slice(&[
                (r as u32 * af / 255) as u8,
                (g as u32 * af / 255) as u8,
                (b as u32 * af / 255) as u8,
                a,
            ]);
        }
    }
    out
}

/// true se o readback não parece um frame renderizado de verdade: quase todos
/// os pixels alpha==0. Em modo hosted a janela tem fundo TRANSPARENTE (o
/// "buraco" da timeline é o wgpu por baixo), então um frame legítimo sempre
/// tem uma região transparente grande — o limiar precisa ser bem mais
/// permissivo que antes. (Readback de backbuffer ainda em transição de
/// tamanho costuma voltar zerado/transparente.)
fn is_transparent_readback(rgba: &[u8]) -> bool {
    let mut transparent = 0usize;
    for px in rgba.chunks_exact(4) {
        if px[3] == 0 {
            transparent += 1;
        }
    }
    transparent * 64 > rgba.len()
}