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
    pub n_size: u16,
    pub n_version: u16,
    pub dw_flags: u32,
    pub i_pixel_type: u8,
    pub c_color_bits: u8,
    pub c_red_bits: u8,
    pub c_red_shift: u8,
    pub c_green_bits: u8,
    pub c_green_shift: u8,
    pub c_blue_bits: u8,
    pub c_blue_shift: u8,
    pub c_alpha_bits: u8,
    pub c_alpha_shift: u8,
    pub c_accum_bits: u8,
    pub c_accum_red_bits: u8,
    pub c_accum_green_bits: u8,
    pub c_accum_blue_bits: u8,
    pub c_accum_alpha_bits: u8,
    pub c_depth_bits: u8,
    pub c_stencil_bits: u8,
    pub c_aux_buffers: u8,
    pub i_layer_type: u8,
    pub b_reserved: u8,
    pub dw_layer_mask: u32,
    pub dw_visible_mask: u32,
    pub dw_damage_mask: u32,
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
struct GdiLib<'a> {
    get_dc: Symbol<'a, FnGetDC>,
    release_dc: Symbol<'a, FnReleaseDC>,
    choose_pixel_format: Symbol<'a, FnChoosePixelFormat>,
    set_pixel_format: Symbol<'a, FnSetPixelFormat>,
    wgl_create_context: Symbol<'a, FnWglCreateContext>,
    wgl_make_current: Symbol<'a, FnWglMakeCurrent>,
    wgl_share_lists: Symbol<'a, FnWglShareLists>,
    wgl_delete_context: Symbol<'a, FnWglDeleteContext>,
    wgl_get_current_context: Symbol<'a, FnWglGetCurrentContext>,
    wgl_get_proc_address: Symbol<'a, FnWglGetProcAddress>,
    wgl_swap_layer_buffers: Symbol<'a, FnWglSwapLayerBuffers>,
    get_last_error: Symbol<'a, FnGetLastError>,
}

static GDI_LIB: OnceLock<GdiLib<'static>> = OnceLock::new();

fn get_gdi_lib() -> &'static GdiLib<'static> {
    GDI_LIB.get_or_init(|| {
        unsafe {
            let user32 = Box::leak(Box::new(Library::new("user32.dll").expect("Failed to load user32.dll")));
            let gdi32 = Box::leak(Box::new(Library::new("gdi32.dll").expect("Failed to load gdi32.dll")));
            let opengl32 = Box::leak(Box::new(Library::new("opengl32.dll").expect("Failed to load opengl32.dll")));
            let kernel32 = Box::leak(Box::new(Library::new("kernel32.dll").expect("Failed to load kernel32.dll")));

            GdiLib {
                get_dc: user32.get(b"GetDC\0").expect("GetDC not found"),
                release_dc: user32.get(b"ReleaseDC\0").expect("ReleaseDC not found"),
                choose_pixel_format: gdi32.get(b"ChoosePixelFormat\0").expect("ChoosePixelFormat not found"),
                set_pixel_format: gdi32.get(b"SetPixelFormat\0").expect("SetPixelFormat not found"),
                wgl_create_context: opengl32.get(b"wglCreateContext\0").expect("wglCreateContext not found"),
                wgl_make_current: opengl32.get(b"wglMakeCurrent\0").expect("wglMakeCurrent not found"),
                wgl_share_lists: opengl32.get(b"wglShareLists\0").expect("wglShareLists not found"),
                wgl_delete_context: opengl32.get(b"wglDeleteContext\0").expect("wglDeleteContext not found"),
                wgl_get_current_context: opengl32.get(b"wglGetCurrentContext\0").expect("wglGetCurrentContext not found"),
                wgl_get_proc_address: opengl32.get(b"wglGetProcAddress\0").expect("wglGetProcAddress not found"),
                wgl_swap_layer_buffers: opengl32.get(b"wglSwapLayerBuffers\0").expect("wglSwapLayerBuffers not found"),
                get_last_error: kernel32.get(b"GetLastError\0").expect("GetLastError not found"),
            }
        }
    })
}

// ============================================================
// Wrappers públicos
// ============================================================

pub unsafe fn get_dc(hwnd: *mut c_void) -> *mut c_void {
    (get_gdi_lib().get_dc)(hwnd)
}

pub unsafe fn release_dc(hwnd: *mut c_void, hdc: *mut c_void) -> i32 {
    (get_gdi_lib().release_dc)(hwnd, hdc)
}

pub unsafe fn choose_pixel_format(hdc: *mut c_void, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32 {
    (get_gdi_lib().choose_pixel_format)(hdc, ppfd)
}

pub unsafe fn set_pixel_format(hdc: *mut c_void, format: i32, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32 {
    (get_gdi_lib().set_pixel_format)(hdc, format, ppfd)
}

pub unsafe fn wgl_create_context(hdc: *mut c_void) -> *mut c_void {
    (get_gdi_lib().wgl_create_context)(hdc)
}

pub unsafe fn wgl_make_current(hdc: *mut c_void, hglrc: *mut c_void) -> i32 {
    (get_gdi_lib().wgl_make_current)(hdc, hglrc)
}

pub unsafe fn wgl_share_lists(hglrc1: *mut c_void, hglrc2: *mut c_void) -> i32 {
    (get_gdi_lib().wgl_share_lists)(hglrc1, hglrc2)
}

pub unsafe fn wgl_delete_context(hglrc: *mut c_void) -> i32 {
    (get_gdi_lib().wgl_delete_context)(hglrc)
}

pub unsafe fn wgl_get_current_context() -> *mut c_void {
    (get_gdi_lib().wgl_get_current_context)()
}

pub unsafe fn wgl_get_proc_address(proc: *const u8) -> *mut c_void {
    (get_gdi_lib().wgl_get_proc_address)(proc)
}

pub unsafe fn wgl_swap_layer_buffers(hdc: *mut c_void, fu_planes: u32) -> i32 {
    (get_gdi_lib().wgl_swap_layer_buffers)(hdc, fu_planes)
}

pub unsafe fn get_last_error() -> u32 {
    (get_gdi_lib().get_last_error)()
}