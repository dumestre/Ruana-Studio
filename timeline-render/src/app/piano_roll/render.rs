//! Geração de instâncias do piano roll: notas, preview de arrasto, marquee,
//! splitter e indicador de hover. Reutiliza `RectInstance` e a paleta do
//! `app::render`.

use crate::{rgba, rgba_alpha, palette, RectInstance};
use super::{PianoRollInteraction, PianoRollMetrics, PianoRollNote};

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

/// Gera as instâncias do splitter entre timeline e piano roll.
pub fn build_splitter_instances(
    width: f32,
    height: f32,
    _ruler_px: f32,
    roll_height: f32,
    hovered: bool,
    theme: usize,
) -> Vec<RectInstance> {
    let t = theme.clamp(0, 6);
    let splitter_y = height - roll_height - 4.0;
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
