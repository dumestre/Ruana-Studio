//! Renderização da timeline: paleta, helpers de cor, fontes/texto,
//! instâncias de retângulos e o `render`/`build_instances` do `App`.

use ab_glyph::{Font, ScaleFont};

use wgpu::util::DeviceExt;

use crate::clips;
use crate::fades;
use crate::tools::{current_tool, Tool};
use crate::App;

// Paleta e tipo ----------------------------------------------------------------

pub mod palette {
    pub type Rgb = (u8, u8, u8);

    // Theme.base — cor usada para o grid da régua e da timeline.
    pub const BASE: [Rgb; 7] = [
        (0x31, 0x31, 0x31), (0x16, 0x1A, 0x21), (0x19, 0x17, 0x22),
        (0x14, 0x16, 0x0E), (0x21, 0x1A, 0x12), (0x0F, 0x1E, 0x21),
        (0x64, 0x64, 0x64),
    ];

    // Theme.accent — seleção/clips selecionados.
    pub const ACCENT: [Rgb; 7] = [
        (0x00, 0xC8, 0x53), (0x4F, 0xA8, 0xFF), (0xB8, 0x5C, 0xFF),
        (0xA8, 0xFF, 0x00), (0xFF, 0x9F, 0x43), (0x2E, 0xC4, 0xB6),
        (0x00, 0xC8, 0x53),
    ];

    pub const SURFACE_HIGH: [Rgb; 7] = [
        (0x55, 0x55, 0x55), (0x26, 0x30, 0x3D), (0x2D, 0x27, 0x40),
        (0x26, 0x2A, 0x1D), (0x36, 0x2A, 0x1B), (0x1E, 0x30, 0x35),
        (0x91, 0x91, 0x91),
    ];
    pub const SURFACE_STEP: [Rgb; 7] = [
        (0x42, 0x42, 0x42), (0x23, 0x2D, 0x38), (0x27, 0x23, 0x34),
        (0x20, 0x23, 0x1A), (0x30, 0x26, 0x1F), (0x19, 0x2C, 0x31),
        (0x7A, 0x7A, 0x7A),
    ];
    pub const GRID_LINE: [Rgb; 7] = [
        (0x75, 0x75, 0x75), (0x4A, 0x5A, 0x70), (0x4A, 0x44, 0x66),
        (0x4A, 0x4D, 0x3A), (0x5A, 0x4E, 0x3A), (0x3A, 0x5C, 0x66),
        (0xD6, 0xD6, 0xD6),
    ];
    pub const GRID_SUB: [Rgb; 7] = [
        (0x45, 0x45, 0x45), (0x2A, 0x36, 0x45), (0x2F, 0x2C, 0x42),
        (0x2A, 0x2D, 0x24), (0x3A, 0x30, 0x28), (0x21, 0x36, 0x3C),
        (0xEF, 0xEF, 0xEF),
    ];
    pub const TIMELINE_BG: [Rgb; 7] = [
        (0x69, 0x69, 0x69), (0x1E, 0x2C, 0x35), (0x1F, 0x1B, 0x28),
        (0x18, 0x1B, 0x11), (0x25, 0x21, 0x19), (0x12, 0x26, 0x28),
        (0x79, 0x79, 0x79),
    ];
    pub const CLIP_PALETTE: [Rgb; 20] = [
        (0x6F, 0xA8, 0x6B), (0x4F, 0x8A, 0xE0), (0xE0, 0xA9, 0x4F),
        (0xE0, 0x5E, 0x5E), (0xA0, 0x60, 0xD8), (0xE0, 0x5E, 0xAA),
        (0x3E, 0xC7, 0xB0), (0xC0, 0xC0, 0x50), (0xE0, 0x7A, 0x3E),
        (0x8F, 0xD1, 0x4F), (0x3F, 0xA3, 0x4D), (0x35, 0xB5, 0xE5),
        (0x5A, 0x8F, 0xE0), (0x55, 0x60, 0xD8), (0x9B, 0x5C, 0xE0),
        (0xD8, 0x4F, 0xB2), (0xE0, 0x4F, 0x6F), (0xA8, 0x74, 0x4A),
        (0xD8, 0xC0, 0x4F), (0x7A, 0x93, 0xA8),
    ];
    pub const CLIP_MIDI_DEFAULT: [Rgb; 7] = [
        (0x6F, 0xA8, 0x6B), (0x6F, 0xA8, 0x6B), (0x6F, 0xA8, 0x6B),
        (0x8F, 0xC1, 0x4F), (0x8F, 0xAA, 0x6B), (0x6F, 0xA8, 0x6B),
        (0x52, 0xBA, 0x46),
    ];
    pub const CLIP_AUDIO_DEFAULT: Rgb = (0x00, 0xC8, 0x53);
    pub const TEXT_PRIMARY: [Rgb; 7] = [
        (0xE0, 0xE0, 0xE0), (0xE2, 0xE6, 0xEE), (0xE0, 0xDA, 0xF0),
        (0xD5, 0xE8, 0xB0), (0xE8, 0xD8, 0xC0), (0xC8, 0xE8, 0xE0),
        (0x17, 0x17, 0x17),
    ];
    pub const TEXT_DIM: [Rgb; 7] = [
        (0x9A, 0x9A, 0x9A), (0x84, 0x9A, 0xB5), (0x8F, 0x8A, 0xB0),
        (0x7A, 0x85, 0x9A), (0x9A, 0x94, 0x82), (0x82, 0xC2, 0x92),
        (0x5A, 0x5A, 0x5A),
    ];
    pub const PLAYHEAD_COLOR: Rgb = (0xDC, 0x14, 0x3C);
}

