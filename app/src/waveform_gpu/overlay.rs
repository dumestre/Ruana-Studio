//! GPU Context creation — cria contexto OpenGL compartilhado com Slint.
//!
//! No Windows, extrai o HWND do Slint e cria contexto WGL compartilhado.
//! Usa windows-sys para API Windows estável.

use glow::Context;
use raw_window_handle::RawWindowHandle;
use std::sync::Arc;

/// Cria contexto OpenGL compartilhado com a janela Slint.
/// Retorna (Arc<Context>, hwnd) onde hwnd é o HWND da janela Slint (para swap buffers).
#[cfg(windows)]
pub fn create_shared_gl_context(
    slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut std::ffi::c_void), String> {
    use glow::Context;
    use raw_window_handle::RawWindowHandle;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        GetDC, ReleaseDC, SetPixelFormat, ChoosePixelFormat, PIXELFORMATDESCRIPTOR,
        PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_DOUBLEBUFFER, PFD_TYPE_RGBA,
    };
    use windows_sys::Win32::Graphics::OpenGL::{
        wglCreateContext, wglMakeCurrent, wglShareLists, wglDeleteContext, wglGetCurrentContext,
        wglGetProcAddress,
    };

    unsafe {
        // Obtém HWND da janela Slint
        let slint_hwnd = match slint_window_handle {
            RawWindowHandle::Win32(h) => HWND(h.hwnd.get() as *mut _),
            _ => return Err("Slint window handle not Win32".into()),
        };

        // Device context da janela Slint
        let slint_dc = GetDC(Some(slint_hwnd));

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
            ..std::mem::zeroed()
        };

        let slint_pf = ChoosePixelFormat(slint_dc, &pfd);
        if slint_pf == 0 {
            ReleaseDC(Some(slint_hwnd), slint_dc);
            return Err("ChoosePixelFormat failed for Slint DC".into());
        }
        if SetPixelFormat(slint_dc, slint_pf, &pfd) == 0 {
            ReleaseDC(Some(slint_hwnd), slint_dc);
            return Err("SetPixelFormat failed for Slint DC".into());
        }

        // Obtém contexto atual do Slint (já criado pelo renderer Skia)
        let slint_glrc = wglGetCurrentContext();
        if slint_glrc.is_null() {
            ReleaseDC(Some(slint_hwnd), slint_dc);
            return Err("Could not get Slint GL context (wglGetCurrentContext returned null)".into());
        }

        // Cria contexto overlay com mesmo pixel format
        let overlay_dc = GetDC(Some(slint_hwnd)); // Mesmo DC
        let overlay_glrc = wglCreateContext(overlay_dc);
        if overlay_glrc.is_null() {
            ReleaseDC(Some(slint_hwnd), slint_dc);
            ReleaseDC(Some(slint_hwnd), overlay_dc);
            return Err("wglCreateContext failed for overlay".into());
        }

        // Compartilha listas
        if wglShareLists(slint_glrc, overlay_glrc) == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            ReleaseDC(Some(slint_hwnd), slint_dc);
            ReleaseDC(Some(slint_hwnd), overlay_dc);
            wglDeleteContext(overlay_glrc);
            return Err(format!("wglShareLists failed: {:?}", err));
        }

        // Torna overlay atual
        if wglMakeCurrent(overlay_dc, overlay_glrc) == 0 {
            ReleaseDC(Some(slint_hwnd), slint_dc);
            ReleaseDC(Some(slint_hwnd), overlay_dc);
            wglDeleteContext(overlay_glrc);
            return Err("wglMakeCurrent failed for overlay".into());
        }

        // Cria glow::Context
        let gl = Context::from_loader_function(|s| {
            let cstr = match std::ffi::CString::new(s) {
                Ok(c) => c,
                Err(_) => return std::ptr::null(),
            };
            let proc = wglGetProcAddress(cstr.as_ptr() as *const u8);
            proc as *const c_void
        });

        ReleaseDC(Some(slint_hwnd), slint_dc);
        ReleaseDC(Some(slint_hwnd), overlay_dc);

        Ok((Arc::new(gl), slint_hwnd as *mut c_void))
    }
}

/// Stub para não-Windows
#[cfg(not(windows))]
pub fn create_shared_gl_context(
    _slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut std::ffi::c_void), String> {
    Err("Non-Windows GL context sharing not implemented".into())
}