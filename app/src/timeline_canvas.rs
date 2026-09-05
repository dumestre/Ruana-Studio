//! TimelineCanvas — renderiza o fundo da timeline (grid + faixas de
//! compasso) via wgpu e o publica como `slint::Image` offscreen.
//!
//! Estratégia (alinhada a AGENST.md):
//! - A janela do Slint usa Skia e não compartilha surface com wgpu, então a
//!   imagem é rasterizada numa textura offscreen + readback e exibida pelo
//!   Slint como `slint::Image` (mesmo padrão do `waveform_gpu`).
//! - Câmera ortográfica 2D (`OrthoCamera`) converte coordenadas de conteúdo
//!   (px) em NDC. O recto de conteúdo (que já aplica scroll-x no .slint) é
//!   o ponto de origem; aqui a câmera usa scroll=0 e viewport = dimensões do
//!   canvas, então (0,0) é o topo-esquerdo do conteúdo.
//! - Instancing render: um único quad base expandido por instância desenha
//!   faixas alternadas de compasso, linhas de barra, batidas e subdivisões,
//!   e os separadores de track — custo constante.
//!
//! Render-ON-CHANGE: a cada pedido (timer de bounds de 50ms), compara o
//! estado (scroll, zoom, métrica, tema, dims) com o último rasterizado e só
//! re-renderiza quando algo muda. Ocioso = zero tráfego wgpu.

use std::rc::Rc;

use slint::{ComponentHandle, Timer, TimerMode};
use crate::MainWindow;
use crate::timeline_gpu::wgpu::{OrthoCamera, RectInstance, TimelineGpuRenderer};
use daw_core::project::Project;

// Cor do playhead (crimson #DC143C = 220, 20, 60) - igual ao Slint.
const PLAYHEAD_COLOR: (u8, u8, u8) = (0xDC, 0x14, 0x3C);

// =====================================================
// PALETA (espelho do global Theme, por índice do tema)
// =====================================================

/// (r, g, b) do tema. Índices 0..6, mesma ordem de ui/theme.slint.
type Rgb = (u8, u8, u8);

const SURFACE_HIGH: [Rgb; 7] = [
    (0x55, 0x55, 0x55),
    (0x26, 0x30, 0x3D),
    (0x2D, 0x27, 0x40),
    (0x26, 0x2A, 0x1D),
    (0x36, 0x2A, 0x1B),
    (0x1E, 0x30, 0x35),
    (0x91, 0x91, 0x91),
]; // [6] Paper ★

const SURFACE_STEP: [Rgb; 7] = [
    (0x42, 0x42, 0x42),
    (0x23, 0x2D, 0x38),
    (0x27, 0x23, 0x34),
    (0x20, 0x23, 0x1A),
    (0x30, 0x26, 0x1F),
    (0x19, 0x2C, 0x31),
    (0x7A, 0x7A, 0x7A),
]; // [6] Paper ★

const GRID_LINE: [Rgb; 7] = [
    (0x75, 0x75, 0x75),
    (0x4A, 0x5A, 0x70),
    (0x4A, 0x44, 0x66),
    (0x4A, 0x4D, 0x3A),
    (0x5A, 0x4E, 0x3A),
    (0x3A, 0x5C, 0x66),
    (0xD6, 0xD6, 0xD6),
]; // [6] Paper ★

const GRID_SUB: [Rgb; 7] = [
    (0x45, 0x45, 0x45),
    (0x2A, 0x36, 0x45),
    (0x2F, 0x2C, 0x42),
    (0x2A, 0x2D, 0x24),
    (0x3A, 0x30, 0x28),
    (0x21, 0x36, 0x3C),
    (0xEF, 0xEF, 0xEF),
]; // [6] Paper ★

const TIMELINE_BG: [Rgb; 7] = [
    (0x69, 0x69, 0x69),
    (0x1E, 0x2C, 0x35),
    (0x1F, 0x1B, 0x28),
    (0x18, 0x1B, 0x11),
    (0x25, 0x21, 0x19),
    (0x12, 0x26, 0x28),
    (0x79, 0x79, 0x79),
]; // [6] Paper ★

// =====================================================
// PALETA DOS CLIPS (espelho do ClipPalette + cor padrão)
// =====================================================

