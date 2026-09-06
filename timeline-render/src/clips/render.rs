//! Geração de instâncias de render dos clips (corpo, header, waveform,
//! fantasma, bordas e overlays de arrasto/seleção).

use crate::{
    darker,
    rgba,
    rgba_alpha,
    palette,
    RectInstance,
    build_text_instances,
};
use super::{clip_rect, ClipDemo, ClipInteraction, ClipMetrics};

/// Desenha o nome do clip no header.
fn push_clip_name(
    instances: &mut Vec<RectInstance>,
    clip: &ClipDemo,
    x: f32,
    y: f32,
    w: f32,
    header_h: f32,
    bw: f32,
) {
    if let Some(ref name) = clip.name {
        let scale = 1.0;
        let char_w = 8.0 * scale;
        let gap = 1.0 * scale;

        let total_w =
            (name.len() as f32 - 1.0) * gap
                + name.len() as f32 * char_w;

        let available =
            w - 2.0 * bw;

        if total_w <= available {
            let text_y =
                y + header_h / 2.0
                    - 4.0 * scale;

            let start_x =
                x + bw + 4.0;

            let center_x =
                start_x + total_w / 2.0;

            let text_color =
                [0u8, 0, 0, 255];

            instances.extend(
                build_text_instances(
                    name,
                    center_x,
                    text_y,
                    text_color,
                    scale,
                ),
            );
        }
    }
}

