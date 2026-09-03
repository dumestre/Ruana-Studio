//! Linux EGL FFI Dinâmico — cria um contexto OpenGL via EGL para o
//! renderer GPU de waveform (equivalente Linux do `win32_gdi`).
//!
//! Usa `libloading` para carregar `libEGL.so.1` em runtime, sem linkar
//! contra o driver na build. Não requer window handle: cria um contexto
//! num pbuffer offscreen (EGL surface) que o renderer usa para as
//! operações GL e `eglSwapBuffers` para apresentar.

#[cfg(target_os = "linux")]
pub mod imp {
    use libloading::{Library, Symbol};
    use std::sync::OnceLock;

    // Tipos EGL (handles).
    pub type EGLDisplay = *mut std::ffi::c_void;
    pub type EGLConfig = *mut std::ffi::c_void;
    pub type EGLSurface = *mut std::ffi::c_void;
    pub type EGLContext = *mut std::ffi::c_void;
    pub type EGLNativeDisplayType = *const std::ffi::c_void;
    pub type EGLint = i32;

    pub const EGL_DEFAULT_DISPLAY: EGLNativeDisplayType = std::ptr::null();
    pub const EGL_NO_CONTEXT: EGLContext = std::ptr::null_mut();

    pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
    pub const EGL_PBUFFER_BIT: EGLint = 0x0001;
    pub const EGL_WINDOW_BIT: EGLint = 0x0004;
    pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
    pub const EGL_OPENGL_BIT: EGLint = 0x0008;
    pub const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
    pub const EGL_RED_SIZE: EGLint = 0x3024;
    pub const EGL_GREEN_SIZE: EGLint = 0x3023;
    pub const EGL_BLUE_SIZE: EGLint = 0x3022;
    pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
    pub const EGL_DEPTH_SIZE: EGLint = 0x3025;
    pub const EGL_STENCIL_SIZE: EGLint = 0x3026;
    pub const EGL_NONE: EGLint = 0x3038;

    pub const EGL_WIDTH: EGLint = 0x3057;
    pub const EGL_HEIGHT: EGLint = 0x3056;
    pub const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
    pub const EGL_EXTENSIONS: EGLint = 0x3055;

