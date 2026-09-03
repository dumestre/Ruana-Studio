//! GPU Context creation — cria contexto OpenGL para waveform GPU overlay.
//!
//! Estratégia por plataforma:
//!   - **Windows**: WGL sobre o DC da janela Slint (com fallback de janela
//!     helper message-only invisível quando o PFD da Slint é incompatível).
//!   - **Linux**: EGL com pbuffer offscreen (contexto renderizável, sem
//!     requerer janela). O overlay não compõe no framebuffer da Slint, mas o
//!     pipeline GL roda e `present_frame()` faz o swap.
//!
//! O contexto criado neste módulo é exposto como `Arc<glow::Context>` e o
//! swap (apresentação) é feito por `present_frame()`, chamado pelo renderer
//! no fim de cada frame.

use glow::Context;
use raw_window_handle::RawWindowHandle;
use std::ffi::c_void;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Swap target global — o renderer chama `present_frame()` após desenhar.
// ---------------------------------------------------------------------------

#[cfg(windows)]
static SWAP_HWND: std::sync::OnceLock<std::sync::Mutex<*mut c_void>> = std::sync::OnceLock::new();

#[cfg(target_os = "linux")]
use crate::waveform_gpu::linux_egl;

#[cfg(target_os = "linux")]
static SWAP_EGL: std::sync::OnceLock<std::sync::Mutex<Option<linux_swap::SwapTarget>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "linux")]
mod linux_swap {
    use crate::waveform_gpu::linux_egl::imp::*;

    /// Guarda display + surface para `eglSwapBuffers` no fim do frame.
    #[derive(Clone, Copy)]
    pub struct SwapTarget {
        pub display: EGLDisplay,
        pub surface: EGLSurface,
    }

    // EGL display/surface são handles opacos válidos entre threads
    // (todas as chamadas GL/EGL acontecem sob o bloqueio do renderer).
    unsafe impl Send for SwapTarget {}
    unsafe impl Sync for SwapTarget {}

    pub fn set(target: SwapTarget) {
        let _ = super::SWAP_EGL.get_or_init(Default::default).lock().map(|mut g| *g = Some(target));
    }

    pub fn present() {
        if let Some(swap) = super::SWAP_EGL.get() {
            if let Some(g) = swap.lock().ok() {
                if let Some(t) = *g {
                    unsafe { egl_swap_buffers(t.display, t.surface) };
                }
            }
        }
    }
}

