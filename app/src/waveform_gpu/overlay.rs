//! GPU Context creation — cria contexto OpenGL independente para waveform GPU overlay.
//!
//! Estratégia de fallback em 3 níveis (Windows):
//!   1. **Caminho rápido**: tenta `wglCreateContext` diretamente no DC da janela Slint.
//!      Funciona se Slint estiver usando backend ANGLE/OpenGL (configurado via feature).
//!   2. **Janela helper invisível**: se (1) falhar (ex.: Slint/Skia Direct2D, PFD sem WGL),
//!      cria uma janela message-only OWNDC invisível, define PFD com suporte a OpenGL nela
//!      (SetPixelFormat válido aqui porque é janela recém-criada, sem PFD prévio), e cria
//!      o contexto GL nesse DC dedicado.
//!   3. **Graceful fallback**: se (2) também falhar, retorna `Err` com detalhe; a app
//!      continua rodando sem render GPU de waveform (nenhum `unwrap` no renderer).

use crate::waveform_gpu::win32_gdi::*;
use glow::Context;
use libloading::Library;
use raw_window_handle::RawWindowHandle;
use std::ffi::{c_void, CString};
use std::sync::{Arc, OnceLock};

/// WNDPROC padrão para a janela helper — só despacha para DefWindowProcW.
unsafe extern "system" fn helper_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { def_window_proc_w(hwnd, msg, wparam, lparam) }
}

/// Cria contexto OpenGL independente.
/// Retorna (Arc<Context>, hwnd) onde hwnd é o HWND para `SwapBuffers` posterior.
#[cfg(windows)]
pub fn create_shared_gl_context(
    slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut c_void), String> {
    unsafe {
        let slint_hwnd = match slint_window_handle {
            RawWindowHandle::Win32(h) => h.hwnd.get() as *mut c_void,
            _ => return Err("Slint window handle not Win32".into()),
        };

        // ============================================================
        // PASSO 1 — Caminho rápido: contexto direto no DC da janela Slint
        // (funciona se Slint usar ANGLE/OpenGL como renderer)
        // ============================================================
        if let Ok(result) = try_context_on_slint_dc(slint_hwnd) {
            eprintln!("[waveform_gpu] Context created directly on Slint DC");
            return Ok(result);
        }

        eprintln!("[waveform_gpu] Step 1 failed (Slint DC PFD incompatible) — trying helper window");

        // ============================================================
        // PASSO 2 — Janela helper invisível (message-only) com OWNDC + PFD próprio
        // ============================================================
        match try_context_on_helper_window(slint_hwnd) {
            Ok(result) => {
                eprintln!("[waveform_gpu] Context created on helper message-only window");
                Ok(result)
            }
            Err(e) => {
                eprintln!("[waveform_gpu] Step 2 failed: {}", e);
                // ============================================================
                // PASSO 3 — Graceful fallback: app continua sem GPU
                // ============================================================
                Err(format!(
                    "All GL context strategies failed (Slint DC + helper window). Last: {}",
                    e
                ))
            }
        }
    }
}

/// PASSO 1: Tenta criar contexto diretamente no DC da janela Slint.
#[cfg(windows)]
unsafe fn try_context_on_slint_dc(
    slint_hwnd: *mut c_void,
) -> Result<(Arc<Context>, *mut c_void), String> {
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
        // Mantemos DC + contexto current (render loop 60FPS precisa).
        Ok((Arc::new(gl), slint_hwnd))
    }
}

