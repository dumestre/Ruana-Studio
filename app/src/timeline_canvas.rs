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

use slint::ComponentHandle;
use crate::MainWindow;
use crate::timeline_gpu::wgpu::{OrthoCamera, RectInstance, TimelineGpuRenderer};
use daw_core::project::Project;

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
        });
    };

    // Compassos: linha da barra + subdivisões adaptativas (mesma regra do
    // ruler: ≥40px batidas, ≥120px 1/8, ≥240px 1/16).
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
        }
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
/// (seleção, hover, ghost, ghost reanomeação) NÃO vem para cá — fica
/// como overlay Slint sobre o corpo (borda, nome, handles, fades).
pub struct ClipVisual {
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
) -> Vec<RectInstance> {
    let min_w = 4.0f32; // minimum-clip-width (px)
    let header_h = 18.0f32; // header-height (px)
    let bw = 1.0f32; // border-width (px), resting
    let mut out = Vec::new();

    for clip in clips {
        let base = clip_base_color(clip, theme);

        let x = clip.beat * px_per_beat;
        let y = clip.track_index as f32 * row_height + ruler_px;
        let w = (clip.len * px_per_beat).max(min_w);
        let h = row_height;

        // Culling: pula clips totalmente fora da viewport horizontal.
        // (Sem acesso à viewport aqui; o caller filtra por scroll.)
        if w <= 0.0 {
            continue;
        }

        // Header opaco no topo do clip.
        out.push(RectInstance {
            x: x + bw,
            y: y + bw,
            w: (w - 2.0 * bw).max(0.0),
            h: (header_h - bw).max(0.0),
            color: rgba(base, 255),
        });

        // Corpo translúcido (DARKER + alpha) abaixo da header → compõe
        // SRC-OVER com o grid por baixo graças ao ALPHA_BLENDING.
        out.push(RectInstance {
            x: x + bw,
            y: y + header_h,
            w: (w - 2.0 * bw).max(0.0),
            h: (h - header_h - bw).max(0.0),
            color: rgba_alpha(darker(base, 0.12), 0.50),
        });

        // Divisória header/corpo.
        out.push(RectInstance {
            x: x + bw,
            y: y + header_h - 1.0,
            w: (w - 2.0 * bw).max(0.0),
            h: 1.0,
            color: rgba(darker(base, 0.18), 255),
        });

        // Indicador direito (3×8 px) perto do topo.
        out.push(RectInstance {
            x: x + w - bw - 6.0,
            y: y + 5.0,
            w: 3.0,
            h: 8.0,
            color: rgba(TEXT_PRIMARY[theme.clamp(0, 6)], 255),
        });

        // Culling vertical implícito: rects fora do raster são descartados
        // pelo wgpu (topology/clip), então não é preciso filtrar por y aqui.
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
pub fn wire(ui: &MainWindow, project: Rc<std::cell::RefCell<Project>>) {
    let Some(renderer) = TimelineGpuRenderer::new().ok() else {
        eprintln!("[timeline_canvas] Renderer indisponível — fundo desativado");
        return;
    };
    let renderer = Rc::new(renderer);

    // Estado de render-on-change (compare com o último rasterizado).
    let last: Rc<std::cell::RefCell<FrameKey>> =
        Rc::new(std::cell::RefCell::new(FrameKey::default()));

    let ui_shot = ui.clone_strong();
    let last_shot = last.clone();
    let project_shot = project.clone();

    ui.on_timeline_canvas_render_requested(move || {
        let ui = &ui_shot;

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
                track_index: track_index_of(c.track_id),
                beat: c.start_beat,
                len: c.length_beats,
                is_midi: c.is_midi(),
                color: c.color,
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
        };

        // Render-on-change: ignora se nada mudou.
        if *last_shot.borrow() == key {
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
        instances.extend(build_clip_instances(
            ruler_px,
            row_height,
            px_per_beat,
            theme as usize,
            &clip_visuals,
        ));

        let camera = OrthoCamera {
            scroll_x: key.scroll_x,
            scroll_y: key.scroll_y,
            viewport_w: w as f32,
            viewport_h: h as f32,
        };

        // Renderiza offscreen → readback.
        let Some(rgba) = renderer.render_frame(w, h, &camera, &instances) else {
            return;
        };

        // Converte RGBA → slint::Image → publicação.
        let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, w, h);
        let img = slint::Image::from_rgba8(buf);
        ui.set_timeline_canvas_background(img);
    });
}