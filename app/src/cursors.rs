use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use windows::Win32::Foundation::{HINSTANCE, HWND, LRESULT, POINT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::ScreenToClient;
#[cfg(windows)]
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HCURSOR, IDC_ARROW, IMAGE_CURSOR, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadCursorW,
    LoadImageW, SetCursor, WM_SETCURSOR,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCursor {
    Select = 0,
    Slice = 1,
    Glue = 2,
    Stretch = 3,
    Draw = 4,
    Erase = 5,
}

impl ToolCursor {
    pub fn from_tool_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(ToolCursor::Select),
            1 => Some(ToolCursor::Slice),
            2 => Some(ToolCursor::Glue),
            3 => Some(ToolCursor::Stretch),
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
            ToolCursor::Stretch => "stretch.cur",
            ToolCursor::Draw => "draw.cur",
            ToolCursor::Erase => "erase.cur",
        }
    }

    /// Variante escura (modo Paper) — mesmo nome com sufixo `-dark`.
    pub fn dark_cursor_file_name(&self) -> &'static str {
        match self {
            ToolCursor::Select => "select-dark.cur",
            ToolCursor::Slice => "slice-dark.cur",
            ToolCursor::Glue => "glue-dark.cur",
            ToolCursor::Stretch => "stretch-dark.cur",
            ToolCursor::Draw => "draw-dark.cur",
            ToolCursor::Erase => "erase-dark.cur",
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
    #[cfg(windows)]
    dark_cursors: HashMap<ToolCursor, CursorHandle>,
    current_tool: Option<ToolCursor>,
    dark: bool,
}

impl CursorManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(windows)]
        {
            let cursor_dir = Self::cursor_directory()?;
            let all = [
                ToolCursor::Select,
                ToolCursor::Slice,
                ToolCursor::Glue,
                ToolCursor::Stretch,
                ToolCursor::Draw,
                ToolCursor::Erase,
            ];

            let mut cursors = HashMap::new();
            let mut dark_cursors = HashMap::new();

            for tool in all {
                if let Some(h) = Self::load_cursor(&cursor_dir, tool.cursor_file_name()) {
                    cursors.insert(tool, h);
                }
                if let Some(h) = Self::load_cursor(&cursor_dir, tool.dark_cursor_file_name()) {
                    dark_cursors.insert(tool, h);
                }
            }

            Ok(Self {
                cursors,
                dark_cursors,
                current_tool: None,
                dark: false,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                current_tool: None,
                dark: false,
            })
        }
    }

    #[cfg(windows)]
    fn load_cursor(
        cursor_dir: &std::path::Path,
        file_name: &str,
    ) -> Option<CursorHandle> {
        let cur_path = cursor_dir.join(file_name);
        if !cur_path.exists() {
            eprintln!("[cursor] arquivo nao encontrado: {}", cur_path.display());
            return None;
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

        hcursor.ok().map(|h| CursorHandle {
            hcursor: HCURSOR(h.0),
        })
    }

    #[cfg(windows)]
    fn cursor_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_dir = exe_path.parent().ok_or("No parent directory")?;

        // Tenta exe_dir/cursors (layout padrão do build.rs).
        // Fallbacks para debug/release quando o exe é movido ou rodado via cargo.
        let candidates = [
            exe_dir.join("cursors"),
            exe_dir.join("../cursors"),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("app/cursors"),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("cursors"),
        ];
        for cand in &candidates {
            if cand.exists() && cand.is_dir() {
                let has_any = std::fs::read_dir(cand)
                    .map(|mut it| it.next().is_some())
                    .unwrap_or(false);
                if has_any {
                    eprintln!("[cursor] usando diretorio: {}", cand.display());
                    return Ok(cand.clone());
                }
            }
        }
        // Fallback final: exe_dir/cursors mesmo que vazio — load_cursor vai falhar com log.
        eprintln!("[cursor] diretorio nao encontrado, usando {}", exe_dir.join("cursors").display());
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
            self.apply_tool_cursor(tool);
        }
        self.current_tool = Some(tool);
        Ok(())
    }

    /// Alterna entre variantes claras (temas escuros) e escuras (Paper).
    /// Reaplica o cursor da ferramenta atual quando houver uma ativa.
    pub fn set_dark(&mut self, dark: bool) -> Result<(), Box<dyn std::error::Error>> {
        self.dark = dark;
        #[cfg(windows)]
        {
            if let Some(tool) = self.current_tool {
                self.apply_tool_cursor(tool);
            }
        }
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

    #[cfg(windows)]
    fn apply_tool_cursor(&self, tool: ToolCursor) {
        let map = if self.dark {
            &self.dark_cursors
        } else {
            &self.cursors
        };
        if let Some(cursor_handle) = map.get(&tool) {
            if let Some(mut active) = ACTIVE_CURSOR.get().and_then(|m| m.lock().ok()) {
                *active = Some(cursor_handle.hcursor.0 as isize);
            }
            unsafe {
                SetCursor(cursor_handle.hcursor);
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
                unsafe {
                    SetCursor(HCURSOR(default.0));
                }
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
        Self {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            valid: false,
        }
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
            if let Some(active) = ACTIVE_CURSOR
                .get()
                .and_then(|m| m.lock().ok())
                .and_then(|c| *c)
            {
                unsafe { SetCursor(HCURSOR(active as *mut core::ffi::c_void)) };
                return LRESULT(1);
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

pub fn init_cursor_manager() -> Result<(), Box<dyn std::error::Error>> {
    let _ = CURSOR_MANAGER
        .get_or_init(|| Mutex::new(CursorManager::new().expect("Failed to create CursorManager")));
    Ok(())
}

fn get_cursor_manager_lock() -> Option<std::sync::MutexGuard<'static, CursorManager>> {
    CURSOR_MANAGER.get()?.lock().ok()
}

pub fn set_cursor_for_tool(tool_index: i32) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(tool) = ToolCursor::from_tool_index(tool_index) {
        if let Some(mut mgr) = get_cursor_manager_lock() {
            eprintln!("[cursor] set_cursor_for_tool {} -> {:?}", tool_index, tool);
            mgr.set_tool(tool)?;
        } else {
            eprintln!("[cursor] manager nao inicializado para tool {}", tool_index);
        }
    } else {
        eprintln!("[cursor] tool_index {} sem cursor mapeado", tool_index);
        // Ferramenta sem cursor customizado: volta ao cursor padrão
        if let Some(mut mgr) = get_cursor_manager_lock() {
            let _ = mgr.reset_to_default();
        }
    }
    Ok(())
}

/// Ativa a variante escura dos cursores (usada no tema Paper).
pub fn set_dark_cursors(dark: bool) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mut mgr) = get_cursor_manager_lock() {
        mgr.set_dark(dark)?;
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
