use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use windows::Win32::Foundation::{HINSTANCE, HWND, LRESULT, POINT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::ScreenToClient;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    LoadImageW, LoadCursorW, SetCursor, GetCursorPos,
    IMAGE_CURSOR, LR_DEFAULTSIZE, LR_LOADFROMFILE, IDC_ARROW, HCURSOR, WM_SETCURSOR,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{SetWindowSubclass, DefSubclassProc};
#[cfg(windows)]
use windows::core::PCWSTR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCursor {
    Select = 0,
    Slice = 1,
    Glue = 2,
    Draw = 4,
    Erase = 5,
}

impl ToolCursor {
    pub fn from_tool_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(ToolCursor::Select),
            1 => Some(ToolCursor::Slice),
            2 => Some(ToolCursor::Glue),
            4 => Some(ToolCursor::Draw),
            5 => Some(ToolCursor::Erase),
            _ => None,
        }
    }

    pub fn cursor_file_name(&self) -> &'static str {
        match self {
            ToolCursor::Select => "select.cur",
            ToolCursor::Slice => "slice.cur",
            ToolCursor::Glue => "glue.cur",
            ToolCursor::Draw => "draw.cur",
            ToolCursor::Erase => "erase.cur",
        }
    }
}

#[cfg(windows)]
#[derive(Copy, Clone)]
struct CursorHandle {
    hcursor: HCURSOR,
}

#[cfg(windows)]
unsafe impl Send for CursorHandle {}
#[cfg(windows)]
unsafe impl Sync for CursorHandle {}

pub struct CursorManager {
    #[cfg(windows)]
    cursors: HashMap<ToolCursor, CursorHandle>,
    current_tool: Option<ToolCursor>,
}

impl CursorManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(windows)]
        {
            let mut cursors = HashMap::new();
            let cursor_dir = Self::cursor_directory()?;

            for tool in [
                ToolCursor::Select,
                ToolCursor::Slice,
                ToolCursor::Glue,
                ToolCursor::Draw,
                ToolCursor::Erase,
            ] {
                let cur_path = cursor_dir.join(tool.cursor_file_name());
                if !cur_path.exists() {
                    continue;
                }

                let path_wide: Vec<u16> = cur_path
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();

                let hcursor = unsafe {
                    LoadImageW(
                        HINSTANCE::default(),
                        PCWSTR::from_raw(path_wide.as_ptr()),
                        IMAGE_CURSOR,
                        0,
                        0,
                        LR_DEFAULTSIZE | LR_LOADFROMFILE,
                    )
                };

                if let Ok(h) = hcursor {
                    cursors.insert(tool, CursorHandle { hcursor: HCURSOR(h.0) });
                }
            }

            Ok(Self {
                cursors,
                current_tool: None,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                current_tool: None,
            })
        }
    }

    #[cfg(windows)]
    fn cursor_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_dir = exe_path.parent().ok_or("No parent directory")?;
        Ok(exe_dir.join("cursors"))
    }

    #[cfg(windows)]
    pub fn set_window_handle(&mut self, hwnd: HWND) {
        unsafe {
            let _ = ACTIVE_CURSOR.get_or_init(|| Mutex::new(None));
            let _ = TIMELINE_BOUNDS.get_or_init(|| Mutex::new(TimelineBounds::default()));
            let _ = SetWindowSubclass(hwnd, Some(cursor_subclass_proc), SUBCLASS_ID, 0);
        }
    }

    pub fn set_tool(&mut self, tool: ToolCursor) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(windows)]
        {
            if let Some(cursor_handle) = self.cursors.get(&tool) {
                if let Some(mut active) = ACTIVE_CURSOR.get().and_then(|m| m.lock().ok()) {
                    *active = Some(cursor_handle.hcursor.0 as isize);
                }
                unsafe { SetCursor(cursor_handle.hcursor); }
            }
        }
        self.current_tool = Some(tool);
        Ok(())
    }

    pub fn set_timeline_bounds(x: f32, y: f32, w: f32, h: f32) {
        #[cfg(windows)]
        {
            if let Some(mut bounds) = TIMELINE_BOUNDS.get().and_then(|m| m.lock().ok()) {
                bounds.x = x;
                bounds.y = y;
                bounds.w = w;
                bounds.h = h;
                bounds.valid = true;
            }
        }
    }

    pub fn reset_to_default(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(windows)]
        {
            if let Some(mut active) = ACTIVE_CURSOR.get().and_then(|m| m.lock().ok()) {
                *active = None;
            }
            if let Ok(default) = unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW) } {
                unsafe { SetCursor(HCURSOR(default.0)); }
            }
            self.current_tool = None;
        }

        #[cfg(not(windows))]
        {
            self.current_tool = None;
        }

        Ok(())
    }
}