/// Apresenta o frame desenhado (swap buffers na plataforma atual).
pub fn present_frame() {
    #[cfg(windows)]
    {
        if let Some(hwnd) = SWAP_HWND.get() {
            if let Ok(hwc) = hwnd.lock() {
                let h = *hwc;
                if !h.is_null() {
                    unsafe {
                        use crate::waveform_gpu::win32_gdi::{get_dc, wgl_swap_layer_buffers, WGL_SWAP_MAIN_PLANE};
                        let dc = get_dc(h);
                        wgl_swap_layer_buffers(dc, WGL_SWAP_MAIN_PLANE);
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        linux_swap::present();
    }
}

/// Cria contexto OpenGL para o renderer de waveform.
///
/// Em todas as plataformas já deixa o contexto *current* para que o
/// renderer possa emitir chamadas GL imediatamente. O valor de retorno é o
/// `glow::Context` pronto para uso.
#[cfg(windows)]
pub fn create_shared_gl_context(
    slint_window_handle: RawWindowHandle,
) -> Result<Arc<Context>, String> {
    unsafe {
        let slint_hwnd = match slint_window_handle {
            RawWindowHandle::Win32(h) => h.hwnd.get() as *mut c_void,
            _ => return Err("Slint window handle not Win32".into()),
        };

        // Guarda o HWND para o swap de apresentação posterior.
        let _ = SWAP_HWND.get_or_init(|| std::sync::Mutex::new(slint_hwnd));
        if let Ok(mut g) = SWAP_HWND.get().unwrap().lock() {
            *g = slint_hwnd;
        }

        // ============================================================
        // PASSO 1 — Caminho rápido: contexto direto no DC da janela Slint
        // ============================================================
        if let Ok(gl) = try_context_on_slint_dc(slint_hwnd) {
            eprintln!("[waveform_gpu] Context created directly on Slint DC");
            return Ok(gl);
        }

        eprintln!("[waveform_gpu] Step 1 failed (Slint DC PFD incompatible) — trying helper window");

        // ============================================================
        // PASSO 2 — Janela helper invisível (message-only) com OWNDC
        // ============================================================
        match try_context_on_helper_window() {
            Ok(gl) => {
                eprintln!("[waveform_gpu] Context created on helper message-only window");
                Ok(gl)
            }
            Err(e) => {
                eprintln!("[waveform_gpu] Step 2 failed: {}", e);
                Err(format!("All GL context strategies failed (Slint DC + helper window). Last: {}", e))
            }
        }
    }
}

/// Cria contexto OpenGL via EGL (Linux).
#[cfg(target_os = "linux")]
pub fn create_shared_gl_context(
    _slint_window_handle: RawWindowHandle,
) -> Result<Arc<Context>, String> {
    use crate::waveform_gpu::linux_egl::imp;
    unsafe {
        let display = imp::egl_get_display();
        if display.is_null() {
            return Err("eglGetDisplay returned null".into());
        }
        if !imp::egl_initialize(display) {
            return Err(format!("eglInitialize failed, EGL error=0x{:x}", imp::egl_get_error()));
        }
        let config = imp::egl_choose_pbuffer_config(display)
            .ok_or_else(|| format!("eglChooseConfig (pbuffer) failed, EGL error=0x{:x}", imp::egl_get_error()))?;

        // Superfície pbuffer (1x1 é suficiente: renderizamos offscreen).
        let surface = imp::egl_create_pbuffer_surface(display, config, 1, 1);
        if surface.is_null() {
            return Err(format!("eglCreatePbufferSurface failed, EGL error=0x{:x}", imp::egl_get_error()));
        }

        let context = imp::egl_create_context(display, config);
        if context.is_null() {
            imp::egl_destroy_surface(display, surface);
            return Err(format!("eglCreateContext failed, EGL error=0x{:x}", imp::egl_get_error()));
        }

        if !imp::egl_make_current(display, surface, context) {
            imp::egl_destroy_context(display, context);
            imp::egl_destroy_surface(display, surface);
            return Err(format!("eglMakeCurrent failed, EGL error=0x{:x}", imp::egl_get_error()));
        }

        // Registra o alvo de swap para `present_frame`.
        linux_swap::set(linux_swap::SwapTarget { display, surface });

        let gl = make_glow_context();
        eprintln!("[waveform_gpu] EGL context created (pbuffer)");
        Ok(Arc::new(gl))
    }
}

/// Stub para outras plataformas (não Windows/Linux).
#[cfg(not(any(windows, target_os = "linux")))]
pub fn create_shared_gl_context(
    _slint_window_handle: RawWindowHandle,
) -> Result<Arc<Context>, String> {
    Err("GL context sharing not implemented on this platform".into())
}

// ---------------------------------------------------------------------------
// Windows: helpers WGL
// ---------------------------------------------------------------------------

#[cfg(windows)]
unsafe fn try_context_on_slint_dc(
    slint_hwnd: *mut c_void,
) -> Result<Arc<Context>, String> {
    use crate::waveform_gpu::win32_gdi::*;
    unsafe {
        let slint_dc = get_dc(slint_hwnd);
        if slint_dc.is_null() {
            return Err("GetDC(Slint) returned null".into());
        }

        let hglrc = wgl_create_context(slint_dc);
        if hglrc.is_null() {
            let err = get_last_error();
            release_dc(slint_hwnd, slint_dc);
            return Err(format!("wglCreateContext(Slint) GetLastError={}", err));
        }

        if wgl_make_current(slint_dc, hglrc) == 0 {
            let err = get_last_error();
            release_dc(slint_hwnd, slint_dc);
            wgl_delete_context(hglrc);
            return Err(format!("wglMakeCurrent(Slint) GetLastError={}", err));
        }

        let gl = make_glow_context();
        Ok(Arc::new(gl))
    }
}

#[cfg(windows)]
static HELPED_CLASS_ATOM: std::sync::OnceLock<atom_holder::Atom> = std::sync::OnceLock::new();

#[cfg(windows)]
mod atom_holder {
    pub struct Atom(pub(crate) crate::waveform_gpu::win32_gdi::ATOM);
}

#[cfg(windows)]
unsafe fn try_context_on_helper_window() -> Result<Arc<Context>, String> {
    use crate::waveform_gpu::win32_gdi::*;
    unsafe {
        // --- Obter hInstance do módulo atual ---
        let h_instance = get_module_handle_w(std::ptr::null());
        if h_instance.is_null() {
            return Err(format!("GetModuleHandleW(null) GetLastError={}", get_last_error()));
        }

        // --- Registrar classe L"RuanaGLHelper" com CS_OWNDC ---
        static CLASS_NAME: &[u16] = &[
            b'R' as u16, b'u' as u16, b'a' as u16, b'n' as u16, b'a' as u16,
            b'G' as u16, b'L' as u16, b'H' as u16, b'e' as u16, b'l' as u16,
            b'p' as u16, b'e' as u16, b'r' as u16, 0,
        ];

        let atom = *HELPED_CLASS_ATOM.get_or_init(|| {
            let wnd_class = WNDCLASSEXW {
                cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_OWNDC,
                lpfn_wnd_proc: Some(helper_wnd_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance,
                h_icon: std::ptr::null_mut(),
                h_cursor: std::ptr::null_mut(),
                hbr_background: std::ptr::null_mut(),
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: CLASS_NAME.as_ptr(),
                h_icon_sm: std::ptr::null_mut(),
            };
            atom_holder::Atom(register_class_ex_w(&wnd_class))
        });

        if atom.0 == 0 {
            // ERROR_CLASS_ALREADY_EXISTS (1410) é OK — já registrada.
            let err = get_last_error();
            if err != 1410 {
                return Err(format!("RegisterClassExW GetLastError={}", err));
            }
        }

        // --- Criar janela message-only invisível 1x1 ---
        let helper_hwnd = create_window_ex_w(
            0,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            h_instance,
            std::ptr::null_mut(),
        );
        if helper_hwnd.is_null() {
            return Err(format!("CreateWindowExW(helper) GetLastError={}", get_last_error()));
        }

        // --- DC da helper window ---
        let helper_dc = get_dc(helper_hwnd);
        if helper_dc.is_null() {
            let err = get_last_error();
            destroy_window(helper_hwnd);
            return Err(format!("GetDC(helper) GetLastError={}", err));
        }

        // --- Configurar PIXELFORMATDESCRIPTOR ---
        let mut pfd: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
        pfd.n_size = std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16;
        pfd.n_version = 1;
        pfd.dw_flags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
        pfd.i_pixel_type = PFD_TYPE_RGBA;
        pfd.c_color_bits = 32;
        pfd.c_alpha_bits = 8;
        pfd.c_depth_bits = 24;
        pfd.c_stencil_bits = 8;
        pfd.i_layer_type = PFD_MAIN_PLANE;

        let pf = choose_pixel_format(helper_dc, &pfd);
        if pf == 0 {
            let err = get_last_error();
            release_dc(helper_hwnd, helper_dc);
            destroy_window(helper_hwnd);
            return Err(format!("ChoosePixelFormat(helper) GetLastError={}", err));
        }
        if set_pixel_format(helper_dc, pf, &pfd) == 0 {
            let err = get_last_error();
            release_dc(helper_hwnd, helper_dc);
            destroy_window(helper_hwnd);
            return Err(format!("SetPixelFormat(helper) GetLastError={}", err));
        }

        // --- Contexto GL no DC da helper ---
        let hglrc = wgl_create_context(helper_dc);
        if hglrc.is_null() {
            let err = get_last_error();
            release_dc(helper_hwnd, helper_dc);
            destroy_window(helper_hwnd);
            return Err(format!("wglCreateContext(helper) GetLastError={}", err));
        }
        if wgl_make_current(helper_dc, hglrc) == 0 {
            let err = get_last_error();
            wgl_delete_context(hglrc);
            release_dc(helper_hwnd, helper_dc);
            destroy_window(helper_hwnd);
            return Err(format!("wglMakeCurrent(helper) GetLastError={}", err));
        }

        Ok(Arc::new(make_glow_context()))
    }
}

/// WNDPROC padrão para a janela helper — só despacha para DefWindowProcW.
#[cfg(windows)]
unsafe extern "system" fn helper_wnd_proc(
    hwnd: crate::waveform_gpu::win32_gdi::HWND,
    msg: u32,
    wparam: crate::waveform_gpu::win32_gdi::WPARAM,
    lparam: crate::waveform_gpu::win32_gdi::LPARAM,
) -> crate::waveform_gpu::win32_gdi::LRESULT {
    unsafe { crate::waveform_gpu::win32_gdi::def_window_proc_w(hwnd, msg, wparam, lparam) }
}

// ---------------------------------------------------------------------------
// glow::Context loader (cross-platform via EGL no Linux, WGL no Windows)
// ---------------------------------------------------------------------------

#[cfg(windows)]
unsafe fn make_glow_context() -> Context {
    use crate::waveform_gpu::win32_gdi::*;
    unsafe {
        // Pré-carrega opengl32.dll para funções core 1.1
        static OGL32_ONCE: std::sync::OnceLock<Option<&'static libloading::Library>> = std::sync::OnceLock::new();
        let ogl32 = OGL32_ONCE.get_or_init(|| {
            let lib = libloading::Library::new("opengl32.dll").ok()?;
            Some(&*Box::leak(Box::new(lib)))
        });

        Context::from_loader_function(|s| {
            let Ok(cstr) = std::ffi::CString::new(s) else {
                return std::ptr::null();
            };
            let proc = wgl_get_proc_address(cstr.as_ptr() as *const u8);
            if !proc.is_null() {
                return proc as *const c_void;
            }
            if let Some(lib) = ogl32 {
                if let Ok(sym) = lib.get::<unsafe extern "C" fn()>(cstr.as_bytes_with_nul()) {
                    return *sym as *const c_void;
                }
            }
            std::ptr::null()
        })
    }
}

#[cfg(target_os = "linux")]
unsafe fn make_glow_context() -> Context {
    unsafe {
        Context::from_loader_function(|s| {
            let Ok(cstr) = std::ffi::CString::new(s) else {
                return std::ptr::null();
            };
            linux_egl::imp::egl_get_proc_address(cstr.as_ptr() as *const u8) as *const c_void
        })
    }
}