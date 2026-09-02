//! Windows GDI/WGL FFI Dinâmico
//!
//! Carrega funções GDI/WGL que faltam no windows-sys 0.61 via libloading.
//! Funções: ChoosePixelFormat, SetPixelFormat, PIXELFORMATDESCRIPTOR,
//! PFD_* constants, wglGetProcAddress, wglCreateContext, wglMakeCurrent,
//! wglShareLists, wglDeleteContext, wglGetCurrentContext, wglSwapLayerBuffers.

use libloading::{Library, Symbol};
use std::ffi::c_void;
use std::sync::OnceLock;

// ============================================================
// Constantes PFD (Pixel Format Descriptor)
// ============================================================
pub const PFD_DRAW_TO_WINDOW: u32 = 0x00000004;
pub const PFD_SUPPORT_OPENGL: u32 = 0x00000020;
pub const PFD_DOUBLEBUFFER: u32 = 0x00000001;
pub const PFD_TYPE_RGBA: u8 = 0;
pub const WGL_SWAP_MAIN_PLANE: u32 = 1;

// ============================================================
// Structs Windows (replicadas do windows-sys)
// ============================================================

#[repr(C)]
pub struct PIXELFORMATDESCRIPTOR {
    pub nSize: u16,
    pub nVersion: u16,
    pub dwFlags: u32,
    pub iPixelType: u8,
    pub cColorBits: u8,
    pub cRedBits: u8,
    pub cRedShift: u8,
    pub cGreenBits: u8,
    pub cGreenShift: u8,
    pub cBlueBits: u8,
    pub cBlueShift: u8,
    pub cAlphaBits: u8,
    pub cAlphaShift: u8,
    pub cAccumBits: u8,
    pub cAccumRedBits: u8,
    pub cAccumGreenBits: u8,
    pub cAccumBlueBits: u8,
    pub cAccumAlphaBits: u8,
    pub cDepthBits: u8,
    pub cStencilBits: u8,
    pub cAuxBuffers: u8,
    pub iLayerType: u8,
    pub bReserved: u8,
    pub dwLayerMask: u32,
    pub dwVisibleMask: u32,
    pub dwDamageMask: u32,
}

