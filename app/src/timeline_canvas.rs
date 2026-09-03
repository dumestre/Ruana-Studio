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

/// Cor (r,g,b) → RGBA opaco.
fn rgba((r, g, b): Rgb, a: u8) -> [u8; 4] {
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
// WIRING
// =====================================================

/// Cria o renderer e liga o callback de re-render do canvas.
pub fn wire(ui: &MainWindow) {
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
        let width = ui.get_timeline_canvas_content_width().max(0.0);
        let height = ui.get_timeline_canvas_content_height().max(0.0);
        let theme = ui.get_theme_index();

        let key = FrameKey {
            scroll_x: ui.get_timeline_scroll_x(),
            scroll_y: ui.get_timeline_scroll_y(),
            zoom: ui.get_zoom_level(),
            width: width as u32,
            height: height as u32,
            total_bars: ui.get_total_bars(),
            beats_per_bar: ui.get_beats_per_bar(),
            ruler_px,
            px_per_bar,
            theme,
        };

        // Render-on-change: ignora se nada mudou.
        if *last_shot.borrow() == key {
            return;
        }
        *last_shot.borrow_mut() = key;

        let w = key.width.max(1);
        let h = key.height.max(1);

        let instances = build_instances(
            w,
            h,
            key.total_bars,
            key.beats_per_bar,
            ruler_px,
            px_per_bar,
            theme,
        );

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