/// ClipPalette.colors (ui/clip_palette.slint), índices 0..19.
const CLIP_PALETTE: [Rgb; 20] = [
    (0x6F, 0xA8, 0x6B),
    (0x4F, 0x8A, 0xE0),
    (0xE0, 0xA9, 0x4F),
    (0xE0, 0x5E, 0x5E),
    (0xA0, 0x60, 0xD8),
    (0xE0, 0x5E, 0xAA),
    (0x3E, 0xC7, 0xB0),
    (0xC0, 0xC0, 0x50),
    (0xE0, 0x7A, 0x3E),
    (0x8F, 0xD1, 0x4F),
    (0x3F, 0xA3, 0x4D),
    (0x35, 0xB5, 0xE5),
    (0x5A, 0x8F, 0xE0),
    (0x55, 0x60, 0xD8),
    (0x9B, 0x5C, 0xE0),
    (0xD8, 0x4F, 0xB2),
    (0xE0, 0x4F, 0x6F),
    (0xA8, 0x74, 0x4A),
    (0xD8, 0xC0, 0x4F),
    (0x7A, 0x93, 0xA8),
];

/// Cor de clip MIDI padrão (color == 0), por tema (Theme.clip-midi-list).
const CLIP_MIDI_DEFAULT: [Rgb; 7] = [
    (0x6F, 0xA8, 0x6B), // [0]
    (0x6F, 0xA8, 0x6B), // [1]
    (0x6F, 0xA8, 0x6B), // [2]
    (0x8F, 0xC1, 0x4F), // [3]
    (0x8F, 0xAA, 0x6B), // [4]
    (0x6F, 0xA8, 0x6B), // [5]
    (0x52, 0xBA, 0x46), // [6] Paper ★
];

/// Cor de clip AUDIO padrão (color == 0), fixa no code (track_clip.slint).
const CLIP_AUDIO_DEFAULT: Rgb = (0x00, 0xC8, 0x53);

/// Theme.text-primary (indicador direito do clip), por tema.
const TEXT_PRIMARY: [Rgb; 7] = [
    (0xE0, 0xE0, 0xE0),
    (0xE2, 0xE6, 0xEE),
    (0xE0, 0xDA, 0xF0),
    (0xD5, 0xE8, 0xB0),
    (0xE8, 0xD8, 0xC0),
    (0xC8, 0xE8, 0xE0),
    (0x17, 0x17, 0x17),
]; // [6] Paper ★

/// Cor (r,g,b) → RGBA opaco.
fn rgba((r, g, b): Rgb, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

/// Escurece `Rgb` por fator (aproxima o `color.darker(amount)` do Slint,
/// que reduz a luminância; usamos escala linear (1-amount) por canal).
fn darker((r, g, b): Rgb, amount: f32) -> Rgb {
    let f = (1.0 - amount).max(0.0);
    (
        (r as f32 * f).round() as u8,
        (g as f32 * f).round() as u8,
        (b as f32 * f).round() as u8,
    )
}

/// RGBA straight-alpha a partir de Rgb + alpha [0..1] → [0..255].
fn rgba_alpha((r, g, b): Rgb, alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    [r, g, b, a]
}

// =====================================================
// Fonte bitmap 8x8 para renderizar números na régua.
// Cada caractere é 8px de largura, 8px de altura.
// Cada bit (1) = pixel ligado. Bitmap é índiceado [linha][coluna].
// =====================================================

/// Fonte bitmap 8x8 para dígitos 0-9.
const FONT_8X8: [[[u8; 8]; 8]; 10] = [
    // '0'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '1'
    [
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 0, 0, 0],
        [0, 1, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 0],
    ],
    // '2'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 1, 1, 1, 0],
        [0, 0, 0, 1, 1, 1, 0, 0],
        [0, 0, 1, 1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
    ],
    // '3'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '4'
    [
        [0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 1, 1, 1, 0, 0],
        [0, 0, 1, 0, 1, 1, 0, 0],
        [0, 1, 0, 0, 1, 1, 0, 0],
        [1, 0, 0, 0, 1, 1, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 1, 1, 0],
    ],
    // '5'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '6'
    [
        [0, 0, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '7'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 1, 1, 0],
        [0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
    ],
    // '8'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '9'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 1, 1, 1, 1, 0, 0],
    ],
];

/// Desenha um número (0-999) na régua usando a fonte bitmap 8x8.
/// Cada dígito ocupa 8px de largura. O texto é centrado no x informado.
fn build_text_instances(
    text: &str,
    center_x: f32,
    y: f32,
    color: [u8; 4],
) -> Vec<RectInstance> {
    let char_w = 8.0;
    let gap = 1.0;
    let total_w = (text.len() as f32 - 1.0) * gap + text.len() as f32 * char_w;
    let start_x = center_x - total_w / 2.0;

    let mut instances = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let digit = ch.to_digit(10);
        if let Some(d) = digit {
            let glyph = &FONT_8X8[d as usize];
            for row in 0..8 {
                for col in 0..8 {
                    if glyph[row][col] == 1 {
                        instances.push(RectInstance {
                            x: start_x + i as f32 * (char_w + gap) + col as f32,
                            y: y + row as f32,
                            w: 1.0,
                            h: 1.0,
                            color,
                            wave_offset: 0,
                            wave_count: 0,
                            clip_w: 0.0,
                            clip_h: 0.0,
                        });
                    }
                }
            }
        }
    }
    instances
}

