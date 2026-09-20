//! Geração de instâncias do piano roll: notas, preview de arrasto, marquee,
//! splitter e indicador de hover. Reutiliza `RectInstance` e a paleta do
//! `app::render`.

use crate::{rgba, rgba_alpha, palette, RectInstance, build_text_instances};
use super::{PianoRollInteraction, PianoRollMetrics, PianoRollNote, KEYBOARD_W};

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

    instances
}

/// Régua do piano roll: wrapper da régua compartilhada com `x_offset = teclado`
/// e seletor de loop ativado. Mantém a assinatura original (usada por
/// `app::piano_roll_instances`).
pub fn build_piano_ruler_instances(
    _m: &PianoRollMetrics,
    width: f32,
    _pr_playhead_beat: f32,
    px_per_beat: f32,
    scroll_x: f32,
    theme: usize,
    ruler_y: f32,
    ruler_h: f32,
    body_top: f32,
    body_h: f32,
    loop_enabled: bool,
    loop_start_beat: f32,
    loop_end_beat: f32,
    follow_playhead: bool,
) -> Vec<RectInstance> {
    build_bottom_ruler_instances(
        KEYBOARD_W,
        true,
        width,
        px_per_beat,
        scroll_x,
        theme,
        ruler_y,
        ruler_h,
        body_top,
        body_h,
        loop_enabled,
        loop_start_beat,
        loop_end_beat,
        follow_playhead,
    )
}