pub fn rgba((r, g, b): palette::Rgb, a: u8) -> [u8; 4] { [r, g, b, a] }

pub fn darker((r, g, b): palette::Rgb, amount: f32) -> palette::Rgb {
    let f = (1.0 - amount).max(0.0);
    (
        (r as f32 * f).round() as u8,
        (g as f32 * f).round() as u8,
        (b as f32 * f).round() as u8,
    )
}

pub fn rgba_alpha((r, g, b): palette::Rgb, alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    [r, g, b, a]
}

/// Converte um canal de cor sRGB (0..1, como vem das paletas hex) para o
/// espaço LINEAR que o fragment shader entrega ao alvo sRGB. Sem isso a cor
/// chega "lavada" na tela e o header/UI parecem translúcidos, quando na
/// verdade têm alpha 255.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// Fontes e texto ---------------------------------------------------------------

static FONT: std::sync::OnceLock<ab_glyph::FontRef<'static>> = std::sync::OnceLock::new();
static FONT_REGULAR: std::sync::OnceLock<ab_glyph::FontRef<'static>> = std::sync::OnceLock::new();

pub fn get_font() -> &'static ab_glyph::FontRef<'static> {
    FONT.get_or_init(|| {
        ab_glyph::FontRef::try_from_slice(include_bytes!("../../assets/arialbd.ttf"))
            .expect("Failed to load font")
    })
}

/// Fonte Arial regular (não-bold), usada nos nomes dos clips.
pub fn get_regular_font() -> &'static ab_glyph::FontRef<'static> {
    FONT_REGULAR.get_or_init(|| {
        ab_glyph::FontRef::try_from_slice(include_bytes!("../../assets/arial.ttf"))
            .expect("Failed to load font")
    })
}

/// Desenha texto usando a fonte Arial Bold (via ab_glyph), rasterizada a pixels.
/// Cada caractere é rasterizado individualmente e os pixels "on" viram RectInstance.
pub fn build_text_instances(
    text: &str,
    center_x: f32,
    y: f32,
    color: [u8; 4],
    scale: f32,
) -> Vec<RectInstance> {
    build_text_instances_font(get_font(), text, center_x, y, color, scale)
}

/// Retícula de texto com uma fonte específica.
pub fn build_text_instances_font(
    font: &ab_glyph::FontRef<'static>,
    text: &str,
    center_x: f32,
    y: f32,
    color: [u8; 4],
    scale: f32,
) -> Vec<RectInstance> {
    let px_scale = ab_glyph::PxScale::from(scale * 14.0);
    let mut instances = Vec::new();

    // First pass: measure total width for centering
    let sf = font.as_scaled(px_scale);
    let total_width: f32 = text.chars()
        .filter_map(|ch| {
            let gid = font.glyph_id(ch);
            Some(sf.h_advance(gid))
        })
        .sum();

    let mut pen_x = center_x - total_width / 2.0;

    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let advance = sf.h_advance(gid);

        let glyph = font.glyph_id(ch).with_scale_and_position(
            px_scale,
            ab_glyph::point(pen_x, y + sf.ascent() - 2.0),
        );

        if let Some(outlined) = font.outline_glyph(glyph) {
            let b = outlined.px_bounds();
            outlined.draw(|px, py, alpha| {
                if alpha > 0.02 {
                    let a = (color[3] as f32 * alpha).round() as u8;
                    instances.push(RectInstance {
                        x: b.min.x + px as f32,
                        y: b.min.y + py as f32,
                        w: 1.0,
                        h: 1.0,
                        color: [color[0], color[1], color[2], a],
                        wave_offset: 0,
                        wave_count: 0,
                        clip_w: 0.0,
                        clip_h: 0.0,
                        radius: 0.0,
                    });
                }
            });
        }

        pen_x += advance;
    }

    instances
}

/// Instância de render (mesmo layout do shader).
#[derive(Clone, Copy, Debug)]
pub struct RectInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 4],
    pub wave_offset: u32,
    pub wave_count: u32,
    pub clip_w: f32,
    pub clip_h: f32,
    pub radius: f32,
}

impl RectInstance {
    fn to_f32(self) -> [f32; 13] {
        let [r, g, b, a] = self.color;
        [
            self.x, self.y, self.w, self.h,
            srgb_to_linear(r as f32 / 255.0),
            srgb_to_linear(g as f32 / 255.0),
            srgb_to_linear(b as f32 / 255.0),
            a as f32 / 255.0,
            self.wave_offset as f32, self.wave_count as f32,
            self.clip_w, self.clip_h,
            self.radius,
        ]
    }
}

/// Câmera ortográfica 2D.
#[derive(Clone, Copy, Debug)]
pub struct OrthoCamera {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
}

impl OrthoCamera {
    pub fn proj(&self) -> [f32; 16] {
        let sx = self.scroll_x;
        let sy = self.scroll_y;
        let hw = (self.viewport_w / 2.0).max(1.0);
        let hh = (self.viewport_h / 2.0).max(1.0);
        [
            1.0 / hw, 0.0, 0.0, 0.0,
            0.0, -1.0 / hh, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            -sx / hw - 1.0, 1.0 + sy / hh, 0.0, 1.0,
        ]
    }
}

