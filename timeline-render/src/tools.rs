use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::HINSTANCE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HCURSOR, IMAGE_CURSOR, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadImageW, SetCursor, LoadCursorW, IDC_ARROW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Select = 0,
    Draw = 1,
    Stretch = 2,
    Slice = 3,
    Erase = 4,
    Glue = 5,
}

impl Tool {
    #[allow(dead_code)]
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Tool::Select),
            1 => Some(Tool::Draw),
            2 => Some(Tool::Stretch),
            3 => Some(Tool::Slice),
            4 => Some(Tool::Erase),
            5 => Some(Tool::Glue),
            _ => None,
        }
    }

    pub fn cursor_file_name(&self) -> &'static str {
        match self {
            Tool::Select => "select.cur",
            Tool::Draw => "draw.cur",
            Tool::Stretch => "stretch.cur",
            Tool::Slice => "slice.cur",
            Tool::Erase => "erase.cur",
            Tool::Glue => "glue.cur",
        }
    }

    pub fn dark_cursor_file_name(&self) -> &'static str {
        match self {
            Tool::Select => "select-dark.cur",
            Tool::Draw => "draw-dark.cur",
            Tool::Stretch => "stretch-dark.cur",
            Tool::Slice => "slice-dark.cur",
            Tool::Erase => "erase-dark.cur",
            Tool::Glue => "glue-dark.cur",
        }
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Draw => "Draw (Pencil)",
            Tool::Stretch => "Stretch",
            Tool::Slice => "Slice",
            Tool::Erase => "Erase",
            Tool::Glue => "Glue",
        }
    }
}

#[derive(Copy, Clone)]
struct CursorHandle {
    hcursor: HCURSOR,
}

unsafe impl Send for CursorHandle {}
unsafe impl Sync for CursorHandle {}

pub struct ToolManager {
    cursors: HashMap<Tool, CursorHandle>,
    dark_cursors: HashMap<Tool, CursorHandle>,
    current_tool: Tool,
    dark_mode: bool,
}

impl ToolManager {
    pub fn new() -> Result<Self, String> {
        let cursor_dir = Self::cursor_directory()?;
        let all_tools = [
            Tool::Select,
            Tool::Draw,
            Tool::Stretch,
            Tool::Slice,
            Tool::Erase,
            Tool::Glue,
        ];

        let mut cursors = HashMap::new();
        let mut dark_cursors = HashMap::new();

        for tool in all_tools {
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
            current_tool: Tool::Select,
            dark_mode: false,
        })
    }

    fn load_cursor(cursor_dir: &PathBuf, file_name: &str) -> Option<CursorHandle> {
        let cur_path = cursor_dir.join(file_name);
        if !cur_path.exists() {
            eprintln!("[tool] cursor file not found: {}", cur_path.display());
            return None;
        }

        let path_wide: Vec<u16> = cur_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let hcursor = unsafe {
            LoadImageW(
                0 as HINSTANCE,
                windows_sys::core::PCWSTR::from(path_wide.as_ptr()),
                IMAGE_CURSOR,
                0,
                0,
                LR_DEFAULTSIZE | LR_LOADFROMFILE,
            )
        };

        if hcursor.is_null() {
            eprintln!("[tool] LoadImageW failed for: {}", cur_path.display());
            None
        } else {
            Some(CursorHandle { hcursor })
        }
    }

    fn cursor_directory() -> Result<PathBuf, String> {
        let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_dir = exe_path.parent().ok_or("No parent directory")?;

        let candidates = [
            exe_dir.join("cursors"),
            exe_dir.join("../cursors"),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("cursors"),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("timeline-render/cursors"),
        ];

        for cand in &candidates {
            if cand.exists() && cand.is_dir() {
                let has_any = std::fs::read_dir(cand)
                    .map(|mut it| it.next().is_some())
                    .unwrap_or(false);
                if has_any {
                    eprintln!("[tool] using cursor directory: {}", cand.display());
                    return Ok(cand.clone());
                }
            }
        }

        eprintln!("[tool] cursor directory not found, using {}", exe_dir.join("cursors").display());
        Ok(exe_dir.join("cursors"))
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.current_tool = tool;
        self.apply_cursor();
    }

    pub fn current_tool(&self) -> Tool {
        self.current_tool
    }

    pub fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = dark;
        self.apply_cursor();
    }

    fn apply_cursor(&self) {
        let map = if self.dark_mode { &self.dark_cursors } else { &self.cursors };
        if let Some(handle) = map.get(&self.current_tool) {
            unsafe { SetCursor(handle.hcursor); }
        }
    }

    #[allow(dead_code)]
    pub fn reset_to_default(&self) {
        let default = unsafe { LoadCursorW(core::ptr::null_mut(), IDC_ARROW as usize as *const u16) };
        if !default.is_null() {
            unsafe { SetCursor(default); }
        }
    }

    pub fn update_cursor(&self) {
        self.apply_cursor();
    }
}

static TOOL_MANAGER: OnceLock<Mutex<Option<ToolManager>>> = OnceLock::new();

pub fn init_tool_manager() -> Result<(), String> {
    let mgr = ToolManager::new()?;
    let _ = TOOL_MANAGER.get_or_init(|| Mutex::new(Some(mgr)));
    Ok(())
}

fn get_tool_manager() -> Option<std::sync::MutexGuard<'static, Option<ToolManager>>> {
    TOOL_MANAGER.get()?.lock().ok()
}

pub fn set_tool(tool: Tool) {
    if let Some(mut guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_mut() {
            mgr.set_tool(tool);
        }
    }
}

pub fn current_tool() -> Tool {
    if let Some(guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_ref() {
            return mgr.current_tool();
        }
    }
    Tool::Select
}

pub fn set_dark_mode(dark: bool) {
    if let Some(mut guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_mut() {
            mgr.set_dark_mode(dark);
        }
    }
}

pub fn update_cursor() {
    if let Some(guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_ref() {
            mgr.update_cursor();
        }
    }
}

/// Força o cursor visual sem mudar a ferramenta atual (usado para hover de seam).
pub fn set_cursor_override(tool: Tool) {
    if let Some(guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_ref() {
            let map = if mgr.dark_mode { &mgr.dark_cursors } else { &mgr.cursors };
            if let Some(handle) = map.get(&tool) {
                unsafe { SetCursor(handle.hcursor); }
            }
        }
    }
}