/// Gera os retângulos de todos os clips.
pub fn build_clip_instances(
    clips: &[ClipDemo],
    inter: &ClipInteraction,
    m: &ClipMetrics,
    width: f32,
) -> Vec<RectInstance> {
    let t =
        m.theme.clamp(0, 6) as usize;

    let header_h = 16.0;
    let bw = 1.0;

    let body_alpha_normal = 0.50;
    let body_alpha_hover = 0.32;
    let body_alpha_ghost = 0.08;
    let body_alpha_overlay = 0.40;

    let mut instances = Vec::new();

    for (idx, clip) in clips.iter().enumerate() {
        let base =
            clip_color(clip, t);

        let (x, y, w, _h) =
            clip_rect(clip, m);

        if w <= 0.0
            || x + w < 0.0
            || x > width
        {
            continue;
        }

        let draw_w =
            w.min(width - x).max(0.0);

        let in_group =
            inter.drag_group
                .iter()
                .any(|(gi, _, _)| *gi == idx);

        let being_dragged =
            in_group
                && inter.drag_moved
                && !inter.stretch_mode;

        let in_marquee =
            if inter.marquee_active {
                let (mx, my, mw, mh) =
                    inter.marquee_rect();

                x < mx + mw
                    && x + w > mx
                    && y < my + mh
                    && y + m.row_height > my
            } else {
                false
            };

        let body_alpha =
            if being_dragged {
                body_alpha_ghost
            } else if inter.hover_idx == Some(idx)
                || in_marquee
            {
                body_alpha_hover
            } else {
                body_alpha_normal
            };

        let body_dark =
            if being_dragged {
                0.70
            } else {
                0.12
            };

        let wave_natural_w =
            clip.pre_glue_len
                .unwrap_or(clip.len)
                * m.px_per_beat;

        push_clip(
            &mut instances,
            clip,
            base,
            x,
            y,
            w,
            draw_w,
            header_h,
            bw,
            body_alpha,
            body_dark,
            &m.wave_segments,
            idx,
            t,
            m.row_height,
            wave_natural_w,
        );

        push_clip_name(
            &mut instances,
            clip,
            x,
            y,
            w,
            header_h,
            bw,
        );

        // Borda de seleção.
        if !being_dragged
            && (idx < inter.selected.len()
                && inter.selected[idx]
                || in_marquee)
        {
            let ac =
                [0x26, 0x26, 0x26, 255];

            let thin = 2.0;

            instances.push(RectInstance {
                x,
                y,
                w,
                h: thin,
                color: ac,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 3.0,
            });

            instances.push(RectInstance {
                x,
                y: y + m.row_height - thin,
                w,
                h: thin,
                color: ac,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 3.0,
            });

            instances.push(RectInstance {
                x,
                y,
                w: thin,
                h: m.row_height,
                color: ac,
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
                h: m.row_height,
                color: ac,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
        }

        // Seam.
        if let Some(seam_beat) =
            clip.seam_beat
        {
            let seam_x =
                seam_beat * m.px_per_beat
                    - m.scroll_x;

            if seam_x >= x
                && seam_x <= x + draw_w
            {
                let seam_col =
                    (0x26, 0x26, 0x26);

                let is_hovered =
                    inter.seam_hover_idx
                        == Some(idx);

                let alpha: u8 =
                    if is_hovered {
                        200
                    } else {
                        110
                    };

                let dash_w = 2.0;
                let dash_h = 5.0;
                let gap = 3.0;

                let mut dy = y;

                while dy + dash_h
                    <= y + m.row_height
                {
                    instances.push(
                        RectInstance {
                            x: seam_x
                                - dash_w / 2.0,
                            y: dy,
                            w: dash_w,
                            h: dash_h,
                            color: [
                                seam_col.0,
                                seam_col.1,
                                seam_col.2,
                                alpha,
                            ],
                            wave_offset: 0,
                            wave_count: 0,
                            clip_w: 0.0,
                            clip_h: 0.0,
                            radius: 0.0,
                        },
                    );

                    dy += dash_h + gap;
                }
            }
        }
    }

    // ============================================================
    // OVERLAY DO CLIP EM ARRASTO
    // ============================================================

    if let (Some(idx), true) =
        (inter.drag_idx, inter.drag_moved)
    {
        if let Some(clip) =
            clips.get(idx)
        {
            let live =
                if inter.stretch_mode {
                    ClipDemo {
                        track_index:
                            inter.drag_orig_track,
                        beat:
                            inter.drag_orig_beat,
                        len:
                            inter.stretch_live_len,
                        is_midi:
                            clip.is_midi,
                        color:
                            clip.color,
                        name:
                            clip.name.clone(),
                        seam_beat:
                            clip.seam_beat,
                        pre_glue_len:
                            clip.pre_glue_len,
                        pre_glue_color:
                            clip.pre_glue_color,
                        wave:
                            clip.wave.clone(),
                        file_path:
                            clip.file_path.clone(),
                        source_start_beats:
                            clip.source_start_beats,
                        source_length_beats:
                            clip.source_length_beats,
                        channels:
                            clip.channels,
                    }
                } else {
                    ClipDemo {
                        track_index:
                            inter.drag_live_track,
                        beat:
                            inter.drag_live_beat,
                        len:
                            clip.len,
                        is_midi:
                            clip.is_midi,
                        color:
                            clip.color,
                        name:
                            clip.name.clone(),
                        seam_beat:
                            clip.seam_beat,
                        pre_glue_len:
                            clip.pre_glue_len,
                        pre_glue_color:
                            clip.pre_glue_color,
                        wave:
                            clip.wave.clone(),
                        file_path:
                            clip.file_path.clone(),
                        source_start_beats:
                            clip.source_start_beats,
                        source_length_beats:
                            clip.source_length_beats,
                        channels:
                            clip.channels,
                    }
                };

            let (x, y, w, _h) =
                clip_rect(&live, m);

            let wave_natural_w =
                if inter.stretch_mode {
                    w
                } else {
                    live.pre_glue_len
                        .unwrap_or(live.len)
                        * m.px_per_beat
                };

            if w > 0.0
                && x < width
                && x + w > 0.0
            {
                let draw_w =
                    w.min(width - x).max(0.0);

                let base =
                    clip_color(clip, t);

                push_clip(
                    &mut instances,
                    &live,
                    base,
                    x,
                    y,
                    w,
                    draw_w,
                    header_h,
                    bw,
                    body_alpha_overlay,
                    0.12,
                    &m.wave_segments,
                    idx,
                    t,
                    m.row_height,
                    wave_natural_w,
                );

                push_clip_name(
                    &mut instances,
                    &live,
                    x,
                    y,
                    w,
                    header_h,
                    bw,
                );

                let border_col =
                    (0x26, 0x26, 0x26);

                let th = 2.0;

                instances.push(RectInstance {
                    x,
                    y,
                    w: draw_w,
                    h: th,
                    color: [
                        border_col.0,
                        border_col.1,
                        border_col.2,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 3.0,
                });

                instances.push(RectInstance {
                    x,
                    y: y + m.row_height - th,
                    w: draw_w,
                    h: th,
                    color: [
                        border_col.0,
                        border_col.1,
                        border_col.2,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });

                instances.push(RectInstance {
                    x,
                    y,
                    w: th,
                    h: m.row_height,
                    color: [
                        border_col.0,
                        border_col.1,
                        border_col.2,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });

                instances.push(RectInstance {
                    x: x + draw_w - th,
                    y,
                    w: th,
                    h: m.row_height,
                    color: [
                        border_col.0,
                        border_col.1,
                        border_col.2,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });
            }
        }

        // ========================================================
        // OVERLAY DOS MEMBROS DO GRUPO
        // ========================================================

        if !inter.stretch_mode
            && inter.drag_moved
            && !inter.drag_group.is_empty()
        {
            let beat_delta =
                inter.drag_live_beat
                    - inter.drag_orig_beat;

            let track_delta =
                inter.drag_live_track as i32
                    - inter.drag_orig_track as i32;

            let min_group_beat =
                inter.drag_group
                    .iter()
                    .map(|(_, ob, _)| *ob)
                    .fold(inter.drag_orig_beat, f32::min);

            let clamped_delta =
                beat_delta.max(-min_group_beat);

            for (gi, orig_beat, orig_track)
                in inter.drag_group.iter()
            {
                if Some(*gi) == inter.drag_idx {
                    continue;
                }

                if let Some(clip) =
                    clips.get(*gi)
                {
                    let live_beat =
                        (orig_beat + clamped_delta)
                            .max(0.0);

                    // Aplica o mesmo delta de track clampado dos limites do
                    // grupo (calculado no drag_move), preservando a colagem.
                    let live_track =
                        (*orig_track as i32
                            + track_delta)
                            .max(0) as usize;

                    let live = ClipDemo {
                        track_index:
                            live_track,
                        beat:
                            live_beat,
                        len:
                            clip.len,
                        is_midi:
                            clip.is_midi,
                        color:
                            clip.color,
                        name:
                            clip.name.clone(),
                        seam_beat:
                            clip.seam_beat,
                        pre_glue_len:
                            clip.pre_glue_len,
                        pre_glue_color:
                            clip.pre_glue_color,
                        wave:
                            clip.wave.clone(),
                        file_path:
                            clip.file_path.clone(),
                        source_start_beats:
                            clip.source_start_beats,
                        source_length_beats:
                            clip.source_length_beats,
                        channels:
                            clip.channels,
                    };

                    let (x, y, w, _h) =
                        clip_rect(&live, m);

                    if w > 0.0
                        && x < width
                        && x + w > 0.0
                    {
                        let draw_w =
                            w.min(width - x)
                                .max(0.0);

                        let base =
                            clip_color(
                                clip,
                                t,
                            );

                        let wave_natural_w =
                            clip.pre_glue_len
                                .unwrap_or(
                                    clip.len,
                                )
                                * m.px_per_beat;

                        push_clip(
                            &mut instances,
                            &live,
                            base,
                            x,
                            y,
                            w,
                            draw_w,
                            header_h,
                            bw,
                            body_alpha_overlay,
                            0.12,
                            &m.wave_segments,
                            *gi,
                            t,
                            m.row_height,
                            wave_natural_w,
                        );

                        push_clip_name(
                            &mut instances,
                            &live,
                            x,
                            y,
                            w,
                            header_h,
                            bw,
                        );

                        let border_col =
                            (0x26, 0x26, 0x26);

                        let th = 2.0;

                        instances.push(
                            RectInstance {
                                x,
                                y,
                                w: draw_w,
                                h: th,
                                color: [
                                    border_col.0,
                                    border_col.1,
                                    border_col.2,
                                    255,
                                ],
                                wave_offset: 0,
                                wave_count: 0,
                                clip_w: 0.0,
                                clip_h: 0.0,
                                radius: 3.0,
                            },
                        );
                    }
                }
            }
        }
    }

    // ============================================================
    // DESENHO CONTÍNUO
    // ============================================================

    if let Some(draw_state) =
        &inter.draw_state
    {
        if draw_state.active {
            let start_beat =
                draw_state
                    .start_beat
                    .min(draw_state.current_beat);

            let end_beat =
                draw_state
                    .start_beat
                    .max(draw_state.current_beat);

            let len =
                (end_beat - start_beat)
                    .max(0.25);

            let track =
                draw_state
                    .start_track
                    .min(draw_state.current_track);

            let temp_clip = ClipDemo {
                track_index: track,
                beat: start_beat,
                len,
                is_midi: true,
                color: 0,
                name: None,
                seam_beat: None,
                pre_glue_len: None,
                pre_glue_color: None,
                wave: Vec::new(),
                file_path: None,
                source_start_beats: 0.0,
                source_length_beats: 0.0,
                channels: 1,
            };

            let (x, y, w, _h) =
                clip_rect(&temp_clip, m);

            if w > 0.0
                && x < width
                && x + w > 0.0
            {
                let draw_w =
                    w.min(width - x)
                        .max(0.0);

                let base =
                    clip_color(
                        &temp_clip,
                        t,
                    );

                push_clip(
                    &mut instances,
                    &temp_clip,
                    base,
                    x,
                    y,
                    w,
                    draw_w,
                    header_h,
                    bw,
                    body_alpha_overlay,
                    0.12,
                    &m.wave_segments,
                    clips.len(),
                    t,
                    m.row_height,
                    w,
                );
            }
        }
    }

    // ============================================================
    // MARQUEE
    // ============================================================

    if inter.marquee_active {
        let (mx, my, mw, mh) =
            inter.marquee_rect();

        if mw > 0.0 && mh > 0.0 {
            instances.push(
                RectInstance {
                    x: mx,
                    y: my,
                    w: mw,
                    h: mh,
                    color: [
                        0x26,
                        0x26,
                        0x26,
                        60,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                },
            );

            let thin = 1.0;

            instances.push(
                RectInstance {
                    x: mx,
                    y: my,
                    w: mw,
                    h: thin,
                    color: [
                        0x26,
                        0x26,
                        0x26,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                },
            );

            instances.push(
                RectInstance {
                    x: mx,
                    y: my + mh - thin,
                    w: mw,
                    h: thin,
                    color: [
                        0x26,
                        0x26,
                        0x26,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                },
            );

            instances.push(
                RectInstance {
                    x: mx,
                    y: my,
                    w: thin,
                    h: mh,
                    color: [
                        0x26,
                        0x26,
                        0x26,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                },
            );

            instances.push(
                RectInstance {
                    x: mx + mw - thin,
                    y: my,
                    w: thin,
                    h: mh,
                    color: [
                        0x26,
                        0x26,
                        0x26,
                        255,
                    ],
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                },
            );
        }
    }

    // ============================================================
    // OVERLAP
    // ============================================================

    for i in 0..clips.len() {
        for j in (i + 1)..clips.len() {
            let (xi, yi, wi, hi) =
                clip_rect(&clips[i], m);

            let (xj, yj, wj, hj) =
                clip_rect(&clips[j], m);

            let ix0 = xi.max(xj);
            let ix1 =
                (xi + wi).min(xj + wj);

            let iy0 = yi.max(yj);
            let iy1 =
                (yi + hi).min(yj + hj);

            if ix1 > ix0
                && iy1 > iy0
            {
                let ov =
                    (0xFFu8, 0x8Cu8, 0x00u8);

                instances.push(
                    RectInstance {
                        x: ix0,
                        y: iy0,
                        w: ix1 - ix0,
                        h: iy1 - iy0,
                        color: [
                            ov.0,
                            ov.1,
                            ov.2,
                            220,
                        ],
                        wave_offset: 0,
                        wave_count: 0,
                        clip_w: -1.0,
                        clip_h: 0.0,
                        radius: 0.0,
                    },
                );
            }
        }
    }

    instances
}

fn clip_color(
    clip: &ClipDemo,
    t: usize,
) -> palette::Rgb {
    if clip.color == 0 {
        if clip.is_midi {
            palette::CLIP_MIDI_DEFAULT[t]
        } else {
            palette::CLIP_AUDIO_DEFAULT
        }
    } else {
        palette::CLIP_PALETTE[
            clip.color
                % palette::CLIP_PALETTE.len()
        ]
    }
}

#[allow(clippy::too_many_arguments)]
fn push_clip(
    instances: &mut Vec<RectInstance>,
    clip: &ClipDemo,
    base: palette::Rgb,
    x: f32,
    y: f32,
    w: f32,
    draw_w: f32,
    header_h: f32,
    bw: f32,
    body_alpha: f32,
    dark_amount: f32,
    segs: &[(u32, u32)],
    idx: usize,
    _t: usize,
    row_height: f32,
    _wave_natural_w: f32,
) {
    let clip_r = 3.0;

    let decor_alpha: u8 =
        if body_alpha < 0.15 {
            ((body_alpha * 255.0 * 6.0)
                .round() as u8)
                .min(180)
        } else {
            255
        };

    let (wo, wc) =
        if clip.is_midi {
            (0u32, 0u32)
        } else if let Some(&(o, c)) =
            segs.get(idx)
        {
            (o, c)
        } else {
            (0u32, 0u32)
        };

    // ============================================================
    // CORPO DO CLIP
    //
    // IMPORTANTE:
    // O corpo agora é UM ÚNICO RETÂNGULO.
    //
    // A antiga implementação dividia o corpo em:
    //   mono   -> 3 retângulos
    //   stereo -> 5 retângulos
    //
    // Isso foi removido.
    //
    // A waveform continua sendo desenhada separadamente abaixo,
    // mantendo exatamente as posições verticais anteriores.
    // ============================================================

    let body_h =
        (row_height - header_h).max(0.0);

    let body_col =
        rgba_alpha(
            darker(base, dark_amount),
            body_alpha,
        );

    // Corpo – retângulo inferior com cantos arredondados apenas na parte de baixo.
    // A altura do corpo é a altura total da track menos a altura do header.
    let body_rect_h = (row_height - header_h).max(0.0);
    instances.push(RectInstance {
        x: x + bw,
        y: y + header_h,
        w: w,
        h: body_rect_h,
        color: body_col,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: body_rect_h,
        radius: clip_r,
    });

    // Header – retângulo superior com cantos arredondados (apenas canto superior).
    // Usa a altura fixa do header (header_h) e garante que o total (header + corpo)
    // preencha exatamente a altura da track (row_height).
    let header_rect_h = header_h;
    instances.push(RectInstance {
        x: x + bw,
        y,
        w: w,
        h: header_rect_h,
        color: body_col, // mesmo tom usado no corpo
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: header_rect_h,
        radius: clip_r,
    });

    // ============================================================
    // WAVEFORM
    //
    // Mantém as posições verticais originais:
    //
    // MONO:
    //   corpo dividido conceitualmente em 3 partes
    //   waveform ocupava o terço central.
    //
    // STEREO:
    //   corpo dividido conceitualmente em 5 partes
    //   L ocupava o 2º quinto.
    //   R ocupava o 4º quinto.
    //
    // Agora essas divisões NÃO são mais usadas para formar o fundo.
    // Elas só determinam a posição/tamanho da waveform.
    // ============================================================

    let is_stereo =
        clip.channels == 2
            && !clip.is_midi
            && body_h > 40.0;

    if is_stereo {
        let fifth =
            body_h / 5.0;

        // --------------------------------------------------------
        // Waveform L — mesma posição anterior
        // --------------------------------------------------------

        instances.push(
            RectInstance {
                x: x + bw,
                y: y
                    + header_h
                    + fifth,
                w,
                h: fifth,
                color: body_col,

                wave_offset: wo,
                wave_count: wc,

                // Sinaliza stereo L para o shader.
                clip_w: -4.0,
                clip_h: fifth,

                radius: clip_r,
            },
        );

        // --------------------------------------------------------
        // Waveform R — mesma posição anterior
        // --------------------------------------------------------

        instances.push(
            RectInstance {
                x: x + bw,
                y: y
                    + header_h
                    + 3.0 * fifth,
                w,
                h: fifth,
                color: body_col,

                wave_offset: wo,
                wave_count: wc,

                // Sinaliza stereo R para o shader.
                clip_w: -5.0,
                clip_h: fifth,

                radius: clip_r,
            },
        );
    } else {
        // --------------------------------------------------------
        // Waveform MONO — mesma posição anterior
        // --------------------------------------------------------

        let third =
            body_h / 3.0;

        instances.push(
            RectInstance {
                x: x + bw,
                y: y
                    + header_h
                    + third,
                w,
                h: third,
                color: body_col,

                wave_offset: wo,
                wave_count: wc,

                // Sinaliza mono-center para o shader.
                clip_w: -6.0,
                clip_h: third,

                radius: clip_r,
            },
        );
    }

    // ============================================================
    // HEADER
    // ============================================================

    instances.push(RectInstance {
        x: x + bw,
        y: y + bw,
        w: (draw_w - 2.0 * bw).max(0.0),
        h: (header_h - bw).max(0.0),
        color: rgba(base, decor_alpha),
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: clip_r,
    });

    // ============================================================
    // DIVISÓRIA HEADER / CORPO
    // ============================================================

    instances.push(RectInstance {
        x: x + bw,
        y: y + header_h - 1.0,
        w: (draw_w - 2.0 * bw).max(0.0),
        h: 1.0,
        color: rgba(
            darker(base, 0.18),
            decor_alpha,
        ),
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });

    // ============================================================
    // INDICADOR DIREITO
    // ============================================================

    instances.push(RectInstance {
        x: x + draw_w - bw - 6.0,
        y: y + 5.0,
        w: 3.0,
        h: 8.0,
        color: rgba(
            palette::TEXT_PRIMARY[_t],
            decor_alpha,
        ),
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });

    // ============================================================
    // GHOST — posição original durante drag
    // ============================================================

    if body_alpha < 0.15 {
        let ghost_border =
            rgba(
                darker(base, 0.15),
                55,
            );

        let th = 1.0;

        // Superior.
        instances.push(
            RectInstance {
                x,
                y,
                w: draw_w,
                h: th,
                color: ghost_border,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: clip_r,
            },
        );

        // Inferior.
        instances.push(
            RectInstance {
                x,
                y: y + row_height - th,
                w: draw_w,
                h: th,
                color: ghost_border,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            },
        );

        // Esquerda.
        instances.push(
            RectInstance {
                x,
                y,
                w: th,
                h: row_height,
                color: ghost_border,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            },
        );

        // Direita.
        instances.push(
            RectInstance {
                x: x + draw_w - th,
                y,
                w: th,
                h: row_height,
                color: ghost_border,
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            },
        );
    }
}
