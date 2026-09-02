//! GPU Context creation — cria contexto OpenGL compartilhado com Slint.
//!
//! No Windows, usa FFI dinâmico via libloading para funções GDI/WGL
//! que faltam no windows-sys 0.61.

use crate::waveform_gpu::win32_gdi::*;
use glow::Context;
use raw_window_handle::RawWindowHandle;
use std::ffi::c_void;
use std::sync::Arc;

/// Cria contexto OpenGL compartilhado com a janela Slint.
/// Retorna (Arc<Context>, hwnd) onde hwnd é o HWND da janela Slint.
#[cfg(windows)]
pub fn create_shared_gl_context(
    slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut c_void), String> {
    use glow::Context;
    use raw_window_handle::RawWindowHandle;
    use std::ffi::CString;

    unsafe {
        // Obtém HWND da janela Slint
        let slint_hwnd = match slint_window_handle {
            RawWindowHandle::Win32(h) => h.hwnd.get() as *mut c_void,
            _ => return Err("Slint window handle not Win32".into()),
        };

        // Device context da janela Slint
        let slint_dc = get_dc(slint_hwnd);
        if slint_dc.is_null() {
            return Err("GetDC failed".into());
        }

        // Pixel format
        let mut pfd = PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            cAlphaBits: 8,
            cDepthBits: 24,
            cStencilBits: 8,
            iLayerType: 0,
            ..Default::default()
        };

        let slint_pf = choose_pixel_format(slint_dc, &pfd);
        if slint_pf == 0 {
            release_dc(slint_hwnd, slint_dc);
            return Err("ChoosePixelFormat failed for Slint DC".into());
        }
        if set_pixel_format(slint_dc, slint_pf, &pfd) == 0 {
            release_dc(slint_hwnd, slint_dc);
            return Err("SetPixelFormat failed for Slint DC".into());
        }

        // Obtém contexto atual do Slint (já criado pelo renderer Skia)
        let slint_glrc = wgl_get_current_context();
        if slint_glrc.is_null() {
            release_dc(slint_hwnd, slint_dc);
            return Err("Could not get Slint GL context (wglGetCurrentContext returned null)".into());
        }

        // Cria contexto overlay com mesmo pixel format
        let overlay_dc = get_dc(slint_hwnd); // Mesmo DC
        let overlay_glrc = wgl_create_context(overlay_dc);
        if overlay_glrc.is_null() {
            release_dc(slint_hwnd, slint_dc);
            release_dc(slint_hwnd, overlay_dc);
            return Err("wglCreateContext failed for overlay".into());
        }

        // Compartilha listas
        if wgl_share_lists(slint_glrc, overlay_glrc) == 0 {
            let err = get_last_error();
            release_dc(slint_hwnd, slint_dc);
            release_dc(slint_hwnd, overlay_dc);
            wgl_delete_context(overlay_glrc);
            return Err(format!("wglShareLists failed: {:?}", err));
        }

        // Torna overlay atual
        if wgl_make_current(overlay_dc, overlay_glrc) == 0 {
            release_dc(slint_hwnd, slint_dc);
            release_dc(slint_hwnd, overlay_dc);
            wgl_delete_context(overlay_glrc);
            return Err("wglMakeCurrent failed for overlay".into());
        }

        // Cria glow::Context
        let gl = Context::from_loader_function(|s| {
            let cstr = match CString::new(s) {
                Ok(c) => c,
                Err(_) => return std::ptr::null(),
            };
            let proc = wgl_get_proc_address(cstr.as_ptr() as *const u8);
            proc as *const c_void
        });

        release_dc(slint_hwnd, slint_dc);
        release_dc(slint_hwnd, overlay_dc);

        Ok((Arc::new(gl), slint_hwnd))
    }
}

/// Stub para não-Windows
#[cfg(not(windows))]
pub fn create_shared_gl_context(
    _slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut c_void), String> {
    Err("Non-Windows GL context sharing not implemented".into())
}