impl App {
    pub fn build_instances(&mut self, width: f32, height: f32) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6) as usize;
        let mut instances = Vec::new();
        let px_per_bar = self.px_per_beat * 4.0;
        let _total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        let row_height = 64.0 * self.zoom_y;

        // Altura efetiva da timeline.
        // Quando o piano roll está visível, a timeline termina na time_ruler
        // (que é empurrada para cima). A splitter fica ENTRE a time_ruler e o piano roll.
        let layout = self.layout(width, height);
        let timeline_bottom = if self.show_piano_roll {
            layout.time_ruler.y
        } else {
            height - self.time_ruler_h
        };
        let timeline_bottom = timeline_bottom.max(self.ruler_px + 4.0);

        // Sincroniza notas MIDI do daw-core (fonte de verdade) para o cache do wgpu
        if self.show_piano_roll {
            self.sync_piano_roll_from_core();
        }

        // Fundo geral - cobre desde a régua até o fim da timeline (não o piano roll)
        instances.push(RectInstance {
            x: 0.0, y: self.ruler_px,
            w: width, h: (timeline_bottom - self.ruler_px).max(0.0),
            color: rgba(palette::TIMELINE_BG[t], 255),
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Régua — com listras alternadas (igualdade das faixas/tracks)
        // Desenha segments alternados começando com SURFACE_STEP (mais escuro)
        let ruler_segment_w = px_per_bar; // uma "coluna" de compasso por segmento
        let n_segments = (width / ruler_segment_w).ceil() as usize + 1;
        for seg in 0..n_segments {
            let seg_x = (seg as f32 * ruler_segment_w) - self.scroll_x;
            let is_odd = seg % 2 == 1;
            let bg_color = if is_odd {
                palette::SURFACE_HIGH[t]   // ímpar: mais claro
            } else {
                palette::SURFACE_STEP[t]   // par: mais escuro (primeiro)
            };
            // Largura do segmento, limitado ao viewport
            let seg_w = ruler_segment_w.min(width - seg_x).max(0.0);
            if seg_w > 0.0 {
                instances.push(RectInstance {
                    x: seg_x, y: 0.0, w: seg_w, h: self.ruler_px,
                    color: [bg_color.0, bg_color.1, bg_color.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Fundos alternados das colunas do grid (antes das linhas do grid)
        let px_per_bar = self.px_per_beat * 4.0;
        let total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let is_odd_col = i % 2 == 1;
            let bg = if is_odd_col {
                palette::SURFACE_HIGH[t]
            } else {
                palette::SURFACE_STEP[t]
            };
            let bw = px_per_bar;
            let bh = (timeline_bottom - self.ruler_px + self.scroll_y).max(0.0);
            if bw > 0.0 && bh > 0.0 {
                instances.push(RectInstance {
                    x: bx, y: self.ruler_px - self.scroll_y, w: bw, h: bh,
                    color: [bg.0, bg.1, bg.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Grid do corpo (abaixo da régua) — rola verticalmente com scroll_y,
        // enquanto a régua permanece fixa no topo.
        let body_top = self.ruler_px - self.scroll_y;
        let body_h = (height - self.ruler_px + self.scroll_y).max(0.0);
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }
            let bar_a = if i % 2 == 1 { 255 } else { 140 };
            instances.push(RectInstance {
                x: bx, y: body_top, w: 1.0, h: body_h,
                color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, bar_a],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });

            // Subdivisões como frações uniformes do compasso (px_per_bar), da
            // mais fina para a mais grossa para que as linhas principais fiquem por cima.
            // 1/64
            if px_per_bar >= 960.0 {
                let n = 64u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 70],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/32
            if px_per_bar >= 480.0 {
                let n = 32u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 90],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/16
            if px_per_bar >= 240.0 {
                let n = 16u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/8
            if px_per_bar >= 120.0 {
                let n = 8u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Batidas (1/4) — mais grossas, por cima das subdivisões
            if px_per_bar >= 40.0 {
                let n = 4u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }

        // Separadores horizontais entre as tracks (row separators). Francem a
        // largura toda do corpo e rolam verticalmente junto com scroll_y.
        let view_bottom = timeline_bottom;
        let max_track = self.clips.iter().map(|c| c.track_index as i32).max().unwrap_or(-1);
        // FICA: sempre ao menos 1 track visível (uma track padrão).
        let track_count = ((max_track + 1).max(1)) as usize;
        for tr in 0..track_count {
            let y = self.ruler_px + (tr as f32 + 1.0) * row_height - self.scroll_y;
            if y < self.ruler_px || y > view_bottom { continue; }
            instances.push(RectInstance {
                x: 0.0, y, w: width, h: 1.0,
                color: rgba(palette::BASE[t], 150),
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
        }

        // Régua fixa no topo — será desenhada por cima dos clips mais adiante.
        let ruler_h = self.ruler_px;

        // Clips (hover/drag/fantasma + overlay) — módulo clips
        let cm = clips::ClipMetrics {
            px_per_beat: self.px_per_beat,
            wave_segments: self.wave_segments.clone(),
            row_height,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
            ruler_px: self.ruler_px,
            theme: t,
            max_track: self.clips.iter().map(|c| c.track_index).max().unwrap_or(0),
        };
        instances.extend(clips::build_clip_instances(
            &self.clips,
            &self.clip_interaction,
            &cm,
            width,
        ));

        // Renderiza preview de áudio arrastado (se houver)
        if let Some(preview) = &self.drop_preview {
            let row_height = 64.0 * self.zoom_y;
            let x = preview.beat * self.px_per_beat - self.scroll_x;
            let y = self.ruler_px + preview.track as f32 * row_height - self.scroll_y;
            let w = if preview.len_beats > 0.0 { preview.len_beats * self.px_per_beat } else { 4.0 * self.px_per_beat };
            let h = row_height;
            let col = rgba(palette::ACCENT[t], 80);
            instances.push(RectInstance {
                x,
                y,
                w,
                h,
                color: col,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
        }

        // Renderiza Fades (In / Out), Curvas Bézier, Nível de Ganho e Bolinhas/Handles
        for (i, clip) in self.clips.iter().enumerate() {
            if !clip.is_midi {
                if let Some(fade) = self.fades.get(i) {
                    let (cx, cy, cw, ch) = clips::clip_rect(clip, &cm);
                    let header_h = 16.0;
                    let body_y = cy + header_h;
                    let body_h = ch - header_h;
                    if cw > 0.0 && cx + cw > 0.0 && cx < width && body_h > 0.0 {
                        fades::build_clip_fade_instances(
                            i,
                            fade,
                            cx,
                            body_y,
                            cw,
                            body_h,
                            self.px_per_beat,
                            &self.fade_interaction,
                            &mut instances,
                        );
                    }
                }
            }
        }

        // Indicador visual: borda direita piscando quando hover na ferramenta Stretch
        if current_tool() == Tool::Stretch {
            if let Some(idx) = self.clip_interaction.stretch_edge_idx {
                if let Some(clip) = self.clips.get(idx) {
                    let (cx, cy, cw, ch) = clips::clip_rect(clip, &cm);
                    let edge_x = cx + cw;
                    if edge_x > -4.0 && edge_x < width + 4.0 {
                        // Cor da waveform (#262626)
                        let col = (0x26, 0x26, 0x26);
                        instances.push(RectInstance {
                            x: edge_x - 2.0, y: cy, w: 4.0, h: ch,
                            color: [col.0, col.1, col.2, 200],
                            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                        });
                    }
                }
            }
        }

        // Régua fixa no topo — desenhada por cima do grid e dos clips
        // (cobre qualquer track que role por baixo dela).
        // Fundos alternados dos compassos (par/ímpar), igual ao corpo.
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let bg = if i % 2 == 1 {
                palette::SURFACE_HIGH[t]
            } else {
                palette::SURFACE_STEP[t]
            };
            let bw = px_per_bar;
            if bx + bw > 0.0 && bx < width && bw > 0.0 {
                let draw_x = bx.max(0.0);
                let draw_w = (bx + bw).min(width) - draw_x;
                if draw_w > 0.0 {
                    instances.push(RectInstance {
                        x: draw_x, y: 0.0, w: draw_w, h: ruler_h,
                        color: [bg.0, bg.1, bg.2, 255],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }
            // Número do compasso
            let ruler_text_color = rgba(palette::TEXT_PRIMARY[t], 220);
            if bx > -px_per_bar * 0.5 && bx < width {
                instances.extend(build_text_instances(&format!("{}", i + 1), bx + 4.0, 3.0, ruler_text_color, 1.0));
            }
            // Linha vertical do compasso (1px, altura cheia da régua)
            instances.push(RectInstance {
                x: bx, y: 0.0, w: 1.0, h: ruler_h,
                color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
            let beat_px = px_per_bar / 4.0;
            for k in 0..4 {
                let x = bx + k as f32 * beat_px;
                if x <= 0.0 || x > width { continue; }
                // Linhas de batida (quando px_per_bar >= 40)
                if px_per_bar >= 40.0 {
                    instances.push(RectInstance {
                        x, y: 2.0, w: 1.0, h: ruler_h - 2.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Subdivisões 1/8 (quando px_per_bar >= 120) — tiques menores no baixo,
            // como frações uniformes do compasso
            if px_per_bar >= 120.0 {
                let n = 8u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let x8 = bx + m as f32 * step;
                    if x8 <= 0.0 || x8 > width { continue; }
                    instances.push(RectInstance {
                        x: x8, y: ruler_h - 8.0, w: 1.0, h: 8.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Subdivisões 1/16 (quando px_per_bar >= 240) — tiques ainda menores
            if px_per_bar >= 240.0 {
                let n = 16u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let x16 = bx + m as f32 * step;
                    if x16 <= 0.0 || x16 > width { continue; }
                    instances.push(RectInstance {
                        x: x16, y: ruler_h - 6.0, w: 1.0, h: 6.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }
        // Linha inferior da régua (borda entre a régua e a timeline)
        instances.push(RectInstance {
            x: 0.0, y: ruler_h - 1.0, w: width, h: 1.0,
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Indicador de follow playhead (canto superior direito)
        if self.follow_playhead {
            let px = width - 120.0;
            let py = 2.0;
            instances.extend(build_text_instances("FOLLOW", px, py, rgba(palette::ACCENT[t], 255), 1.0));
        }

        // Playhead — sempre visível; para de avançar quando pausado.
        let bpm = self.bpm;
        let pixels_per_second = self.px_per_beat * bpm / 60.0;
        let head_x = self.transport_seconds * pixels_per_second - self.scroll_x;
            instances.push(RectInstance {
                x: head_x - 1.0, y: 1.0,
                w: 2.0, h: timeline_bottom - 1.0,
                color: [palette::PLAYHEAD_COLOR.0, palette::PLAYHEAD_COLOR.1, palette::PLAYHEAD_COLOR.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0,
                radius: 0.0,
            });
            instances.push(RectInstance {
                x: head_x - 6.0, y: 1.0,
                w: 12.0, h: 12.0,
                color: [palette::PLAYHEAD_COLOR.0, palette::PLAYHEAD_COLOR.1, palette::PLAYHEAD_COLOR.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0,
                radius: 6.0,
            });

        // Seletor de loop — bordas laterais grossas na régua (alças de arrasto)
        // + centro translúcido descendo por toda a timeline.
        if self.loop_enabled {
            let r: (u8, u8, u8) = (0x3A, 0x3A, 0x3A); // cinza escuro do loop
            let border = [r.0, r.1, r.2, 255]; // bordas de ajuste (palitos): opacas
            let fill_ruler = [r.0, r.1, r.2, 45]; // régua: centro bem transparente
            let fill_body = [r.0, r.1, r.2, 70]; // timeline: mais translúcido ainda
            let border_body = [r.0, r.1, r.2, 185]; // borda que desce na timeline: menos translúcida
            let sx = self.beat_to_x(self.loop_start_beat);
            let ex = self.beat_to_x(self.loop_end_beat);
            let lw = (ex - sx).max(0.0);
            let bw = 6.0; // grossura das bordas laterais (alças)
            let body_top = self.ruler_px;
        let body_h = (timeline_bottom - self.ruler_px).max(0.0);

            if lw > 0.0 {
                // Centro que desce toda a timeline — mais translúcido ainda.
                instances.push(RectInstance {
                    x: sx, y: body_top, w: lw, h: body_h,
                    color: fill_body,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });

                // Centro na régua — pouco translúcido.
                if sx + lw > 0.0 && sx < width {
                    instances.push(RectInstance {
                        x: sx, y: 0.0, w: lw.min(width - sx).max(0.0), h: self.ruler_px,
                        color: fill_ruler,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }

                // Bordas laterais grossas NA RÉGUA (alças de arrasto) — opacas.
                if sx + lw >= -bw && sx <= width {
                    instances.push(RectInstance {
                        x: sx, y: 0.0, w: bw, h: self.ruler_px, color: border,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
                if ex >= 0.0 && ex - bw <= width {
                    instances.push(RectInstance {
                        x: ex - bw, y: 0.0, w: bw, h: self.ruler_px, color: border,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }

                // Bordas laterais descendo pelo CORPO da timeline — menos
                // translúcidas que o fill (só a borda).
                if sx + lw >= -bw && sx <= width {
                    instances.push(RectInstance {
                        x: sx, y: body_top, w: 2.0, h: body_h, color: border_body,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
                if ex >= 0.0 && ex - bw <= width {
                    instances.push(RectInstance {
                        x: ex - 2.0, y: body_top, w: 2.0, h: body_h, color: border_body,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }

        // Linha de corte vertical viva (ferramenta Slice — tecla 4) na cor da
        // borda de seleção (#262626).
        if current_tool() == Tool::Slice && self.last_cursor.1 >= self.ruler_px {
            let cx = self.last_cursor.0;
            if cx >= 0.0 && cx <= width {
                let col = (0x26, 0x26, 0x26);
                instances.push(RectInstance {
                    x: cx - 1.0, y: self.ruler_px,
                    w: 2.0, h: (timeline_bottom - self.ruler_px).max(0.0),
                    color: [col.0, col.1, col.2, 180],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // =====================================================================
        // Time ruler — posicionada pelo layout (pode ficar acima do piano roll)
        // =====================================================================
        let tr_h = self.time_ruler_h;
        let tr_y = layout.time_ruler.y;
        let tr_bg = palette::SURFACE_HIGH[t];
        let tr_border = palette::BASE[t];

        // Fundo da time ruler
        instances.push(RectInstance {
            x: 0.0, y: tr_y, w: width, h: tr_h,
            color: [tr_bg.0, tr_bg.1, tr_bg.2, 255],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Linha separadora superior da time ruler
        instances.push(RectInstance {
            x: 0.0, y: tr_y, w: width, h: 1.0,
            color: [tr_border.0, tr_border.1, tr_border.2, 255],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Time ruler: grid de tempo fixo com marcas major + minor
        let bpm = self.bpm;
        let pixels_per_second = self.px_per_beat * bpm / 60.0;
        if pixels_per_second > 0.0 {
            let px_per_sec = pixels_per_second;
            let step_seconds: f32 = if px_per_sec < 30.0 { 1.0 }
                else if px_per_sec < 50.0 { 5.0 }
                else if px_per_sec < 80.0 { 10.0 }
                else { 30.0 };

            // Começa do primeiro segundo visível
            let start_sec = (self.scroll_x / pixels_per_second).floor();
            let end_sec = ((self.scroll_x + width) / pixels_per_second).ceil();

            // Grid minor (linhas finas, sem texto) — sumem no zoom out
            let minor_step = (step_seconds / 5.0).max(0.1);
            let minor_px = minor_step * pixels_per_second;
            let show_minor = minor_px >= 15.0;
            if show_minor {
                let minor_col = rgba(palette::GRID_SUB[t], 90);
                let mut sec = start_sec;
                while sec <= end_sec {
                    let x = sec * pixels_per_second - self.scroll_x;
                    if x >= -5.0 && x <= width + 5.0 {
                        if (sec % step_seconds).abs() < 0.01 { sec += minor_step; continue; }
                        instances.push(RectInstance {
                            x, y: tr_y + tr_h / 4.0, w: 1.0, h: tr_h / 2.0,
                            color: minor_col,
                            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                        });
                    }
                    sec += minor_step;
                }
            }

            // Grid major (linhas altas + texto M:SS)
            let major_col = rgba(palette::GRID_LINE[t], 140);
            let mut sec = start_sec;
            while sec <= end_sec {
                let x = sec * pixels_per_second - self.scroll_x;
                if x >= -20.0 && x <= width + 20.0 {
                    instances.push(RectInstance {
                        x, y: tr_y + 2.0, w: 1.0, h: tr_h - 6.0,
                        color: major_col,
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                    let total_s = sec as i64;
                    let m = total_s / 60;
                    let s = total_s % 60;
                    let time_str = format!("{}:{:02}", m, s);
                    let text_col = rgba(palette::TEXT_PRIMARY[t], 200);
                    instances.extend(build_text_instances(&time_str, x + 2.0, tr_y + 8.0, text_col, 1.0));
                }
                sec += step_seconds;
            }
        }

        // Indicador do playhead na time ruler (círculo + linha)
        let head_x = self.transport_seconds * pixels_per_second - self.scroll_x;
        if head_x >= -20.0 && head_x <= width + 20.0 {
            let col = palette::PLAYHEAD_COLOR;
            // Linha vertical fina na time ruler
            instances.push(RectInstance {
                x: head_x - 0.5, y: tr_y,
                w: 1.0, h: tr_h,
                color: [col.0, col.1, col.2, 120],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
            // Texto do tempo atual (min:sec)
            let total_s = self.transport_seconds as i64;
            let cur_m = total_s / 60;
            let cur_s = total_s % 60;
            let cur_str = format!("{}:{:02}", cur_m, cur_s);
            let cur_col = rgba(palette::TEXT_PRIMARY[t], 220);
            instances.extend(build_text_instances(&cur_str, head_x, tr_y + 8.0, cur_col, 1.0));
        }

        // Borda inferior da time ruler
        instances.push(RectInstance {
            x: 0.0, y: tr_y + tr_h - 1.0, w: width, h: 1.0,
            color: [tr_border.0, tr_border.1, tr_border.2, 255],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // ===== Edit toolbar (ferramentas) — entre time_ruler e splitter =====
        instances.extend(self.edit_toolbar_rect_instances(width, height));

        // ===== Piano roll (editor MIDI) + splitter (se visível) =====
        instances.extend(self.piano_roll_instances(width, height));

        instances
    }

/// Registra todos os passes da cena num command encoder novo (não submetido).
    /// Desenha em `view` nas dimensões `width x height`.
    /// Em modo hostilhado o alvo decide o que sai:
    ///   - `offscreen_target = true` (frame offscreen da timeline): apenas a
    ///     cena + ícones (sem composição — a composição acontece na surface);
    ///   - `offscreen_target = false` (surface): o frame da timeline
    ///     (`timeline_bg`) no retângulo `timeline_rect` + o overlay da UI.
    fn record_scene(
        &mut self,
        width: f32,
        height: f32,
        view: &wgpu::TextureView,
    ) -> wgpu::CommandEncoder {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("timeline-encoder"),
            });

        if self.hosted && !self.offscreen_target {
            self.record_composite_passes(&mut encoder, width, height, view);
            return encoder;
        }

        self.record_scene_passes(&mut encoder, width, height, view);
        if !(self.hosted && self.offscreen_target) {
            self.record_overlay_pass(&mut encoder, width, height, view);
        }

        encoder
    }

    /// Passos da cena da timeline (instâncias, wave bars e ícones), direto
    /// em `view`. Usado pelo modo standalone, pelo frame offscreen e pelo
    /// `render_timeline_bg` (target = textura da timeline no modo hostilhado).
    fn record_scene_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        width: f32,
        height: f32,
        view: &wgpu::TextureView,
    ) {
        let instances = self.build_instances(width, height);

        let data: Vec<f32> = instances.iter().flat_map(|i| i.to_f32()).collect();
        let instance_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::cast_slice(&OrthoCamera {
                scroll_x: 0.0,
                scroll_y: 0.0,
                viewport_w: width.max(1.0),
                viewport_h: height.max(1.0),
            }.proj()),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let wave_bars_data = if self.wave_bars.is_empty() {
            vec![0.0f32, 0.0f32]
        } else {
            self.wave_bars.clone()
        };
        let wave_bars_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wave-bars"),
            contents: bytemuck::cast_slice(&wave_bars_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wave_bars_buf.as_entire_binding(),
                },
            ],
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timeline-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1, g: 0.1, b: 0.12, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.set_vertex_buffer(0, self.unit_quad.slice(..));
            rpass.set_vertex_buffer(1, instance_buf.slice(..));
            rpass.draw(0..4, 0..instances.len() as u32);
        }

        // === 2º pass: ícones (edit toolbar) — quads texturizados ===
        let icon_instances = self.edit_toolbar_icon_instances(width, height);
        if !icon_instances.is_empty() {
            let icon_data: Vec<f32> = icon_instances.iter().flat_map(|i| i.to_f32()).collect();
            let icon_instance_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("icon-instances"),
                contents: bytemuck::cast_slice(&icon_data),
                usage: wgpu::BufferUsages::VERTEX,
            });

            let icon_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("icon-bg"),
                layout: &self.icon_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.icon_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.icon_atlas_view),
                    },
                ],
            });

            let mut ipass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("icon-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            ipass.set_pipeline(&self.icon_pipeline);
            ipass.set_bind_group(0, &icon_bind_group, &[]);
            ipass.set_vertex_buffer(0, self.unit_quad.slice(..));
            ipass.set_vertex_buffer(1, icon_instance_buf.slice(..));
            ipass.draw(0..4, 0..icon_instances.len() as u32);
        }
        }

    /// Pass de overlay de UI (textura RGBA cobrindo o viewport inteiro).
    fn record_overlay_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        width: f32,
        height: f32,
        view: &wgpu::TextureView,
    ) {
        if let Some(overlay_view) = &self.overlay_view {
            let (ow, oh) = self.overlay_size;
            if ow > 0 && oh > 0 {
                let w = width;
                let h = height;
                // Quad em coordenadas de pixels (x, y, u, v): y=0 → v=0 (topo
                // do viewport mapeia para a linha 0 do buffer RGBA).
                let verts: [f32; 16] = [
                    0.0, 0.0, 0.0, 0.0,
                    w,    0.0, 1.0, 0.0,
                    0.0,  h,    0.0, 1.0,
                    w,    h,    1.0, 1.0,
                ];
                let quad = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("overlay-quad"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let overlay_ubo: [f32; 20] = {
                    let p = OrthoCamera {
                        scroll_x: 0.0,
                        scroll_y: 0.0,
                        viewport_w: width.max(1.0),
                        viewport_h: height.max(1.0),
                    }.proj();
                    let mut u = [0.0f32; 20];
                    u[..16].copy_from_slice(&p);
                    u
                };
                let overlay_uniform = self.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("overlay-uniforms"),
                        contents: bytemuck::cast_slice(&overlay_ubo),
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                let overlay_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("overlay-bg"),
                    layout: &self.overlay_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: overlay_uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.overlay_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(overlay_view),
                        },
                    ],
                });

                let mut opass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                opass.set_pipeline(&self.overlay_pipeline);
                opass.set_bind_group(0, &overlay_bind_group, &[]);
                opass.set_vertex_buffer(0, quad.slice(..));
                opass.draw(0..4, 0..1);
            }
        }
    }

    /// Passes de composição do modo hostilhado (surface): o frame da timeline
    /// (`timeline_bg`) no retângulo `timeline_rect` e, por cima, o overlay da
    /// UI. Usado quando `hosted && !offscreen_target`.
    fn record_composite_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        width: f32,
        height: f32,
        view: &wgpu::TextureView,
    ) {
        // Pass de quad texturizado reutilizando o pipeline de overlay: mesmo
        // layout de bind group (uniform + sampler + texture) e blend
        // premultiplicado (o frame offscreen sai com alpha=1 em todo pixel).
        let draw_textured_quad = |encoder: &mut wgpu::CommandEncoder,
                                   device: &wgpu::Device,
                                   _queue: &wgpu::Queue,
                                   pipeline: &wgpu::RenderPipeline,
                                   layout: &wgpu::BindGroupLayout,
                                   sampler: &wgpu::Sampler,
                                   view: &wgpu::TextureView,
                                   texture_view: &wgpu::TextureView,
                                   x: f32,
                                   y: f32,
                                   w: f32,
                                   h: f32,
                                   mask: [f32; 4],
                                   load: wgpu::LoadOp<wgpu::Color>| {
            let verts: [f32; 16] = [
                x,     y,     0.0, 0.0,
                x + w, y,     1.0, 0.0,
                x,     y + h, 0.0, 1.0,
                x + w, y + h, 1.0, 1.0,
            ];
            let quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite-quad"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ubo: [f32; 20] = {
                let p = OrthoCamera {
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    viewport_w: width.max(1.0),
                    viewport_h: height.max(1.0),
                }.proj();
                let mut u = [0.0f32; 20];
                u[..16].copy_from_slice(&p);
                u[16..20].copy_from_slice(&mask);
                u
            };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite-uniforms"),
                contents: bytemuck::cast_slice(&ubo),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("composite-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.set_vertex_buffer(0, quad.slice(..));
            pass.draw(0..4, 0..1);
        };

        // Equivale ao `Theme.base` da UI (#313131): com o fundo da janela do
        // Slint transparente, este clear aparece nas áreas sem painel.
        let clear = wgpu::LoadOp::Clear(wgpu::Color { r: 0.1921569, g: 0.1921569, b: 0.1921569, a: 1.0 });

        // 1. Limpa a janela inteira (fundo atrás da UI do Slint).
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: clear, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        // 2. Frame da timeline dentro do retângulo do componente `Timeline`,
        // desenhado ANTES da UI. O fundo da janela do Slint é transparente no
        // modo hosted: o "buraco" do canvas não recebe tinta, então o quad da
        // engine aparece aqui (fetched via blend do overlay com alpha 0).
        if let (Some(bg_view), Some(_bg_tex)) = (&self.timeline_bg_view, &self.timeline_bg_texture) {
            let (bw, bh) = self.timeline_bg_size;
            let (rx, ry, rw, rh) = self.timeline_rect;
            if bw > 0 && bh > 0 && rw > 0.0 && rh > 0.0 {
                draw_textured_quad(
                    encoder,
                    &self.device,
                    &self.queue,
                    &self.overlay_pipeline,
                    &self.overlay_bind_group_layout,
                    &self.overlay_sampler,
                    view,
                    bg_view,
                    rx,
                    ry,
                    rw,
                    rh,
                    [
                        bw as f32,
                        bh as f32,
                        self.timeline_corner_radius,
                        1.0,
                    ],
                    wgpu::LoadOp::Load,
                );
            }
        }

        // 3. UI do Slint POR CIMA (blend premultiplicado). O fundo é
        // transparente, os painéis são opacos e widgets sobrepostos (dropdowns,
        // clippool, menus) pintam alpha 255 → ficam acima do quad da timeline.
        if let Some(overlay_view) = &self.overlay_view {
            let (ow, oh) = self.overlay_size;
            if ow > 0 && oh > 0 {
                draw_textured_quad(
                    encoder,
                    &self.device,
                    &self.queue,
                    &self.overlay_pipeline,
                    &self.overlay_bind_group_layout,
                    &self.overlay_sampler,
                    view,
                    overlay_view,
                    0.0,
                    0.0,
                    width,
                    height,
                    [0.0, 0.0, 0.0, 0.0],
                    wgpu::LoadOp::Load,
                );
            }
        }
    }

    /// Renderiza a cena da timeline DIRETO na textura `timeline_bg`
    /// (GPU→GPU, sem readback de CPU), para ser composta na surface no modo
    /// hostilhado. `width`/`height` devem casar com o retângulo físico da
    /// timeline na UI. Retorna `false` se o tamanho for inválido.
    pub fn render_timeline_bg(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        self.ensure_timeline_bg(width, height);
        let Some(bg_view) = self.timeline_bg_view.clone() else {
            return false;
        };
        self.size = (width, height);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("timeline-bg-encoder"),
            });
        self.record_scene_passes(&mut encoder, width as f32, height as f32, &bg_view);
        self.queue.submit(std::iter::once(encoder.finish()));
        true
    }

    /// Render do modo standalone (apresenta na surface da janela).
    /// No-op quando não há janela (engine pura — use [`App::render_to_rgba`]).
    pub fn render(&mut self) {
        // Em modo hosted o alvo de composição é a SURFACE (janela); `size`
        // segue sendo o tamanho da cena da timeline (usado pelo submit_frame).
        let (width, height) = if self.hosted { self.surface_size } else { self.size };
        if width == 0 || height == 0 {
            return;
        }
        let Some(surface) = &self.surface else { return };
        let surface_texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                if let Some(config) = self.config.as_ref() {
                    surface.configure(&self.device, config);
                }
                return;
            }
            _ => return,
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.offscreen_target = false;
        let encoder = self.record_scene(width as f32, height as f32, &view);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(surface_texture);
    }

    /// Desenha a cena num buffer offscreen (engine pura) e agenda a cópia
    /// para memória. NÃO bloqueia a UI thread — o readback é coletado depois
    /// em [`App::take_rgba`]. Retorna `false` se um readback anterior ainda
    /// está em voo (o frame é descartado; a última imagem é reapresentada).
    pub fn submit_frame(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        if self.readback_pending {
            return false;
        }
        // A cena usa `self.size` (scroll clamp etc.).
        self.size = (width, height);

        let bytes_per_row = ((width * 4 + 255) / 256) * 256;

        // (Re)cria textura/readback apenas quando o tamanho muda.
        let recreate = self.offscreen_texture.as_ref().map_or(true, |t| {
            t.width() != width || t.height() != height
        });
        if recreate {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("timeline-offscreen"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.render_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timeline-readback"),
                size: (bytes_per_row as u64) * (height as u64),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.offscreen_texture = Some(texture);
            self.readback_buffer = Some(readback_buffer);
            self.readback_size = (width, height);
        }

        let texture = match self.offscreen_texture.clone() {
            Some(t) => t,
            None => return false,
        };
        let buffer = match self.readback_buffer.clone() {
            Some(b) => b,
            None => return false,
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // O frame offscreen é a CENA (sem composição de timeline-bg/overlay):
        // o app hostilhado compõe esses dois na surface.
        self.offscreen_target = true;
        let mut encoder = self.record_scene(width as f32, height as f32, &view);
        self.offscreen_target = false;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        self.readback_done.store(false, std::sync::atomic::Ordering::Release);
        let done = self.readback_done.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |_res| {
            done.store(true, std::sync::atomic::Ordering::Release);
        });
        self.readback_pending = true;
        true
    }

    /// Coleta o readback do último frame, SEM bloquear. Retorna
    /// `Some((width, height, rgba))` (já despadded) quando a cópia terminou,
    /// ou `None` se ainda está em voo ou nada foi submetido.
    pub fn take_rgba(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        if !self.readback_pending {
            return None;
        }
        // Polling não-bloqueante: dispara o callback do map_async (se pronto)
        // e mantém a UI thread livre.
        let _ = self.device.poll(wgpu::PollType::Poll).ok();
        if !self.readback_done.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }

        let (width, height) = self.readback_size;
        let bytes_per_row = ((width * 4 + 255) / 256) * 256;
        let buffer = self.readback_buffer.clone()?;
        let data = buffer.slice(..).get_mapped_range().ok()?;
        let mut rgba: Vec<u8> = data.to_vec();
        drop(data);
        buffer.unmap();
        self.readback_pending = false;

        if bytes_per_row != width * 4 {
            let mut despadded = Vec::with_capacity((width as usize) * (height as usize) * 4);
            for row in rgba.chunks(bytes_per_row as usize) {
                despadded.extend_from_slice(&row[..(width as usize) * 4]);
            }
            rgba = despadded;
        }

        // A textura offscreen usa `render_format` (o formato da surface, em
        // geral Bgra8 no Windows/DX12). Normaliza para RGBA8 — o `set_timeline_bg`
        // espera RGBA (linha 0 = topo).
        if matches!(
            self.render_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }

        Some((width, height, rgba))
    }

    /// Render offscreen (engine pura) + leitura de volta em RGBA já despadded.
    /// Retorna `Vec<u8>` de tamanho `width * height * 4` (ou `None` se inválido).
    ///
    /// ⚠️ BLOQUEIA a thread atual até o GPU terminar. Para janela nativa/UI
    /// prefira [`App::submit_frame`] + [`App::take_rgba`] (não-bloqueante).
    pub fn render_to_rgba(&mut self, width: u32, height: u32) -> Option<Vec<u8>> {
        if !self.submit_frame(width, height) {
            return None;
        }
        let mut result = None;
        for _ in 0..120 {
            if let Some((_, _, rgba)) = self.take_rgba() {
                result = Some(rgba);
                break;
            }
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_millis(2)),
                })
                .ok();
        }
        result
    }
}