impl Default for PIXELFORMATDESCRIPTOR {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

// ============================================================
// Tipos Windows
// ============================================================
pub type HDC = *mut c_void;
pub type HGLRC = *mut c_void;
pub type HWND = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HRESULT = i32;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type HDC__ = c_void;
pub type HGLRC__ = c_void;

// ============================================================
// Function pointers (carregadas dinamicamente)
// ============================================================
type FnGetDC = unsafe extern "system" fn(hwnd: *mut c_void) -> HDC;
type FnReleaseDC = unsafe extern "system" fn(hwnd: *mut c_void, hdc: HDC) -> i32;
type FnChoosePixelFormat = unsafe extern "system" fn(hdc: HDC, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32;
type FnSetPixelFormat = unsafe extern "system" fn(hdc: HDC, format: i32, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32;
type FnWglCreateContext = unsafe extern "system" fn(hdc: HDC) -> HGLRC;
type FnWglMakeCurrent = unsafe extern "system" fn(hdc: HDC, hglrc: HGLRC) -> i32;
type FnWglShareLists = unsafe extern "system" fn(hglrc1: HGLRC, hglrc2: HGLRC) -> i32;
type FnWglDeleteContext = unsafe extern "system" fn(hglrc: HGLRC) -> i32;
type FnWglGetCurrentContext = unsafe extern "system" fn() -> HGLRC;
type FnWglGetProcAddress = unsafe extern "system" fn(lpszProc: *const u8) -> *mut c_void;
type FnWglSwapLayerBuffers = unsafe extern "system" fn(hdc: HDC, fuPlanes: u32) -> i32;
type FnGetLastError = unsafe extern "system" fn() -> u32;

// ============================================================
// Biblioteca carregada
// ============================================================
struct GdiLib {
    lib: Library,
    get_dc: Symbol<FnGetDC>,
    release_dc: Symbol<FnReleaseDC>,
    choose_pixel_format: Symbol<FnChoosePixelFormat>,
    set_pixel_format: Symbol<FnSetPixelFormat>,
    wgl_create_context: Symbol<FnWglCreateContext>,
    wgl_make_current: Symbol<FnWglMakeCurrent>,
    wgl_share_lists: Symbol<FnWglShareLists>,
    wgl_delete_context: Symbol<FnWglDeleteContext>,
    wgl_get_current_context: Symbol<FnWglGetCurrentContext>,
    wgl_get_proc_address: Symbol<FnWglGetProcAddress>,
    wgl_swap_layer_buffers: Symbol<FnWglSwapLayerBuffers>,
    get_last_error: Symbol<FnGetLastError>,
}

static GDI_LIB: OnceLock<GdiLib> = OnceLock::new();

fn get_gdi_lib() -> &'static GdiLib {
    GDI_LIB.get_or_init(|| {
        // Carrega gdi32.dll e opengl32.dll
        unsafe {
            let gdi32 = Library::new("gdi32.dll").expect("Failed to load gdi32.dll");
            let opengl32 = Library::new("opengl32.dll").expect("Failed to load opengl32.dll");

            GdiLib {
                get_dc: gdi32.get(b"GetDC\0").expect("GetDC not found"),
                release_dc: gdi32.get(b"ReleaseDC\0").expect("ReleaseDC not found"),
                choose_pixel_format: gdi32.get(b"ChoosePixelFormat\0").expect("ChoosePixelFormat not found"),
                set_pixel_format: gdi32.get(b"SetPixelFormat\0").expect("SetPixelFormat not found"),
                wgl_create_context: opengl32.get(b"wglCreateContext\0").expect("wglCreateContext not found"),
                wgl_make_current: opengl32.get(b"wglMakeCurrent\0").expect("wglMakeCurrent not found"),
                wgl_share_lists: opengl32.get(b"wglShareLists\0").expect("wglShareLists not found"),
                wgl_delete_context: opengl32.get(b"wglDeleteContext\0").expect("wglDeleteContext not found"),
                wgl_get_current_context: opengl32.get(b"wglGetCurrentContext\0").expect("wglGetCurrentContext not found"),
                wgl_get_proc_address: opengl32.get(b"wglGetProcAddress\0").expect("wglGetProcAddress not found"),
                wgl_swap_layer_buffers: opengl32.get(b"wglSwapLayerBuffers\0").expect("wglSwapLayerBuffers not found"),
                get_last_error: Library::new("kernel32.dll").unwrap().get(b"GetLastError\0").expect("GetLastError not found"),
            }
        }
    })
}

// ============================================================
// Wrappers públicos
// ============================================================

pub unsafe fn get_dc(hwnd: *mut c_void) -> *mut c_void {
    get_gdi_lib().get_dc(hwnd)
}

pub unsafe fn release_dc(hwnd: *mut c_void, hdc: *mut c_void) -> i32 {
    get_gdi_lib().release_dc(hwnd, hdc)
}

pub unsafe fn choose_pixel_format(hdc: *mut c_void, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32 {
    get_gdi_lib().choose_pixel_format(hdc, ppfd)
}

pub unsafe fn set_pixel_format(hdc: *mut c_void, format: i32, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32 {
    get_gdi_lib().set_pixel_format(hdc, format, ppfd)
}

pub unsafe fn wgl_create_context(hdc: *mut c_void) -> *mut c_void {
    get_gdi_lib().wgl_create_context(hdc)
}

pub unsafe fn wgl_make_current(hdc: *mut c_void, hglrc: *mut c_void) -> i32 {
    get_gdi_lib().wgl_make_current(hdc, hglrc)
}

pub unsafe fn wgl_share_lists(hglrc1: *mut c_void, hglrc2: *mut c_void) -> i32 {
    get_gdi_lib().wgl_share_lists(hglrc1, hglrc2)
}

pub unsafe fn wgl_delete_context(hglrc: *mut c_void) -> i32 {
    get_gdi_lib().wgl_delete_context(hglrc)
}

pub unsafe fn wgl_get_current_context() -> *mut c_void {
    get_gdi_lib().wgl_get_current_context()
}

pub unsafe fn wgl_get_proc_address(proc: *const u8) -> *mut c_void {
    get_gdi_lib().wgl_get_proc_address(proc)
}

pub unsafe fn wgl_swap_layer_buffers(hdc: *mut c_void, fu_planes: u32) -> i32 {
    get_gdi_lib().wgl_swap_layer_buffers(hdc, fu_planes)
}

pub unsafe fn get_last_error() -> u32 {
    get_gdi_lib().get_last_error()
}