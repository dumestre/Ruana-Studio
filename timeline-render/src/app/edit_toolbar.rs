//! Edit toolbar (wgpu): barra de ferramentas do piano roll.
//!
//! Reimplementação nativa do `EditToolbar` do Slint (`ui/edit_toolbar.slint`):
//! retângulos via pipeline principal + ícones texturizados via `icon.wgsl`.
//! Visível apenas quando o piano roll está aberto, numa faixa entre a
//! `time_ruler` e a `piano_ruler`.
//!
//! Ícones (SVG em `ui/icons/`) são rasterizados em build-time num atlas 4x3
//! (`$OUT_DIR/icons_atlas.rs`); a ordem do atlas é a ordem do EditToolbar:
//! Select, Slice, Glue, Stretch, Draw, Erase (normal + filled).

use crate::app::layout::Rect;
use crate::app::render::{palette, rgba, srgb_to_linear};
use crate::tools::{current_tool, Tool};
use crate::RectInstance;

/// Atlas de ícones gerado em build-time pelo `build.rs`.
mod atlas {
    include!(concat!(env!("OUT_DIR"), "/icons_atlas.rs"));
}

pub use atlas::{
    ICON_ATLAS_H, ICON_ATLAS_RGBA, ICON_ATLAS_W, ICON_CELL, ICON_COLS,
};

/// Altura (px) da faixa da edit toolbar no layout.
pub const TOOLBAR_H: f32 = 24.0;
/// Lado (px) de cada botão.
pub const BUTTON_SIZE: f32 = 18.0;
/// Espaço entre botões.
pub const BUTTON_GAP: f32 = 2.0;
/// Tamanho (px) do ícone dentro do botão.
pub const ICON_SIZE: f32 = 16.0;

/// Ordem das ferramentas na toolbar (igual ao EditToolbar do Slint) — difere
/// da ordem do enum `Tool` (usado pelas teclas 1..=6).
pub const TOOL_INDEX: [Tool; 6] = [
    Tool::Select,  // 0
    Tool::Slice,   // 1
    Tool::Glue,    // 2
    Tool::Stretch, // 3
    Tool::Draw,    // 4
    Tool::Erase,   // 5
];

/// Ferramenta correspondente ao índice da toolbar (0..5).
pub fn tool_from_index(idx: usize) -> Option<Tool> {
    TOOL_INDEX.get(idx).copied()
}

/// Índice (0..5) da toolbar para uma ferramenta.
#[allow(dead_code)]
pub fn index_of_tool(t: Tool) -> Option<usize> {
    TOOL_INDEX.iter().position(|&x| x == t)
}

/// Índices dos ícones no atlas (ordem do build.rs).
pub const ICON_SELECT: u32 = 0;
pub const ICON_SELECT_FILLED: u32 = 1;
pub const ICON_SLICE: u32 = 2;
pub const ICON_SLICE_FILLED: u32 = 3;
pub const ICON_GLUE: u32 = 4;
pub const ICON_GLUE_FILLED: u32 = 5;
pub const ICON_STRETCH: u32 = 6;
pub const ICON_STRETCH_FILLED: u32 = 7;
pub const ICON_DRAW: u32 = 8;
pub const ICON_DRAW_FILLED: u32 = 9;
pub const ICON_ERASE: u32 = 10;
pub const ICON_ERASE_FILLED: u32 = 11;

/// Par (normal, filled) de cada botão, na ordem da toolbar.
const BTN_ICONS: [(u32, u32); 6] = [
    (ICON_SELECT, ICON_SELECT_FILLED),
    (ICON_SLICE, ICON_SLICE_FILLED),
    (ICON_GLUE, ICON_GLUE_FILLED),
    (ICON_STRETCH, ICON_STRETCH_FILLED),
    (ICON_DRAW, ICON_DRAW_FILLED),
    (ICON_ERASE, ICON_ERASE_FILLED),
];

/// Cor de fundo do botão ativo (equivalente a `Theme.crimson`).
pub const CRIMSON: (u8, u8, u8) = (0xDC, 0x14, 0x3C);
/// Cor do ícone sobre o botão ativo (equivalente a `Theme.on-crimson`).
pub const ON_CRIMSON: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);

/// Instância de quad texturizado (mesmo layout do `icon.wgsl`).
#[derive(Clone, Copy, Debug)]
pub struct IconInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// uv_min (normalizado).
    pub u: f32,
    pub v: f32,
    /// uv_size (normalizado).
    pub du: f32,
    pub dv: f32,
    /// Tint linear (equivale ao `colorize` do Slint).
    pub tint: [f32; 4],
}

impl IconInstance {
    pub fn to_f32(self) -> [f32; 12] {
        [
            self.x, self.y, self.w, self.h,
            self.u, self.v, self.du, self.dv,
            self.tint[0], self.tint[1], self.tint[2], self.tint[3],
        ]
    }
}