static CURSOR_MANAGER: OnceLock<Mutex<CursorManager>> = OnceLock::new();

#[cfg(windows)]
static ACTIVE_CURSOR: OnceLock<Mutex<Option<isize>>> = OnceLock::new();

#[cfg(windows)]
static TIMELINE_BOUNDS: OnceLock<Mutex<TimelineBounds>> = OnceLock::new();

#[cfg(windows)]
#[derive(Clone, Copy)]
struct TimelineBounds {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    valid: bool,
}

#[cfg(windows)]
impl Default for TimelineBounds {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, w: 0.0, h: 0.0, valid: false }
    }
}

#[cfg(windows)]
const SUBCLASS_ID: usize = 1;

#[cfg(windows)]
unsafe fn is_cursor_over_timeline(hwnd: HWND) -> bool {
    let bounds = match TIMELINE_BOUNDS.get().and_then(|m| m.lock().ok()) {
        Some(b) => *b,
        None => return false,
    };
    if !bounds.valid {
        return false;
    }

    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        return false;
    }
    if !unsafe { ScreenToClient(hwnd, &mut pt) }.as_bool() {
        return false;
    }

    let cx = pt.x as f32;
    let cy = pt.y as f32;
    cx >= bounds.x && cx <= bounds.x + bounds.w && cy >= bounds.y && cy <= bounds.y + bounds.h
}

#[cfg(windows)]
unsafe extern "system" fn cursor_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if msg == WM_SETCURSOR && (lparam.0 & 0xFFFF) as u32 == 1 {
        if unsafe { is_cursor_over_timeline(hwnd) } {
            if let Some(active) = ACTIVE_CURSOR.get().and_then(|m| m.lock().ok()).and_then(|c| *c) {
                unsafe { SetCursor(HCURSOR(active as *mut core::ffi::c_void)) };
                return LRESULT(1);
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

pub fn init_cursor_manager() -> Result<(), Box<dyn std::error::Error>> {
    let _ = CURSOR_MANAGER.get_or_init(|| Mutex::new(CursorManager::new().expect("Failed to create CursorManager")));
    Ok(())
}

fn get_cursor_manager_lock() -> Option<std::sync::MutexGuard<'static, CursorManager>> {
    CURSOR_MANAGER.get()?.lock().ok()
}

pub fn set_cursor_for_tool(tool_index: i32) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(tool) = ToolCursor::from_tool_index(tool_index) {
        if let Some(mut mgr) = get_cursor_manager_lock() {
            mgr.set_tool(tool)?;
        }
    }
    Ok(())
}

pub fn set_timeline_bounds(x: f32, y: f32, w: f32, h: f32) {
    CursorManager::set_timeline_bounds(x, y, w, h);
}

pub fn reset_cursor_to_default() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mut mgr) = get_cursor_manager_lock() {
        mgr.reset_to_default()?;
    }
    Ok(())
}

pub fn set_cursor_window_handle(hwnd: *mut std::ffi::c_void) {
    #[cfg(windows)]
    {
        if let Some(mut mgr) = get_cursor_manager_lock() {
            mgr.set_window_handle(HWND(hwnd));
        }
    }
}
