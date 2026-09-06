use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HCURSOR, IMAGE_CURSOR, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadImageW, SetCursor, LoadCursorW, IDC_ARROW,
    SetClassLongPtrW, GCLP_HCURSOR, CreateCursor,
};

// Wrapper para HWND que implementa Send + Sync (HWND é apenas um ponteiro opaco).
#[derive(Copy, Clone)]
struct WindowHandle(HWND);
unsafe impl Send for WindowHandle {}
unsafe impl Sync for WindowHandle {}

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
    gain_cursor: Option<CursorHandle>,
    gain_dark_cursor: Option<CursorHandle>,
    hand_cursor: Option<CursorHandle>,
    hand_dark_cursor: Option<CursorHandle>,
    current_tool: Tool,
    dark_mode: bool,
    hwnd: Option<WindowHandle>,
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

        let gain_cursor = Self::load_cursor(&cursor_dir, "gain.cur");
        let gain_dark_cursor = Self::load_cursor(&cursor_dir, "gain-dark.cur");
        let hand_cursor = Self::load_cursor(&cursor_dir, "hand.cur");
        let hand_dark_cursor = Self::load_cursor(&cursor_dir, "hand-dark.cur");

        Ok(Self {
            cursors,
            dark_cursors,
            gain_cursor,
            gain_dark_cursor,
            hand_cursor,
            hand_dark_cursor,
            current_tool: Tool::Select,
            dark_mode: false,
            hwnd: None,
        })
    }

    pub fn set_hwnd(&mut self, hwnd: HWND) {
        self.hwnd = Some(WindowHandle(hwnd));
    }

    fn apply_cursor(&self) {
        if let Some(hwnd) = self.hwnd {
            let map = if self.dark_mode { &self.dark_cursors } else { &self.cursors };
            if let Some(handle) = map.get(&self.current_tool) {
                // Define o cursor na CLASSE da janela (GCLP_HCURSOR) — persistente,
                // evita o flash do cursor do SO entre WM_SETCURSOR.
                unsafe {
                    SetClassLongPtrW(hwnd.0, GCLP_HCURSOR, handle.hcursor as isize);
                }
                // Também força imediato para a janela atual.
                unsafe { SetCursor(handle.hcursor); }
            }
        } else {
            // Fallback sem HWND: apenas SetCursor (pode piscar).
            let map = if self.dark_mode { &self.dark_cursors } else { &self.cursors };
            if let Some(handle) = map.get(&self.current_tool) {
                unsafe { SetCursor(handle.hcursor); }
            }
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.current_tool = tool;
        self.apply_cursor();
    }

    /// Aplica um cursor override (gain/hand) de forma PERSISTENTE, além do
/// SetCursor imediato. Sem o GCLP_HCURSOR, o WM_SETCURSOR combinado com o
/// movimento do mouse restauraria o cursor da ferramenta a cada frame.
    fn apply_override_cursor(&self, hcursor: HCURSOR) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                SetClassLongPtrW(hwnd.0, GCLP_HCURSOR, hcursor as isize);
                SetCursor(hcursor);
            }
        } else {
            unsafe { SetCursor(hcursor); }
        }
    }

    /// Mostra o cursor de ganho (area amarela do fade) ou volta ao cursor da ferramenta.
    pub fn set_gain_override(&mut self, active: bool) {
        if active {
            let handle = if self.dark_mode { self.gain_dark_cursor } else { self.gain_cursor };
            if let Some(handle) = handle {
                self.apply_override_cursor(handle.hcursor);
                return;
            }
        }
        self.apply_cursor();
    }

    /// Mostra o cursor de mão (handles de fade) ou volta ao cursor da ferramenta.
    pub fn set_hand_override(&mut self, active: bool) {
        if active {
            let handle = if self.dark_mode { self.hand_dark_cursor } else { self.hand_cursor };
            if let Some(handle) = handle {
                self.apply_override_cursor(handle.hcursor);
                return;
            }
        }
        self.apply_cursor();
    }

    pub fn current_tool(&self) -> Tool {
        self.current_tool
    }

    pub fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = dark;
        self.apply_cursor();
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
            // Fallback: sintetiza um cursor a partir de um desenho interno.
            let syn = match file_name {
                "gain.cur" => unsafe { synthesize_gain_cursor() },
                "hand.cur" => unsafe { synthesize_hand_cursor() },
                _ => std::ptr::null_mut(),
            };
            if syn.is_null() {
                eprintln!("[tool] CreateCursor sintetizado falhou para {}", file_name);
                return None;
            }
            eprintln!("[tool] cursor sintetizado usado: {}", file_name);
            Some(CursorHandle { hcursor: syn })
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

pub fn set_hwnd(hwnd: HWND) {
    if let Some(mut guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_mut() {
            mgr.set_hwnd(hwnd);
        }
    }
}

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

pub fn set_gain_override(active: bool) {
    if let Some(mut guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_mut() {
            mgr.set_gain_override(active);
        }
    }
}