/// Ruler compartilhada das áreas expandidas (piano roll e timewarp).
///
/// Idêntica em estrutura: fundo, compassos alternados, números, subdivisões,
/// indicador FOLLOW e, quando `with_loop`, o seletor de loop (fill + alças).
/// `x_offset` é a coluna esquerda (teclado do piano roll ou gutter de BPM do
/// timewarp). O playhead é desenhado por último pela área (z-order).
pub fn build_bottom_ruler_instances(
    x_offset: f32,
    with_loop: bool,
    width: f32,
    px_per_beat: f32,
    scroll_x: f32,
    theme: usize,
    ruler_y: f32,
    ruler_h: f32,
    body_top: f32,
    body_h: f32,
    loop_enabled: bool,
    loop_start_beat: f32,
    loop_end_beat: f32,
    follow_playhead: bool,
) -> Vec<RectInstance> {
    let t = theme.clamp(0, 6);
    let mut instances = Vec::new();

    let ruler_bottom = ruler_y + ruler_h;
    let px_per_bar = px_per_beat * 4.0;
    let total_bars = ((width + scroll_x) / px_per_bar).ceil() as usize + 1;

    // Helper: converte beat → x considerando x_offset + scroll_x
    let beat_to_x = |beat: f32| -> f32 {
        x_offset + beat * px_per_beat - scroll_x
    };

    // ===== Fundo base da régua =====
    instances.push(RectInstance {
        x: 0.0,
        y: ruler_y,
        w: width,
        h: ruler_h,
        color: rgba(palette::SURFACE_HIGH[t], 255),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // ===== Fundos alternados dos compassos (par/ímpar) — a partir da coluna =====
    for i in 0..total_bars {
        let bx = x_offset + i as f32 * px_per_bar - scroll_x;
        if bx > width + px_per_bar || bx < -px_per_bar { continue; }
        let bg = if i % 2 == 1 { palette::SURFACE_HIGH[t] } else { palette::SURFACE_STEP[t] };
        let bw = px_per_bar;
        let draw_x = bx.max(x_offset);
        let draw_w = (bx + bw).min(width) - draw_x;
        if draw_w > 0.0 {
            instances.push(RectInstance {
                x: draw_x, y: ruler_y, w: draw_w, h: ruler_h,
                color: [bg.0, bg.1, bg.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
        }
    }

    // ===== Linhas de compasso + números + subdivisões =====
    for i in 0..total_bars {
        let bx = x_offset + i as f32 * px_per_bar - scroll_x;
        if bx > width || bx < -px_per_bar { continue; }

        // Número do compasso
        let ruler_text_color = rgba(palette::TEXT_PRIMARY[t], 220);
        if bx > -px_per_bar * 0.5 && bx < width {
            instances.extend(build_text_instances(&format!("{}", i + 1), bx + 4.0, ruler_y + 3.0, ruler_text_color, 1.0));
        }
        // Linha vertical do compasso (altura cheia)
        if bx >= x_offset && bx <= width {
            instances.push(RectInstance {
                x: bx, y: ruler_y, w: 1.0, h: ruler_h,
                color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
        }
        // Batidas (1/4) — quando px_per_bar >= 40
        let beat_px = px_per_bar / 4.0;
        for k in 0..4 {
            let x = bx + k as f32 * beat_px;
            if x < x_offset || x > width { continue; }
            if px_per_bar >= 40.0 {
                instances.push(RectInstance {
                    x, y: 2.0 + ruler_y, w: 1.0, h: ruler_h - 2.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
        // Subdivisão 1/8 (tiques menores na parte de baixo) — zoom >= 120
        if px_per_bar >= 120.0 {
            let n = 8u32;
            let step = px_per_bar / n as f32;
            for m in 1..n {
                let x8 = bx + m as f32 * step;
                if x8 < x_offset || x8 > width { continue; }
                instances.push(RectInstance {
                    x: x8, y: ruler_y + ruler_h - 8.0, w: 1.0, h: 8.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
        // Subdivisão 1/16 (zoom >= 240)
        if px_per_bar >= 240.0 {
            let n = 16u32;
            let step = px_per_bar / n as f32;
            for m in 1..n {
                let x16 = bx + m as f32 * step;
                if x16 < x_offset || x16 > width { continue; }
                instances.push(RectInstance {
                    x: x16, y: ruler_y + ruler_h - 6.0, w: 1.0, h: 6.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
        // Subdivisão 1/32 (zoom >= 480)
        if px_per_bar >= 480.0 {
            let n = 32u32;
            let step = px_per_bar / n as f32;
            for m in 1..n {
                let x32 = bx + m as f32 * step;
                if x32 < x_offset || x32 > width { continue; }
                instances.push(RectInstance {
                    x: x32, y: ruler_y + ruler_h - 5.0, w: 1.0, h: 5.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 90],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
        // Subdivisão 1/64 (zoom >= 960)
        if px_per_bar >= 960.0 {
            let n = 64u32;
            let step = px_per_bar / n as f32;
            for m in 1..n {
                let x64 = bx + m as f32 * step;
                if x64 < x_offset || x64 > width { continue; }
                instances.push(RectInstance {
                    x: x64, y: ruler_y + ruler_h - 4.0, w: 1.0, h: 4.0,
                    color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 70],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
    }

    // Linha vertical na borda direita da coluna (marca início do compasso 1)
    instances.push(RectInstance {
        x: x_offset, y: ruler_y, w: 1.0, h: ruler_h,
        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // ===== Indicador FOLLOW playhead (canto superior direito da régua) =====
    if follow_playhead {
        let px = width - 120.0;
        let py = ruler_y + 2.0;
        instances.extend(build_text_instances("FOLLOW", px, py, rgba(palette::ACCENT[t], 255), 1.0));
    }

    // ===== A PLAYHEAD NÃO É DESENHADA AQUI (ordem de z-order). =====
    // Ela é desenhada NO FIM da montagem da área (app/mod.rs), DEPOIS do
    // grid/notas/curva. Se desenharmos aqui, esses elementos cobrem a linha.

    // ===== Seletor de loop — handles + fill na ruler + fill + bordas no corpo =====
    if with_loop && loop_enabled {
        let r: (u8, u8, u8) = (0x3A, 0x3A, 0x3A);
        let border = [r.0, r.1, r.2, 255];
        let fill_ruler = [r.0, r.1, r.2, 45];
        let fill_body = [r.0, r.1, r.2, 70];
        let border_body = [r.0, r.1, r.2, 185];
        let sx = beat_to_x(loop_start_beat);
        let ex = beat_to_x(loop_end_beat);
        let lw = (ex - sx).max(0.0);
        let bw = 6.0;

        if lw > 0.0 {
            // --- Fill no CORPO do piano roll (abaixo da régua) ---
            instances.push(RectInstance {
                x: sx, y: body_top, w: lw, h: body_h.max(0.0),
                color: fill_body,
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
            // --- Fill na RÉGUA ---
            if sx + lw > 0.0 && sx < width {
                instances.push(RectInstance {
                    x: sx, y: ruler_y, w: lw.min(width - sx).max(0.0), h: ruler_h,
                    color: fill_ruler,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
            // --- Bordas laterais grossas NA RÉGUA (handles de arrasto) ---
            if sx + lw >= -bw && sx <= width {
                instances.push(RectInstance {
                    x: sx, y: ruler_y, w: bw, h: ruler_h, color: border,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
            if ex >= 0.0 && ex - bw <= width {
                instances.push(RectInstance {
                    x: ex - bw, y: ruler_y, w: bw, h: ruler_h, color: border,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
            // --- Bordas laterais descendo pelo CORPO ---
            if sx + lw >= -bw && sx <= width {
                instances.push(RectInstance {
                    x: sx, y: body_top, w: 2.0, h: body_h.max(0.0), color: border_body,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
            if ex >= 0.0 && ex - bw <= width {
                instances.push(RectInstance {
                    x: ex - 2.0, y: body_top, w: 2.0, h: body_h.max(0.0), color: border_body,
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }
    }

    // ===== Borda inferior da régua =====
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
