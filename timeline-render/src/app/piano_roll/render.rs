//! Geração de instâncias do piano roll: notas, preview de arrasto, marquee,
//! splitter e indicador de hover. Reutiliza `RectInstance` e a paleta do
//! `app::render`.

use crate::{rgba, rgba_alpha, palette, RectInstance, build_text_instances};
use super::{PianoRollInteraction, PianoRollMetrics, PianoRollNote, PIANO_RULER_H};

/// Altura fixa de cada nota em pixels.
pub const NOTE_HEIGHT: f32 = 12.0;

/// Gera as instâncias de render do piano roll completo.
pub fn build_piano_roll_instances(
    notes: &[PianoRollNote],
    inter: &PianoRollInteraction,
    m: &PianoRollMetrics,
    width: f32,
    height: f32,
) -> Vec<RectInstance> {
    let mut instances = Vec::new();

    // Grade de fundo
    instances.extend(m.build_grid_instances(width, height));

     // Teclado do piano
    instances.extend(m.build_keyboard_instances(width, height));

    // Notas
    for (idx, note) in notes.iter().enumerate() {
        push_note(&mut instances, note, idx, inter, m);
    }

    // Nota em arrasto (ghost)
    if let (Some(idx), true) = (inter.drag_idx, inter.drag_moved) {
        if let Some(clip) = notes.get(idx) {
            let live_note = PianoRollNote {
                id: clip.id,
                pitch: inter.drag_live_pitch,
                start: inter.drag_live_start,
                length: clip.length,
                velocity: clip.velocity,
                selected: true,
            };
            push_note(&mut instances, &live_note, idx, inter, m);
        }
    }

    // Preview de desenho contínuo
    if inter.draw_active {
        let start_beat = inter.draw_start.0.min(inter.draw_current.0);
        let end_beat = inter.draw_start.0.max(inter.draw_current.0);
        let len = (end_beat - start_beat).max(0.25);
        let min_pitch = inter.draw_start.1.min(inter.draw_current.1).max(m.bottom_pitch);
        let max_pitch = inter.draw_start.1.max(inter.draw_current.1).min(m.top_pitch);

        for pitch in min_pitch..=max_pitch {
            let preview = PianoRollNote {
                id: -1,
                pitch,
                start: start_beat,
                length: len,
                velocity: 100,
                selected: false,
            };
            push_note_preview(&mut instances, &preview, m);
        }
    }

    // Marquee overlay
    if inter.marquee_active {
        let (mx, my, mw, mh) = inter.marquee_rect();
        if mw > 0.0 && mh > 0.0 {
            instances.push(RectInstance {
                x: mx,
                y: my,
                w: mw,
                h: mh,
                color: [0x26, 0x26, 0x26, 60],
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });

            let thin = 1.0;
            let border_col = [0x26, 0x26, 0x26, 255];
            instances.push(RectInstance {
                x: mx,
                y: my,
                w: mw,
                h: thin,
                color: border_col,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
            instances.push(RectInstance {
                x: mx,
                y: my + mh - thin,
                w: mw,
                h: thin,
                color: border_col,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
            instances.push(RectInstance {
                x: mx,
                y: my,
                w: thin,
                h: mh,
                color: border_col,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
            instances.push(RectInstance {
                x: mx + mw - thin,
                y: my,
                w: thin,
                h: mh,
                color: border_col,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
        }
    }

    // Playhead (vertical line)
    instances.push(RectInstance {
        x: m.beat_to_x(0.0),
        y: m.ruler_px,
        w: 1.0,
        h: m.roll_height,
        color: [
            palette::PLAYHEAD_COLOR.0,
            palette::PLAYHEAD_COLOR.1,
            palette::PLAYHEAD_COLOR.2,
            180,
        ],
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });

    instances
}

/// Gera as instâncias da régua do piano roll (réplica da régua da timeline,
/// com playhead próprio). Fica entre a splitter e o conteúdo do piano roll.
/// Usa os mesmos `px_per_beat` e `scroll_x` da timeline.
pub fn build_piano_ruler_instances(
    m: &PianoRollMetrics,
    width: f32,
    transport_beat: f32,
    px_per_beat: f32,
    scroll_x: f32,
    theme: usize,
) -> Vec<RectInstance> {
    let t = theme.clamp(0, 6);
    let mut instances = Vec::new();

    let ruler_y = m.ruler_px - PIANO_RULER_H; // topo da régua (ruler_px é o topo do conteúdo)
    let ruler_h = PIANO_RULER_H;
    let ruler_bottom = ruler_y + ruler_h;

    // Fundo da régua do piano roll (mesmo estilo da timeline)
    instances.push(RectInstance {
        x: 0.0,
        y: ruler_y,
        w: width,
        h: ruler_h,
        color: [palette::SURFACE_HIGH[t].0, palette::SURFACE_HIGH[t].1, palette::SURFACE_HIGH[t].2, 255],
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });

    // Réplica da régua da timeline: fundos alternados dos compassos
    let px_per_bar = px_per_beat * 4.0;
    let total_bars = ((width + scroll_x) / px_per_bar).ceil() as usize + 1;
    for i in 0..total_bars {
        let bx = i as f32 * px_per_bar - scroll_x;
        if bx > width + px_per_bar || bx < -px_per_bar {
            continue;
        }
        let bw = px_per_bar;
        let bg = if i % 2 == 1 {
            palette::SURFACE_HIGH[t]
        } else {
            palette::SURFACE_STEP[t]
        };
        if bx + bw > 0.0 && bx < width && bw > 0.0 {
            let draw_x = bx.max(0.0);
            let draw_w = (bx + bw).min(width) - draw_x;
            if draw_w > 0.0 {
                instances.push(RectInstance {
                    x: draw_x, y: ruler_y, w: draw_w, h: ruler_h,
                    color: [bg.0, bg.1, bg.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
    }

    // Linhas de batida (1/4) — mesma densidade da timeline
    if px_per_bar >= 40.0 {
        let n = 4u32;
        let step = px_per_bar / n as f32;
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - scroll_x;
            if bx > width || bx < -px_per_bar { continue; }
            for k in 0..n {
                let x = bx + k as f32 * step;
                if x <= 0.0 || x > width { continue; }
                instances.push(RectInstance {
                    x, y: ruler_y + 2.0, w: 1.0, h: ruler_h - 4.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
    }

    // Números de compasso (mesmo estilo da timeline)
    for i in 0..total_bars {
        let bx = i as f32 * px_per_bar - scroll_x;
        if bx > width || bx < -px_per_bar { continue; }
        let ruler_text_color = [palette::TEXT_PRIMARY[t].0, palette::TEXT_PRIMARY[t].1, palette::TEXT_PRIMARY[t].2, 220];
        if bx > -px_per_bar * 0.5 && bx < width {
            instances.extend(build_text_instances(&format!("{}", i + 1), bx + 4.0, ruler_y + 3.0, ruler_text_color, 1.0));
        }
    }

    // Playhead do piano roll (independente da timeline)
    let head_x = transport_beat * px_per_beat - scroll_x;
    if head_x >= -20.0 && head_x <= width + 20.0 {
        let col = palette::PLAYHEAD_COLOR;
        instances.push(RectInstance {
            x: head_x - 1.0, y: ruler_y,
            w: 2.0, h: ruler_h,
            color: [col.0, col.1, col.2, 180],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });
    }

    // Borda inferior da régua do PR
    instances.push(RectInstance {
        x: 0.0, y: ruler_bottom - 1.0,
        w: width, h: 1.0,
        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    instances
}

/// Gera as instâncias do splitter entre timeline e piano roll.
pub fn build_splitter_instances(
    width: f32,
    piano_top: f32,
    _roll_height: f32,
    _height: f32,
    hovered: bool,
    theme: usize,
) -> Vec<RectInstance> {
    let t = theme.clamp(0, 6);
    let splitter_y = piano_top;
    let splitter_h = 4.0;
    let col = if hovered {
        palette::ACCENT[t]
    } else {
        palette::BASE[t]
    };

    vec![RectInstance {
        x: 0.0,
        y: splitter_y,
        w: width,
        h: splitter_h,
        color: [col.0, col.1, col.2, 180],
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    }]
}

/// Desenha uma única nota no piano roll.
fn push_note(
    instances: &mut Vec<RectInstance>,
    note: &PianoRollNote,
    idx: usize,
    inter: &PianoRollInteraction,
    m: &PianoRollMetrics,
) {
    let t = m.theme.clamp(0, 6);
    let (x, y, w, h) = note_rect(note, m, NOTE_HEIGHT);

    if w <= 0.0 || x + w < 0.0 || x > m.px_per_beat * 1000.0 {
        return;
    }

    let is_selected = idx < inter.selected.len() && inter.selected[idx];
    let is_hovered = inter.hover_idx == Some(idx);

    // Cor baseada na velocity
    let vel_norm = (note.velocity as f32 / 127.0).clamp(0.0, 1.0);
    let base = if is_selected {
        palette::ACCENT[t]
    } else if is_hovered {
        palette::CLIP_MIDI_DEFAULT[(t + 1).clamp(0, 6)]
    } else {
        palette::CLIP_MIDI_DEFAULT[t]
    };

    let r = (base.0 as f32 * vel_norm) as u8;
    let g = (base.1 as f32 * vel_norm) as u8;
    let b = (base.2 as f32 * vel_norm) as u8;

    let body_col = rgba((r, g, b), 200);

    // Corpo da nota
    instances.push(RectInstance {
        x,
        y,
        w,
        h,
        color: body_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 1.0,
    });

    // Borda de seleção
    if is_selected {
        let border_col = [0x26, 0x26, 0x26, 255];
        let thin = 2.0;
        instances.push(RectInstance {
            x,
            y,
            w,
            h: thin,
            color: border_col,
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 1.0,
        });
        instances.push(RectInstance {
            x,
            y: y + h - thin,
            w,
            h: thin,
            color: border_col,
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 1.0,
        });
        instances.push(RectInstance {
            x,
            y,
            w: thin,
            h,
            color: border_col,
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });
        instances.push(RectInstance {
            x: x + w - thin,
            y,
            w: thin,
            h,
            color: border_col,
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });
    }
}

/// Desenha a pré-visualização de uma nota em desenho contínuo.
fn push_note_preview(
    instances: &mut Vec<RectInstance>,
    note: &PianoRollNote,
    m: &PianoRollMetrics,
) {
    let t = m.theme.clamp(0, 6);
    let (x, y, w, h) = note_rect(note, m, NOTE_HEIGHT);

    let base = palette::ACCENT[t];
    let preview_col = rgba_alpha(base, 0.3);

    instances.push(RectInstance {
        x,
        y,
        w,
        h,
        color: preview_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 1.0,
    });

    let border_col = [0x26, 0x26, 0x26, 180];
    let thin = 1.0;
    instances.push(RectInstance {
        x,
        y,
        w,
        h: thin,
        color: border_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 1.0,
    });
    instances.push(RectInstance {
        x,
        y: y + h - thin,
        w,
        h: thin,
        color: border_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 1.0,
    });
    instances.push(RectInstance {
        x,
        y,
        w: thin,
        h,
        color: border_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });
    instances.push(RectInstance {
        x: x + w - thin,
        y,
        w: thin,
        h,
        color: border_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });
}

/// Retorna o retângulo de uma nota em coordenadas de janela.
pub fn note_rect(
    note: &PianoRollNote,
    m: &PianoRollMetrics,
    note_height: f32,
) -> (f32, f32, f32, f32) {
    let (x, y) = m.beat_pitch_to_xy(note.start, note.pitch);
    let w = note.length * m.px_per_beat;
    (x, y, w, note_height)
}