// =====================================================
// ESTADO DE RENDER-ON-CHANGE
// =====================================================

#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameKey {
    scroll_x: f32,
    scroll_y: f32,
    zoom: f32,
    width: u32,
    height: u32,
    total_bars: i32,
    beats_per_bar: i32,
    ruler_px: f32,
    px_per_bar: f32,
    theme: i32,
    // Assinatura dos clips (contagem + hash). Como `Project.clips` é o
    // agregado que muda quando o usuário edita a timeline, qualquer
    // mudança de conteúdo/posição/len/cor/ordem invalida o frame.
    clips_len: u32,
    clips_hash: u64,
    // Playhead state (for render-on-change when playing).
    playing: bool,
    transport_time_seconds: f32,
    bpm: i32,
}

impl Default for FrameKey {
    fn default() -> Self {
        Self {
            scroll_x: -1.0,
            scroll_y: -1.0,
            zoom: -1.0,
            width: 0,
            height: 0,
            total_bars: -1,
            beats_per_bar: -1,
            ruler_px: 0.0,
            px_per_bar: 0.0,
            theme: -1,
            clips_len: 0,
            clips_hash: 0,
            playing: false,
            transport_time_seconds: -1.0,
            bpm: -1,
        }
    }
}

// =====================================================
// BUILD DE INSTÂNCIAS
// =====================================================