pub fn set_hand_override(active: bool) {
    if let Some(mut guard) = get_tool_manager() {
        if let Some(mgr) = guard.as_mut() {
            mgr.set_hand_override(active);
        }
    }
}

// ============================================================================
// Síntese de cursores (fallback quando gain.cur/hand.cur não carregam)
// ============================================================================
// Cria um HCURSOR a partir de um desenho em monochrome (AND/XOR masks),
// usado quando o arquivo .cur correspondente está corrompido/ausente.

/// Tamanho do canvas dos cursores sintetizados (quadrado).
const SYNTH_SIZE: u32 = 32;

/// Gera as AND/XOR masks 1bpp a partir de uma flag opaca (pixels `true` são opacos).
/// Inverte-se a smiles para AND mask (0 = desenhar, 1 = manter fundo).
fn synth_masks(opaque: &[bool]) -> (Vec<u8>, Vec<u8>) {
    let w = SYNTH_SIZE as usize;
    let h = SYNTH_SIZE as usize;
    let scan_bytes = (w + 7) / 8; // 1bpp: 4 bytes p/ 32px
    let mut and = vec![0xFFu8; scan_bytes * h];
    // XOR mask é irrelevante aqui porque o AND mask já recorta o formato.
    let xor = vec![0u8; scan_bytes * h];

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if opaque[idx] {
                // Opaco: AND bit = 0 → desenha a cor (XOR preta = preto sólido).
                let byte = y * scan_bytes + x / 8;
                and[byte] &= !(0x80 >> (x % 8));
            }
        }
    }
    (and, xor)
}

/// Gera um cursor de "mão" (open hand) com palma e dedos em preto.
unsafe fn synthesize_hand_cursor() -> HCURSOR {
    let w = SYNTH_SIZE as usize;
    let h = SYNTH_SIZE as usize;
    let mut opaque = vec![false; w * h];

    // Hotspot na palma (aproximadamente).
    let hx = 10i32;
    let hy = 9i32;

    // Desenha uma mão aberta simples (aproximação por discos/retângulos).
    let set_rect = |op: &mut Vec<bool>, x0: i32, y0: i32, x1: i32, y1: i32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    op[(y as usize) * w + (x as usize)] = true;
                }
            }
        }
    };
    let fill_disk = |op: &mut Vec<bool>, cx: i32, cy: i32, r: i32| {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let x = cx + dx;
                    let y = cy + dy;
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        op[(y as usize) * w + (x as usize)] = true;
                    }
                }
            }
        }
    };

    // Palma (arredondada).
    fill_disk(&mut opaque, 10, 12, 7);
    // Dedos (4 verticais).
    fill_disk(&mut opaque, 12, 3, 3);
    fill_disk(&mut opaque, 16, 2, 3);
    fill_disk(&mut opaque, 20, 3, 3);
    fill_disk(&mut opaque, 24, 6, 3);
    // Polegar.
    fill_disk(&mut opaque, 4, 10, 3);
    // Pulsa/mão para cima.
    set_rect(&mut opaque, 7, 14, 13, 26);

    let (and, xor) = synth_masks(&opaque);
    unsafe {
        CreateCursor(
            std::ptr::null_mut() as HINSTANCE,
            hx,
            hy,
            w as i32,
            h as i32,
            and.as_ptr() as *const std::ffi::c_void,
            xor.as_ptr() as *const std::ffi::c_void,
        )
    }
}

/// Gera um cursor de "ganho": seta apontando para cima com uma linha dupla
/// (símbolo de volume/ganho).
unsafe fn synthesize_gain_cursor() -> HCURSOR {
    let w = SYNTH_SIZE as usize;
    let h = SYNTH_SIZE as usize;
    let mut opaque = vec![false; w * h];

    // Seta apontando para cima (clássico "up arrow" de ganho).
    let set_px = |op: &mut Vec<bool>, x: i32, y: i32| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            op[(y as usize) * w + (x as usize)] = true;
        }
    };
    // Corpo da seta: do centro subindo até o topo.
    for i in 0..26i32 {
        set_px(&mut opaque, 7, 5 + i); // borda esq
        set_px(&mut opaque, 9, 5 + i); // centro
        set_px(&mut opaque, 11, 5 + i); // borda dir
    }
    // Ponta triangular no topo.
    for i in 0..6i32 {
        for j in (-i)..=i {
            set_px(&mut opaque, 9 + j, 5 + i);
        }
    }
    // Hotspot no centro do corpo da seta.
    let hx = 9i32;
    let hy = 12i32;

    let (and, xor) = synth_masks(&opaque);
    unsafe {
        CreateCursor(
            std::ptr::null_mut() as HINSTANCE,
            hx,
            hy,
            w as i32,
            h as i32,
            and.as_ptr() as *const std::ffi::c_void,
            xor.as_ptr() as *const std::ffi::c_void,
        )
    }
}