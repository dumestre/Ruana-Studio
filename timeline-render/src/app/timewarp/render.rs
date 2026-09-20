//! Geração de instâncias do timewarp: régua (reutilizada), corpo com grade
//! SÓ vertical, gutter de BPM, curvas de tempo (polilinhas lineares entre
//! eventos) e marcadores de evento estilo Cubase.

use daw_core::timeline::TempoMap;

use crate::app::piano_roll::build_bottom_ruler_instances;
use crate::{palette, RectInstance};
use super::{TimewarpInteraction, TimewarpMetrics, TempoPoint, BPM_LABEL_W};

/// Gera as instâncias da régua do timewarp (idêntica à do piano roll, mas
/// com a coluna do BPM no lugar do teclado e sem seletor de loop).
pub fn build_timewarp_ruler_instances(
    width: f32,
    px_per_beat: f32,
    scroll_x: f32,
    theme: usize,
    ruler_y: f32,
    ruler_h: f32,
    body_top: f32,
    body_h: f32,
    follow_playhead: bool,
) -> Vec<RectInstance> {
    build_bottom_ruler_instances(
        BPM_LABEL_W,
        false,
        width,
        px_per_beat,
        scroll_x,
        theme,
        ruler_y,
        ruler_h,
        body_top,
        body_h,
        false,
        0.0,
        0.0,
        follow_playhead,
    )
}

/// Instâncias completas do corpo do timewarp (grid + gutter + curva + pontos).
pub fn build_timewarp_instances(
    tempo_map: &TempoMap,
    inter: &TimewarpInteraction,
    m: &TimewarpMetrics,
    width: f32,
    _height: f32,
) -> Vec<RectInstance> {
    let t = m.theme.clamp(0, 6);
    let mut instances = Vec::new();

    // Corpo (grid vertical só) + gutter de BPM.
    instances.extend(m.build_body_instances(width));
    instances.extend(m.build_bpm_gutter_instances(_height));

    // Converte o mapa de tempo em pontos ordenados.
    let mut points: Vec<TempoPoint> = tempo_map
        .changes
        .iter()
        .map(|c| TempoPoint { beat: c.beat, bpm: c.bpm })
        .collect();
    points.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());

    // Curva: segmentos lineares (rampas) entre eventos consecutivos.
    push_curve(&mut instances, &points, m, t, width);

    // Linhas verticais finas de cada evento até a base do corpo.
    for p in &points {
        let x = m.beat_to_x(p.beat);
        if x < BPM_LABEL_W - 4.0 || x > width + 4.0 {
            continue;
        }
        let y = m.bpm_to_y(p.bpm);
        instances.push(RectInstance {
            x, y,
            w: 1.0, h: (m.body_top + m.body_h - y).max(0.0),
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 90],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });
    }

    // Marcadores (pontos de tempo).
    for (i, p) in points.iter().enumerate() {
        push_point_marker(&mut instances, p, i, inter, m);
    }

    instances
}

/// Desenha a polilinha da curva de tempo com tiles de ~3px (o pipeline só tem
/// `RectInstance` alinhados aos eixos — aproximação de linha por segmentos).
fn push_curve(
    instances: &mut Vec<RectInstance>,
    points: &[TempoPoint],
    m: &TimewarpMetrics,
    t: usize,
    width: f32,
) {
    if points.is_empty() {
        return;
    }
    let col = palette::ACCENT[t];
    let body_top = m.body_top;
    let body_bottom = m.body_top + m.body_h;

    for w in points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let (x0, y0) = (m.beat_to_x(a.beat), m.bpm_to_y(a.bpm));
        let (x1, y1) = (m.beat_to_x(b.beat), m.bpm_to_y(b.bpm));

        // Culling do segmento inteiro (fora do viewport).
        if (x0 < BPM_LABEL_W && x1 < BPM_LABEL_W) || (x0 > width && x1 > width) {
            continue;
        }

        let span = (x1 - x0).abs().max(1.0);
        let steps = (span / 3.0).ceil() as usize;
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            let x = x0 + (x1 - x0) * f;
            if x < BPM_LABEL_W || x > width {
                continue;
            }
            let y = y0 + (y1 - y0) * f;
            if y < body_top || y > body_bottom {
                continue;
            }
            instances.push(RectInstance {
                x, y: y - 1.0,
                w: 3.0, h: 2.0,
                color: [col.0, col.1, col.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
        }
    }
}

/// Desenha um marcador de ponto de tempo (quadrado com radius, como as notas
/// do piano roll) com estado de hover/seleção.
fn push_point_marker(
    instances: &mut Vec<RectInstance>,
    p: &TempoPoint,
    idx: usize,
    inter: &TimewarpInteraction,
    m: &TimewarpMetrics,
) {
    let t = m.theme.clamp(0, 6);
    let x = m.beat_to_x(p.beat) - 5.0;
    let y = m.bpm_to_y(p.bpm) - 5.0;
    if y + 10.0 < m.body_top || y > m.body_top + m.body_h {
        return;
    }

    let is_dragging = inter.drag_idx == Some(idx);
    let is_hovered = inter.hover_idx == Some(idx);
    let selected = is_dragging || is_hovered;

    let base = if selected {
        palette::ACCENT[t]
    } else {
        palette::CLIP_MIDI_DEFAULT[t]
    };

    // Corpo do marcador
    instances.push(RectInstance {
        x, y, w: 10.0, h: 10.0,
        color: [base.0, base.1, base.2, 255],
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 2.0,
    });

    // Borda
    let border_col = if selected { [0xFF, 0xFF, 0xFF, 255] } else { [0x1A, 0x1A, 0x1A, 255] };
    instances.push(RectInstance {
        x, y, w: 10.0, h: 1.0, color: border_col,
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });
    instances.push(RectInstance {
        x, y: y + 9.0, w: 10.0, h: 1.0, color: border_col,
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });
    instances.push(RectInstance {
        x, y, w: 1.0, h: 10.0, color: border_col,
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });
    instances.push(RectInstance {
        x: x + 9.0, y, w: 1.0, h: 10.0, color: border_col,
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });
}