/// PASSO 2: Cria janela helper invisível message-only, configura PFD nela,
/// cria contexto GL no seu DC dedicado.
#[cfg(windows)]
unsafe fn try_context_on_helper_window(
    slint_hwnd_for_swap: *mut c_void,
) -> Result<(Arc<Context>, *mut c_void), String> {
    unsafe {
        // --- 2.1 Obter hInstance do módulo atual ---
        let h_instance = get_module_handle_w(std::ptr::null());
        if h_instance.is_null() {
            return Err(format!("GetModuleHandleW(null) GetLastError={}", get_last_error()));
        }

        // --- 2.2 Registrar classe L"RuanaGLHelper" com CS_OWNDC ---
        // CS_OWNDC garante que o DC é exclusivo da janela e persistente
        // (não retorna ao cache do GDI após ReleaseDC).
        static CLASS_NAME: &[u16] = &[
            b'R' as u16, b'u' as u16, b'a' as u16, b'n' as u16, b'a' as u16,
            b'G' as u16, b'L' as u16, b'H' as u16, b'e' as u16, b'l' as u16,
            b'p' as u16, b'e' as u16, b'r' as u16, 0,
        ];

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

        let atom = register_class_ex_w(&wnd_class);
        if atom == 0 {
            let err = get_last_error();
            // ERROR_CLASS_ALREADY_EXISTS (1410) é OK — classe foi registrada
            // em tentativa anterior. Apenas continuamos.
            if err != 1410 {
                return Err(format!("RegisterClassExW GetLastError={}", err));
            }
        }

        // --- 2.3 Criar janela message-only invisível 1x1 ---
        // hWndParent = HWND_MESSAGE → janela "message-only":
        //   - Não é visível, não tem taskbar button, não recebe input.
        //   - Só existe para despachar mensagens e ter um DC OWNDC válido.
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

        // --- 2.4 Obter DC da helper window ---
        let helper_dc = get_dc(helper_hwnd);
        if helper_dc.is_null() {
            let err = get_last_error();
            destroy_window(helper_hwnd);
            return Err(format!("GetDC(helper) GetLastError={}", err));
        }

        // --- 2.5 Configurar PIXELFORMATDESCRIPTOR no helper DC ---
        // Aqui SetPixelFormat é VÁLIDO: é um DC novo de janela recém-criada
        // (o helper nunca teve SetPixelFormat chamado antes — one-shot rule respeitada).
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

        // --- 2.6 Criar contexto GL no DC da helper ---
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

        // --- 2.7 Inicializar glow::Context ---
        let gl = make_glow_context();

        // Mantemos:
        //   - helper_hwnd vivo (sem destroy_window) → DC persistente.
        //   - helper_dc atrelado (sem release_dc) → CS_OWNDC cuida do caching.
        //   - hglrc current → render loop 60FPS precisa.
        // Para swap buffers posterior, usamos `slint_hwnd_for_swap` (parâmetro),
        // porque é a janela visível real do app (a helper é invisível).
        Ok((Arc::new(gl), slint_hwnd_for_swap))
    }
}

/// Constroi o glow::Context loader: tenta wglGetProcAddress primeiro
/// (extensões / GL > 1.1), depois busca em opengl32.dll estaticamente
/// (GL 1.0/1.1 — wglGetProcAddress NÃO retorna essas por especificação).
#[cfg(windows)]
unsafe fn make_glow_context() -> Context {
    unsafe {
        // Pré-carrega opengl32.dll para funções core 1.1
        static OGL32_ONCE: OnceLock<Option<&'static Library>> = OnceLock::new();
        let ogl32 = OGL32_ONCE.get_or_init(|| {
            // Safety: Library carregado e leakado para 'static lifetime
            let lib = Library::new("opengl32.dll").ok()?;
            Some(&*Box::leak(Box::new(lib)))
        });

        Context::from_loader_function(|s| {
            let Ok(cstr) = CString::new(s) else {
                return std::ptr::null();
            };
            // 1) wglGetProcAddress — extensões e GL moderno
            let proc = wgl_get_proc_address(cstr.as_ptr() as *const u8);
            if !proc.is_null() {
                return proc as *const c_void;
            }
            // 2) fallback: busca direta no opengl32.dll (GL 1.0/1.1 estáticas)
            if let Some(lib) = ogl32 {
                if let Ok(sym) = lib.get::<unsafe extern "C" fn()>(cstr.as_bytes_with_nul())
                {
                    return *sym as *const c_void;
                }
            }
            std::ptr::null()
        })
    }
}

/// Stub para não-Windows
#[cfg(not(windows))]
pub fn create_shared_gl_context(
    _slint_window_handle: RawWindowHandle,
) -> Result<(Arc<Context>, *mut c_void), String> {
    Err("Non-Windows GL context sharing not implemented".into())
}