/// Retângulo UV (normalizado) de um ícone no atlas.
fn uv_for(icon: u32) -> (f32, f32, f32, f32) {
    let w = ICON_ATLAS_W;
    let h = ICON_ATLAS_H;
    let cell = ICON_CELL;
    let cols = ICON_COLS;
    let col = icon % cols;
    let row = icon / cols;
    let u = (col * cell) as f32 / w as f32;
    let v = (row * cell) as f32 / h as f32;
    let du = cell as f32 / w as f32;
    let dv = cell as f32 / h as f32;
    (u, v, du, dv)
}

/// Rects dos 6 botões, centralizados na faixa `strip`.
pub fn button_rects(strip: Rect) -> [(f32, f32, f32, f32); 6] {
    let total_w = 6.0 * BUTTON_SIZE + 5.0 * BUTTON_GAP;
    let start_x = strip.x + (strip.width - total_w) / 2.0;
    let y = strip.y + (strip.height - BUTTON_SIZE) / 2.0;

    let mut out = [(0.0f32, 0.0, 0.0, 0.0); 6];
    for (i, r) in out.iter_mut().enumerate() {
        let x = start_x + i as f32 * (BUTTON_SIZE + BUTTON_GAP);
        *r = (x, y, BUTTON_SIZE, BUTTON_SIZE);
    }
    out
}

impl crate::App {
    /// Hit-test do cursor sobre os botões da toolbar (None = não está na faixa).
    pub fn toolbar_hit(&self, width: f32, height: f32, cursor: (f32, f32)) -> Option<usize> {
        if !self.show_piano_roll {
            return None;
        }
        let strip = self.layout(width, height).edit_toolbar?;
        if !strip.contains(cursor.0, cursor.1) {
            return None;
        }
        let rects = button_rects(strip);
        for (i, (bx, by, bw, bh)) in rects.iter().enumerate() {
            if cursor.0 >= *bx
                && cursor.0 <= bx + bw
                && cursor.1 >= *by
                && cursor.1 <= by + bh
            {
                return Some(i);
            }
        }
        None
    }

    /// Retângulos da toolbar (fundo + botões) para o pipeline principal.
    pub fn edit_toolbar_rect_instances(&self, width: f32, height: f32) -> Vec<RectInstance> {
        if !self.show_piano_roll {
            return Vec::new();
        }
        let strip = self.layout(width, height)
            .edit_toolbar
            .unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        if strip.height <= 0.0 {
            return Vec::new();
        }
        let t = self.theme.clamp(0, 6) as usize;
        let cur = current_tool();
        let mut out = Vec::new();

        // Fundo da faixa — mais escuro que a time_ruler para os botões
        // (surface-high) se destacarem.
        out.push(RectInstance {
            x: strip.x, y: strip.y,
            w: strip.width, h: strip.height,
            color: rgba(palette::SURFACE_STEP[t], 255),
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Borda inferior (separa do splitter do piano roll).
        out.push(RectInstance {
            x: strip.x, y: strip.y + strip.height - 1.0,
            w: strip.width, h: 1.0,
            color: rgba(palette::BASE[t], 255),
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        let rects = button_rects(strip);
        for (i, (bx, by, bw, bh)) in rects.iter().enumerate() {
            let toggled = cur == TOOL_INDEX[i];
            let bg = if toggled { CRIMSON } else { palette::SURFACE_HIGH[t] };
            out.push(RectInstance {
                x: *bx, y: *by, w: *bw, h: *bh,
                color: [bg.0, bg.1, bg.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 2.0,
            });
        }

        out
    }

    /// Ícones texturizados da toolbar para o pipeline de ícones.
    pub fn edit_toolbar_icon_instances(&self, width: f32, height: f32) -> Vec<IconInstance> {
        if !self.show_piano_roll || ICON_ATLAS_RGBA.is_empty() {
            return Vec::new();
        }
        let strip = self.layout(width, height)
            .edit_toolbar
            .unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        if strip.height <= 0.0 {
            return Vec::new();
        }
        let t = self.theme.clamp(0, 6) as usize;
        let cur = current_tool();
        let rects = button_rects(strip);
        let mut out = Vec::new();

        for (i, (bx, by, bw, bh)) in rects.iter().enumerate() {
            let toggled = cur == TOOL_INDEX[i];
            let (icon_normal, icon_filled) = BTN_ICONS[i];
            let icon = if toggled { icon_filled } else { icon_normal };
            let (u, v, du, dv) = uv_for(icon);

            let base: (u8, u8, u8) = if toggled {
                ON_CRIMSON
            } else if self.toolbar_hover == Some(i) {
                palette::ACCENT[t]
            } else {
                palette::TEXT_PRIMARY[t]
            };
            let tint = [
                srgb_to_linear(base.0 as f32 / 255.0),
                srgb_to_linear(base.1 as f32 / 255.0),
                srgb_to_linear(base.2 as f32 / 255.0),
                1.0,
            ];

            out.push(IconInstance {
                x: bx + (bw - ICON_SIZE) / 2.0,
                y: by + (bh - ICON_SIZE) / 2.0,
                w: ICON_SIZE,
                h: ICON_SIZE,
                u, v, du, dv,
                tint,
            });
        }

        out
    }
}