/// Monta as instâncias retangulares do fundo da timeline.
///
/// Ordem de empilhamento preservada: primeiro as faixas alternadas de
/// compasso (z mais baixo = desenhado antes), depois as linhas de grade.
fn build_instances(
    width: u32,
    height: u32,
    total_bars: i32,
    beats_per_bar: i32,
    ruler_px: f32,
    px_per_bar: f32,
    theme: i32,
) -> Vec<RectInstance> {
    let t = theme.clamp(0, 6) as usize;
    let mut out = Vec::new();

    // Fundo geral da área de tracks (timeline-bg).
    out.push(RectInstance {
        x: 0.0,
        y: ruler_px,
        w: width as f32,
        h: (height as f32 - ruler_px).max(0.0),
        color: rgba(TIMELINE_BG[t], 255),
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
    });

    // Faixas alternadas de compasso (abaixo da régua).
    for i in 0..total_bars {
        let x = i as f32 * px_per_bar;
        if x > width as f32 {
            break;
        }
        // Fundo alternado: barra ímpar = surface-high, par = surface-step.
        let bg = if i % 2 == 1 { SURFACE_HIGH[t] } else { SURFACE_STEP[t] };
        out.push(RectInstance {
            x,
            y: ruler_px,
            w: px_per_bar,
            h: (height as f32 - ruler_px).max(0.0),
            color: [bg.0, bg.1, bg.2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });
    }

    // Regua: banda superior com superfície alta (push direto ANTES do
    // closure `vline`, para não conflitar com o borrow de `out`).
    out.push(RectInstance {
        x: 0.0,
        y: 0.0,
        w: width as f32,
        h: ruler_px,
        color: rgba(SURFACE_HIGH[t], 255),
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
    });

    // Linhas verticais do grid (compasso, batida, subdivisões) + régua.
    // Inline via closure único para evitar double-borrow do `out`.
    let mut vline = |x: f32, y: f32, h: f32, w: f32, c: Rgb, a: u8| {
        out.push(RectInstance {
            x,
            y,
            w,
            h: h.max(0.0),
            color: [c.0, c.1, c.2, a],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });
    };

    // Compassos: linha da barra + subdivisões adaptativas (mesma regra do
    // ruler: ≥40px batidas, ≥120px 1/8, ≥240px 1/16, ≥480px 1/32, ≥960px 1/64).
    for i in 0..total_bars {
        let bx = i as f32 * px_per_bar;
        if bx > width as f32 {
            break;
        }
        let bar_a = if i % 2 == 1 { 255 } else { 140 };
        vline(bx, 0.0, height as f32, 1.0, GRID_LINE[t], bar_a);

        // Linhas dentro do compasso (régua curtas + corpo inteiro).
        let beat_px = px_per_bar / beats_per_bar as f32;
        for k in 0..beats_per_bar {
            let x = bx + k as f32 * beat_px;
            if x <= 0.0 {
                continue;
            }
            if px_per_bar >= 40.0 {
                vline(x, ruler_px, height as f32 - ruler_px, 1.0, GRID_LINE[t], 150);
                vline(x, ruler_px - 10.0, 10.0, 1.0, GRID_LINE[t], 200);
            }
            // 1/8
            if px_per_bar >= 120.0 {
                let x8 = x + beat_px * 0.5;
                vline(x8, ruler_px, height as f32 - ruler_px, 1.0, GRID_SUB[t], 130);
                vline(x8, ruler_px - 8.0, 8.0, 1.0, GRID_SUB[t], 180);
            }
            // 1/16 (1/4 e 3/4 da batida)
            if px_per_bar >= 240.0 {
                let x16a = x + beat_px * 0.25;
                let x16b = x + beat_px * 0.75;
                vline(
                    x16a,
                    ruler_px,
                    height as f32 - ruler_px,
                    1.0,
                    GRID_SUB[t],
                    110,
                );
                vline(
                    x16b,
                    ruler_px,
                    height as f32 - ruler_px,
                    1.0,
                    GRID_SUB[t],
                    110,
                );
                vline(x16a, ruler_px - 6.0, 6.0, 1.0, GRID_SUB[t], 160);
                vline(x16b, ruler_px - 6.0, 6.0, 1.0, GRID_SUB[t], 160);
            }
            // 1/32 (>=480px por compasso)
            if px_per_bar >= 480.0 {
                let thirty_second_px = beat_px / 8.0;
                for s in 0..7 {
                    let xs = x + thirty_second_px * (s as f32 + 0.5);
                    vline(xs, ruler_px, height as f32 - ruler_px, 1.0, GRID_SUB[t], 90);
                    vline(xs, ruler_px - 4.0, 4.0, 1.0, GRID_SUB[t], 140);
                }
            }
            // 1/64 (>=960px por compasso)
            if px_per_bar >= 960.0 {
                let sixty_fourth_px = beat_px / 16.0;
                for s in 0..15 {
                    let xs = x + sixty_fourth_px * (s as f32 + 0.5);
                    vline(xs, ruler_px, height as f32 - ruler_px, 1.0, GRID_SUB[t], 70);
                    vline(xs, ruler_px - 2.0, 2.0, 1.0, GRID_SUB[t], 120);
                }
            }
        }
    }

    // Números de compasso na régua (centrado na barra).
    let ruler_text_color = rgba(TEXT_PRIMARY[t], 220);
    for i in 0..total_bars {
        let bx = i as f32 * px_per_bar;
        if bx > width as f32 || bx < -px_per_bar {
            continue;
        }
        out.extend(build_text_instances(
            &format!("{}", i + 1),
            bx + px_per_bar / 2.0,
            8.0,
            ruler_text_color,
        ));
    }

    out
}

// =====================================================
// CLIPS (corpo + header) — lidos do CORE (Project.clips)
// =====================================================

/// Dados mínimos de um clip para o canvas (espelho do core `Clip`,
/// projetados com a geometria da timeline já aplicada).
///
/// O CORE é a fonte autoritativa (`Project.clips`); a UI Slint replica
/// via `ClipData`, mas aqui lemos direto do modelo Rust para não
/// duplicar/cristalizar estado. O que é estado transiente de UI
/// (seleção, hover, ghost, renomeação) NÃO vem para cá — fica
/// como overlay Slint sobre o corpo (borda, nome, handles, fades).
#[allow(dead_code)]
pub struct ClipVisual {
    /// ID do clip no core (para buscar waveform no cache).
    pub clip_id: u64,
    /// Índice da row na timeline (posição de `track_id` em `tracks`).
    pub track_index: usize,
    /// Posição inicial em beats.
    pub beat: f32,
    /// Duração em beats.
    pub len: f32,
    /// `true` = clip MIDI.
    pub is_midi: bool,
    /// Índice de cor da paleta (0 = padrão por tipo).
    pub color: u8,
    /// Para clips de áudio: caminho do arquivo (para buscar waveform).
    pub file_path: Option<String>,
    /// Ganho em dB.
    pub gain_db: f32,
    /// Fade-in em beats.
    pub fade_in_beats: f32,
    /// Fade-out em beats.
    pub fade_out_beats: f32,
    /// Número de canais: 1 = mono, 2 = stereo.
    pub channels: u16,
    /// Preenchido no render: offset no buffer global de wave_bars.
    pub wave_offset: u32,
    /// Preenchido no render: nº de colunas da waveform.
    pub wave_count: u32,
}

/// Cor-base de um clip (mesma regra do `base-color` em track_clip.slint):
/// `color == 0` → MIDI usa a cor padrão do tema, audio usa #00C853;
/// senão índexa a `ClipPalette.colors[color mod 20]`.
fn clip_base_color(clip: &ClipVisual, theme: usize) -> Rgb {
    let t = theme.clamp(0, 6);
    if clip.color == 0 {
        if clip.is_midi {
            CLIP_MIDI_DEFAULT[t]
        } else {
            CLIP_AUDIO_DEFAULT
        }
    } else {
        CLIP_PALETTE[(clip.color as usize) % CLIP_PALETTE.len()]
    }
}

/// Monta as instâncias retangulares dos clips (corpo translúcido + header
/// opaco + divisória + indicador direito) em coordenadas de CONTEÚDO.
///
/// O que fica de fora (desenhado pelo Slint por cima): borda/seleção/hover/
/// ghost, nome, fades, gain, preview de slice e handles de resize/drag —
/// são estados transientes de UI que não vivem no core.
fn build_clip_instances(
    ruler_px: f32,
    row_height: f32,
    px_per_beat: f32,
    theme: usize,
    clips: &[ClipVisual],
    _wave_bars: &[f32], // buffer global de min/max (preenchido no render)
) -> Vec<RectInstance> {
    let min_w = 4.0f32; // minimum-clip-width (px)
    let header_h = 16.0f32; // header-height (px)
    let bw = 1.0f32; // border-width (px), resting
    let mut out = Vec::new();

    for clip in clips {
        let base = clip_base_color(clip, theme);

        let x = clip.beat * px_per_beat;
        let y = clip.track_index as f32 * row_height + ruler_px;
        let w = (clip.len * px_per_beat).max(min_w);
        let h = row_height;

        // Culling: pula clips totalmente fora da viewport horizontal.
        if w <= 0.0 {
            continue;
        }

        let is_stereo = clip.channels == 2 && !clip.is_midi;

        // Header opaco no topo do clip.
        out.push(RectInstance {
            x: x + bw,
            y: y + bw,
            w: (w - 2.0 * bw).max(0.0),
            h: (header_h - bw).max(0.0),
            color: rgba(base, 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });

        if is_stereo {
            // Stereo: divide o corpo em 5 partes iguais. A waveform L ocupa o 2º
            // quinto, a waveform R ocupa o 4º quinto, e os 1º/3º/5º ficam vazios
            // como separadores. layout de cima pra baixo: espaco, L, espaco, R, espaco.
            let body_w = (w - 2.0 * bw).max(0.0);
            let body_h = (h - header_h - bw).max(0.0);
            let fifth = body_h / 5.0;

            // Fill top spacer (1º quinto)
            out.push(RectInstance {
                x: x + bw,
                y: y + header_h,
                w: body_w,
                h: fifth,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: fifth,
            });

            // Faixa L (2º quinto)
            out.push(RectInstance {
                x: x + bw,
                y: y + header_h + fifth,
                w: body_w,
                h: fifth,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: clip.wave_offset,
                wave_count: clip.wave_count,
                clip_w: -4.0, // sinaliza stereo L para o shader
                clip_h: fifth,
            });

            // Fill center spacer (3º quinto)
            out.push(RectInstance {
                x: x + bw,
                y: y + header_h + 2.0 * fifth,
                w: body_w,
                h: fifth,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: fifth,
            });

            // Faixa R (4º quinto)
            out.push(RectInstance {
                x: x + bw,
                y: y + header_h + 3.0 * fifth,
                w: body_w,
                h: fifth,
                color: rgba_alpha(darker(base, 0.18), 0.50),
                wave_offset: clip.wave_offset,
                wave_count: clip.wave_count,
                clip_w: -5.0, // sinaliza stereo R para o shader
                clip_h: fifth,
            });

            // Fill bottom spacer (5º quinto)
            out.push(RectInstance {
                x: x + bw,
                y: y + header_h + 4.0 * fifth,
                w: body_w,
                h: fifth,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: fifth,
            });

            // Linhas divisórias entre as 5 partes (4 dividers)
            for i in 1..5u32 {
                let di = i as f32;
                out.push(RectInstance {
                    x: x + bw,
                    y: y + header_h + di * fifth - 1.0,
                    w: body_w,
                    h: 1.0,
                    color: rgba(darker(base, 0.25), 255),
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                });
            }
        } else {
            // Mono: waveform na faixa central (1/3 do corpo), com 2 linhas
            // divisórias separando as 3 partes.
            let body_w = (w - 2.0 * bw).max(0.0);
            let third = (h - header_h - bw).max(0.0) / 3.0;
            let body_y = y + header_h;

            // Fill top spacer (1/3)
            out.push(RectInstance {
                x: x + bw,
                y: body_y,
                w: body_w,
                h: third,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: third,
            });

            out.push(RectInstance {
                x: x + bw,
                y: body_y + third,
                w: body_w,
                h: third,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: clip.wave_offset,
                wave_count: clip.wave_count,
                clip_w: -6.0, // sinaliza mono-center para o shader
                clip_h: third,
            });

            // Fill bottom spacer (1/3)
            out.push(RectInstance {
                x: x + bw,
                y: body_y + 2.0 * third,
                w: body_w,
                h: third,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: third,
            });

            // Linha divisória acima da faixa central
            out.push(RectInstance {
                x: x + bw,
                y: body_y + third - 1.0,
                w: body_w,
                h: 1.0,
                color: rgba(darker(base, 0.25), 255),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
            });

            // Linha divisória abaixo da faixa central
            out.push(RectInstance {
                x: x + bw,
                y: body_y + 2.0 * third - 1.0,
                w: body_w,
                h: 1.0,
                color: rgba(darker(base, 0.25), 255),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
            });
        }

        // Divisória header/corpo.
        out.push(RectInstance {
            x: x + bw,
            y: y + header_h - 1.0,
            w: (w - 2.0 * bw).max(0.0),
            h: 1.0,
            color: rgba(darker(base, 0.18), 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });

        // Indicador direito (3×8 px) perto do topo.
        out.push(RectInstance {
            x: x + w - bw - 6.0,
            y: y + 5.0,
            w: 3.0,
            h: 8.0,
            color: rgba(TEXT_PRIMARY[theme.clamp(0, 6)], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });

        // Culling vertical implícito: rects fora do raster são descartados
        // pelo wgpu (topology/clip), então não é preciso filtrar por y aqui.
    }

    out
}

/// Monta as instâncias do playhead (linha vertical + cabeça circular).
///
/// Coordenadas de CONTEÚDO. A linha vai do topo da régua até o fim
/// das tracks. A cabeça (bolinha) fica centralizada na régua.
fn build_playhead_instances(
    head_x: f32,
    ruler_px: f32,
    tracks_height: f32,
) -> Vec<RectInstance> {
    vec![
        // Linha vertical (2px largura) da régua até fim das tracks.
        RectInstance {
            x: head_x - 1.0,
            y: ruler_px,
            w: 2.0,
            h: tracks_height,
            color: [PLAYHEAD_COLOR.0, PLAYHEAD_COLOR.1, PLAYHEAD_COLOR.2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        },
        // Cabeça (bolinha 12px) na régua (y=1px, centrada verticalmente ~7px).
        // Desenhada como quadrado; o shader faz quadrados. Para círculo real
        // precisaria SDF no fragment shader. Por enquanto quadrado —
        // aceitável no playhead fino.
        RectInstance {
            x: head_x - 6.0,
            y: 1.0,
            w: 12.0,
            h: 12.0,
            color: [PLAYHEAD_COLOR.0, PLAYHEAD_COLOR.1, PLAYHEAD_COLOR.2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        },
    ]
}

// =====================================================
// VU METERS (barras horizontais por track, à direita)
// =====================================================

/// Monta as instâncias dos VU meters: uma barra horizontal por track
/// à direita da área de conteúdo, com largura proporcional ao nível (0.0-1.0).
fn build_vu_meter_instances(
    levels: &[f32],
    ruler_px: f32,
    row_height: f32,
    canvas_width: f32,
    canvas_height: f32,
    theme: i32,
) -> Vec<RectInstance> {
    let mut out = Vec::new();
    let t = theme.clamp(0, 6) as usize;

    // Posição X: um pouco antes da borda direita do canvas.
    // O canvas width é a viewport width; colocamos os meters em [w-60, w-10].
    let meter_x = canvas_width - 60.0;
    let meter_w_max = 50.0;
    let meter_h = (row_height * 0.6).max(4.0); // 60% da altura da track, mín 4px
    let meter_y_offset = (row_height - meter_h) / 2.0; // centralizado verticalmente na track

    // Cor do meter: verde brilhante (padrão) ou vermelho se clipping (>0.9)
    let meter_green: Rgb = (0x00, 0xE0, 0x00);
    let meter_red: Rgb = (0xE0, 0x20, 0x20);
    let meter_bg: Rgb = GRID_SUB[t]; // fundo sutil

    for (track_idx, &level) in levels.iter().enumerate() {
        let y = track_idx as f32 * row_height + ruler_px + meter_y_offset;
        if y > canvas_height {
            break;
        }
        let lvl = level.clamp(0.0, 1.0);
        let color = if lvl > 0.9 { meter_red } else { meter_green };

        // Fundo do meter (track bg)
        out.push(RectInstance {
            x: meter_x,
            y,
            w: meter_w_max,
            h: meter_h,
            color: rgba(meter_bg, 180),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
        });
        // Nível atual
        if lvl > 0.01 {
            out.push(RectInstance {
                x: meter_x,
                y,
                w: meter_w_max * lvl,
                h: meter_h,
                color: rgba(color, 230),
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
            });
        }
        // Pico (linha fina no topo) - opcional, simplificado
    }

    out
}

// =====================================================
// WIRING
// =====================================================

/// Cria o renderer e liga o callback de re-render do canvas.
///
/// `project` é o core (`Project`) fonte dos clips — lemos o corpo/header
/// dos clips direto do modelo Rust (não duplicamos estado pelo Slint).
/// `meter_state` é o estado dos VU meters (níveis por track, 0.0-1.0).
/// `waveforms` é o cache de waveforms (para ler barras baked dos clips de áudio).
pub fn wire(
    ui: &MainWindow,
    project: Rc<std::cell::RefCell<Project>>,
    meter_state: Rc<std::cell::RefCell<Vec<f32>>>,
    waveforms: Rc<std::cell::RefCell<crate::waveform::WaveformCache>>,
) {
    let Some(renderer) = TimelineGpuRenderer::new().ok() else {
        eprintln!("[timeline_canvas] Renderer indisponível — fundo desativado");
        return;
    };
    let renderer = Rc::new(renderer);

    // Estado de render-on-change (compare com o último rasterizado).
    let last: Rc<std::cell::RefCell<FrameKey>> =
        Rc::new(std::cell::RefCell::new(FrameKey::default()));

    let ui_shot = ui.as_weak();
    let last_shot = last.clone();
    let project_shot = project.clone();
    let renderer_shot = renderer.clone();
    let meter_state_shot = meter_state.clone();
    let waveforms_shot = waveforms.clone();

    // Função de renderização compartilhada.
    // `force = true` ignora o check de render-on-change (usado pelo timer de playhead).
    let do_render = move |force: bool| {
        let Some(ui) = ui_shot.upgrade() else { return; };

        // Coleta os parâmetros do estado atual.
        //
        // `px-per-bar` no Slint = px-per-beat (40*zoom) × beats-per-bar.
        // O canvas precisa usar o MESMO valor para alinhar com a régua e
        // com os steps desenhados pelo Slint; antes usava só 40*zoom, o
        // que comprimia o grid em `beats` vezes e dessincronizava compasso.
        let ppb = ui.get_beats_per_bar().max(1);
        let px_per_bar = 40.0 * ui.get_zoom_level() * ppb as f32;
        let ruler_px = 25.0;
        let zoom = ui.get_zoom_level();
        let zoom_y = ui.get_zoom_y_level();
        let row_height = 64.0 * zoom_y;
        let px_per_beat = 40.0 * zoom;
        let width = ui.get_timeline_canvas_content_width().max(0.0);
        let height = ui.get_timeline_canvas_content_height().max(0.0);
        let theme = ui.get_theme_index();

        // Estado do playhead.
        let playing = ui.get_playing();
        let transport_time_seconds = ui.get_transport_time_seconds();
        let bpm = ui.get_bpm();

        // Lê os clips do CORE (fonte autoritativa) e projeta a geometria.
        // Mapeia track_id → índice da row percorrendo `project.tracks`.
        let proj = project_shot.borrow();
        let track_index_of = |id: u64| -> usize {
            proj.tracks
                .iter()
                .position(|t| t.id == id)
                .unwrap_or(0)
        };

        let mut clip_visuals: Vec<ClipVisual> = proj
            .clips
            .iter()
            .map(|c| ClipVisual {
                clip_id: c.id,
                track_index: track_index_of(c.track_id),
                beat: c.start_beat,
                len: c.length_beats,
                is_midi: c.is_midi(),
                color: c.color,
                file_path: c.file_path.clone(),
                gain_db: c.gain_db,
                fade_in_beats: c.fade_in_beats,
                fade_out_beats: c.fade_out_beats,
                channels: 1,
                wave_offset: 0,
                wave_count: 0,
            })
            .collect();

        // Os clips já vêm ordenados por track do core; garante ordem
        // estável para a assinatura (independente de ordens parciais).
        clip_visuals.sort_by_key(|c| (c.track_index as u32, (c.beat * 100.0) as i64));

        // Assinatura dos clips (conteúdo + posição) → render-on-change.
        let clips_len = clip_visuals.len() as u32;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325u64; // FNV offset basis
        for c in &clip_visuals {
            let s = format!(
                "{}:{}:{}:{}:{}:",
                c.track_index,
                (c.beat * 100.0) as i64,
                (c.len * 100.0) as i64,
                c.color,
                c.is_midi as u8
            );
            for b in s.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
            }
        }

        let key = FrameKey {
            scroll_x: ui.get_timeline_scroll_x(),
            scroll_y: ui.get_timeline_scroll_y(),
            zoom,
            width: width as u32,
            height: height as u32,
            total_bars: ui.get_total_bars(),
            beats_per_bar: ui.get_beats_per_bar(),
            ruler_px,
            px_per_bar,
            theme,
            clips_len,
            clips_hash: h,
            playing,
            transport_time_seconds,
            bpm,
        };

        // Render-on-change: ignora se nada mudou (exceto quando forçado).
        if !force && *last_shot.borrow() == key {
            return;
        }
        *last_shot.borrow_mut() = key;

        let w = key.width.max(1);
        let h = key.height.max(1);

        // Grid primeiro (fundo), clips por cima.
        let mut instances = build_instances(
            w,
            h,
            key.total_bars,
            key.beats_per_bar,
            ruler_px,
            px_per_bar,
            theme,
        );

        // Waveforms: constrói buffer global de barras (min/max interleaved) para clips de áudio.
        // Lê do WaveformCache, escolhe LOD por largura de exibição, bakeia gain/fade.
        let mut wave_bars = Vec::new();
        let mut wave_offset = 0u32;
        {
            let wf_cache = waveforms_shot.borrow();
            for clip in &mut clip_visuals {
                if clip.is_midi || clip.file_path.is_none() {
                    clip.wave_offset = 0;
                    clip.wave_count = 0;
                    continue;
                }
                let clip_w = (clip.len * px_per_beat).max(4.0);
                let width_px = clip_w.round() as u32;
                if width_px < 16 {
                    clip.wave_offset = 0;
                    clip.wave_count = 0;
                    continue;
                }
                if let Some(cw) = wf_cache.get(&clip.clip_id) {
                    let level = crate::waveform::pick_level(&cw.lods, width_px as usize);
                    let fi_px = clip.fade_in_beats * px_per_beat;
                    let fo_px = clip.fade_out_beats * px_per_beat;
                    let (mins, maxs) = crate::waveform::bake_bars(level, width_px, 1.0, fi_px, fo_px);
                    clip.wave_offset = wave_offset;
                    clip.wave_count = mins.len() as u32;
                    wave_offset += mins.len() as u32;
                    for (&mn, &mx) in mins.iter().zip(maxs.iter()) {
                        wave_bars.push(mn);
                        wave_bars.push(mx);
                    }
                } else {
                    clip.wave_offset = 0;
                    clip.wave_count = 0;
                }
            }
        }

        instances.extend(build_clip_instances(
            ruler_px,
            row_height,
            px_per_beat,
            theme as usize,
            &clip_visuals,
            &wave_bars,
        ));

        // Playhead: desenha por cima de tudo quando tocando.
        if playing {
            // playhead-x em coordenadas de CONTEÚDO (igual ao Slint):
            // timeline-left-padding + current-time-seconds * pixels-per-second
            // pixels-per-second = px_per_beat / (60.0 / bpm)
            let secs = transport_time_seconds.max(0.0);
            let bpm_f = bpm.max(1) as f32;
            let pixels_per_second = px_per_beat * bpm_f / 60.0;
            let head_x = ui.get_timeline_left_padding() + secs * pixels_per_second;

            // A linha do playhead vai da régua até o fim do canvas (height).
            instances.extend(build_playhead_instances(
                head_x,
                ruler_px,
                h as f32,
            ));
        }

        // VU meters: barras horizontais pequenas à direita de cada track.
        // Lê os níveis do estado compartilhado (atualizado pelo timer de 80ms).
        let meter_levels = meter_state_shot.borrow();
        instances.extend(build_vu_meter_instances(
            &meter_levels,
            ruler_px,
            row_height,
            w as f32,
            h as f32,
            theme,
        ));

        let camera = OrthoCamera {
            scroll_x: key.scroll_x,
            scroll_y: key.scroll_y,
            viewport_w: w as f32,
            viewport_h: h as f32,
        };

        // Renderiza offscreen → readback.
        let Some(rgba) = renderer_shot.render_frame(w, h, &camera, &instances, &wave_bars) else {
            return;
        };

        // Converte RGBA → slint::Image → publicação.
        let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, w, h);
        let img = slint::Image::from_rgba8(buf);
        ui.set_timeline_canvas_background(img);
    };

    // Clone do_render para poder usar em ambos os closures.
    let do_render_for_callback = do_render.clone();
    let do_render_for_timer = do_render.clone();

    // Callback do timer de bounds (50ms) — render-on-change normal.
    ui.on_timeline_canvas_render_requested(move || {
        do_render_for_callback(false);
    });

    // Timer dedicado para playhead a ~60fps (16ms) quando tocando.
    // Roda no Rust para não depender do timer Slint de 50ms.
    let playhead_timer = Timer::default();
    let ui_timer = ui.as_weak();
    playhead_timer.start(
        TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            let ui = ui_timer.upgrade();
            if ui.is_some() && ui.as_ref().unwrap().get_playing() {
                do_render_for_timer(true);
            }
        },
    );
    // Mantém o timer vivo (o closure captura a referência).
    std::mem::forget(playhead_timer);
}