    type FnGetDisplay =
        unsafe extern "system" fn(dpy: EGLNativeDisplayType) -> EGLDisplay;
    type FnInitialize = unsafe extern "system" fn(
        dpy: EGLDisplay,
        major: *mut EGLint,
        minor: *mut EGLint,
    ) -> EGLint;
    type FnChooseConfig = unsafe extern "system" fn(
        dpy: EGLDisplay,
        attribs: *const EGLint,
        configs: *mut EGLConfig,
        config_size: EGLint,
        num_config: *mut EGLint,
    ) -> EGLint;
    type FnCreatePbufferSurface = unsafe extern "system" fn(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribs: *const EGLint,
    ) -> EGLSurface;
    type FnGetConfigAttrib = unsafe extern "system" fn(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> EGLint;
    type FnCreateContext = unsafe extern "system" fn(
        dpy: EGLDisplay,
        config: EGLConfig,
        share_context: EGLContext,
        attribs: *const EGLint,
    ) -> EGLContext;
    type FnMakeCurrent = unsafe extern "system" fn(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> EGLint;
    type FnSwapBuffers =
        unsafe extern "system" fn(dpy: EGLDisplay, surface: EGLSurface) -> EGLint;
    type FnGetProcAddress =
        unsafe extern "system" fn(name: *const u8) -> *const std::ffi::c_void;
    type FnGetError = unsafe extern "system" fn() -> EGLint;
    type FnDestroySurface =
        unsafe extern "system" fn(dpy: EGLDisplay, surface: EGLSurface) -> EGLint;
    type FnDestroyContext =
        unsafe extern "system" fn(dpy: EGLDisplay, ctx: EGLContext) -> EGLint;

    struct EglLib {
        get_display: Symbol<'static, FnGetDisplay>,
        initialize: Symbol<'static, FnInitialize>,
        choose_config: Symbol<'static, FnChooseConfig>,
        create_pbuffer_surface: Symbol<'static, FnCreatePbufferSurface>,
        get_config_attrib: Symbol<'static, FnGetConfigAttrib>,
        create_context: Symbol<'static, FnCreateContext>,
        make_current: Symbol<'static, FnMakeCurrent>,
        swap_buffers: Symbol<'static, FnSwapBuffers>,
        get_proc_address: Symbol<'static, FnGetProcAddress>,
        get_error: Symbol<'static, FnGetError>,
        destroy_surface: Symbol<'static, FnDestroySurface>,
        destroy_context: Symbol<'static, FnDestroyContext>,
    }

    static EGL_LIB: OnceLock<Option<EglLib>> = OnceLock::new();

    fn egl() -> Option<&'static EglLib> {
        EGL_LIB
            .get_or_init(|| unsafe {
                let lib = Library::new("libEGL.so.1").ok()?;
                // Leak para 'static (o lib fica carregado até a app sair).
                let lib = Box::leak(Box::new(lib));
                Some(EglLib {
                    get_display: lib.get(b"eglGetDisplay\0").ok()?,
                    initialize: lib.get(b"eglInitialize\0").ok()?,
                    choose_config: lib.get(b"eglChooseConfig\0").ok()?,
                    create_pbuffer_surface: lib.get(b"eglCreatePbufferSurface\0").ok()?,
                    get_config_attrib: lib.get(b"eglGetConfigAttrib\0").ok()?,
                    create_context: lib.get(b"eglCreateContext\0").ok()?,
                    make_current: lib.get(b"eglMakeCurrent\0").ok()?,
                    swap_buffers: lib.get(b"eglSwapBuffers\0").ok()?,
                    get_proc_address: lib.get(b"eglGetProcAddress\0").ok()?,
                    get_error: lib.get(b"eglGetError\0").ok()?,
                    destroy_surface: lib.get(b"eglDestroySurface\0").ok()?,
                    destroy_context: lib.get(b"eglDestroyContext\0").ok()?,
                })
            })
            .as_ref()
    }

    pub unsafe fn egl_get_display() -> EGLDisplay {
        unsafe { match egl() { Some(lib) => (lib.get_display)(EGL_DEFAULT_DISPLAY), None => std::ptr::null_mut() } }
    }

    pub unsafe fn egl_initialize(dpy: EGLDisplay) -> bool {
        unsafe {
            match egl() {
                Some(lib) => {
                    let mut major = 0;
                    let mut minor = 0;
                    (lib.initialize)(dpy, &mut major, &mut minor) == 1
                }
                None => false,
            }
        }
    }

    /// Escolhe um config com suporte a pbuffer. Prioriza OpenGL (desktop);
    /// se não houver, cai para OpenGL ES 2.
    pub unsafe fn egl_choose_pbuffer_config(dpy: EGLDisplay) -> Option<EGLConfig> {
        unsafe {
            let lib = egl()?;
            const MAX: EGLint = 64;
            let mut configs: [EGLConfig; MAX as usize] = [std::ptr::null_mut(); MAX as usize];
            let mut num = 0;

            // Tenta OpenGL desktop primeiro.
            let attrs = [
                EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
                EGL_NONE,
            ];
            if (lib.choose_config)(dpy, attrs.as_ptr(), configs.as_mut_ptr(), MAX, &mut num) == 1
                && num > 0
            {
                return Some(configs[0]);
            }

            // Fallback: OpenGL ES 2.
            let attrs = [
                EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                EGL_NONE,
            ];
            num = 0;
            if (lib.choose_config)(dpy, attrs.as_ptr(), configs.as_mut_ptr(), MAX, &mut num) == 1
                && num > 0
            {
                return Some(configs[0]);
            }
            None
        }
    }

    pub unsafe fn egl_get_config_value(dpy: EGLDisplay, config: EGLConfig, attr: EGLint) -> EGLint {
        unsafe {
            let mut v = 0;
            if let Some(lib) = egl() {
                let _ = (lib.get_config_attrib)(dpy, config, attr, &mut v);
            }
            v
        }
    }

    pub unsafe fn egl_create_pbuffer_surface(
        dpy: EGLDisplay,
        config: EGLConfig,
        width: EGLint,
        height: EGLint,
    ) -> EGLSurface {
        unsafe {
            match egl() {
                Some(lib) => {
                    let attrs = [EGL_WIDTH, width, EGL_HEIGHT, height, EGL_NONE];
                    (lib.create_pbuffer_surface)(dpy, config, attrs.as_ptr())
                }
                None => std::ptr::null_mut(),
            }
        }
    }

    pub unsafe fn egl_create_context(
        dpy: EGLDisplay,
        config: EGLConfig,
    ) -> EGLContext {
        unsafe {
            match egl() {
                Some(lib) => {
                    // Se o config é ES2, passamos client_version=2; senão, sem atributos.
                    let renderable = egl_get_config_value(dpy, config, EGL_RENDERABLE_TYPE);
                    let use_es = renderable & EGL_OPENGL_ES2_BIT != 0;
                    let attrs = if use_es {
                        [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE]
                    } else {
                        [EGL_NONE, EGL_NONE, EGL_NONE]
                    };
                    (lib.create_context)(dpy, config, EGL_NO_CONTEXT, attrs.as_ptr())
                }
                None => std::ptr::null_mut(),
            }
        }
    }

    pub unsafe fn egl_make_current(dpy: EGLDisplay, surface: EGLSurface, ctx: EGLContext) -> bool {
        unsafe {
            match egl() {
                Some(lib) => (lib.make_current)(dpy, surface, surface, ctx) == 1,
                None => false,
            }
        }
    }

    pub unsafe fn egl_swap_buffers(dpy: EGLDisplay, surface: EGLSurface) {
        unsafe {
            if let Some(lib) = egl() {
                let _ = (lib.swap_buffers)(dpy, surface);
            }
        }
    }

    pub unsafe fn egl_get_proc_address(name: *const u8) -> *const std::ffi::c_void {
        unsafe {
            match egl() {
                Some(lib) => (lib.get_proc_address)(name),
                None => std::ptr::null(),
            }
        }
    }

    pub unsafe fn egl_get_error() -> EGLint {
        unsafe {
            match egl() {
                Some(lib) => (lib.get_error)(),
                None => -1,
            }
        }
    }

    pub unsafe fn egl_destroy_surface(dpy: EGLDisplay, surface: EGLSurface) {
        unsafe {
            if let Some(lib) = egl() {
                let _ = (lib.destroy_surface)(dpy, surface);
            }
        }
    }

    pub unsafe fn egl_destroy_context(dpy: EGLDisplay, ctx: EGLContext) {
        unsafe {
            if let Some(lib) = egl() {
                let _ = (lib.destroy_context)(dpy, ctx);
            }
        }
    }
}
