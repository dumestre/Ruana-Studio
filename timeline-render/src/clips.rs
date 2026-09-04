//! Clips da timeline: estruturas de dados, estado de hover/arrasto e
//! geração dos retângulos de render (corpo, header, waveform, fantasma e
//! overlay do clip em arrasto).

use super::{rgba, rgba_alpha, darker, RectInstance, palette};

/// Um clip posicionado numa faixa (track) da timeline.
#[derive(Clone, Copy, Debug)]
pub struct ClipDemo {
    pub track_index: usize,
    pub beat: f32,
    pub len: f32,
    pub is_midi: bool,
    pub color: usize,
}

/// Métricas de conversão cursor ↔ conteúdo, espelhadas do App.
#[derive(Clone, Copy)]
pub struct ClipMetrics {
    pub px_per_beat: f32,
    pub row_height: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub ruler_px: f32,
    pub theme: usize,
}

impl ClipMetrics {
    /// Converte o cursor da janela para o (beat, track) do conteúdo.
    fn cursor_to_beat_track(&self, cursor: (f32, f32)) -> (f32, usize) {
        let content_x = cursor.0 + self.scroll_x;
        let content_y = cursor.1 - self.ruler_px + self.scroll_y;
        let beat = if self.px_per_beat > 0.0 {
            content_x / self.px_per_beat
        } else {
            0.0
        };
        let track = if self.row_height > 0.0 {
            (content_y / self.row_height).floor().max(0.0) as usize
        } else {
            0
        };
        (beat, track)
    }
}

/// Retângulo de um clip em coordenadas de janela (já descontando scroll).
fn clip_rect(clip: &ClipDemo, m: &ClipMetrics) -> (f32, f32, f32, f32) {
    let x = clip.beat * m.px_per_beat - m.scroll_x;
    let w = (clip.len * m.px_per_beat).max(40.0);
    let y = m.ruler_px + clip.track_index as f32 * m.row_height - m.scroll_y;
    (x, y, w, m.row_height)
}

/// Estado de interação (hover + arrasto) sobre os clips.
#[derive(Clone, Debug)]
pub struct ClipInteraction {
    pub hover_idx: Option<usize>,
    pub drag_idx: Option<usize>,
    pub drag_orig_beat: f32,
    pub drag_orig_track: usize,
    pub drag_start_cursor: (f32, f32),
    pub drag_live_beat: f32,
    pub drag_live_track: usize,
    pub drag_moved: bool,
}

impl ClipInteraction {
    pub fn new() -> Self {
        Self {
            hover_idx: None,
            drag_idx: None,
            drag_orig_beat: 0.0,
            drag_orig_track: 0,
            drag_start_cursor: (0.0, 0.0),
            drag_live_beat: 0.0,
            drag_live_track: 0,
            drag_moved: false,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_idx.is_some()
    }

    /// Hit-test: qual clip está sob o cursor, na ordem (último por cima).
    pub fn hit_test(&self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) -> Option<usize> {
        let (beat, track) = m.cursor_to_beat_track(cursor);
        clips.iter().enumerate().rev().find(|(_, c)| {
            c.track_index == track && beat >= c.beat && beat < c.beat + c.len
        }).map(|(i, _)| i)
    }

    /// Atualiza o hover conforme o cursor (quando não arrastando).
    pub fn update_hover(&mut self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) {
        if self.is_dragging() {
            return;
        }
        self.hover_idx = self.hit_test(clips, cursor, m);
    }

    /// Inicia um possível arrasto: captura a origem e o ponto de pega.
    pub fn begin_drag(&mut self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) -> bool {
        if let Some(idx) = self.hit_test(clips, cursor, m) {
            let clip = &clips[idx];
            self.drag_idx = Some(idx);
            self.drag_orig_beat = clip.beat;
            self.drag_orig_track = clip.track_index;
            self.drag_start_cursor = cursor;
            self.drag_live_beat = clip.beat;
            self.drag_live_track = clip.track_index;
            self.drag_moved = false;
            self.hover_idx = None;
            true
        } else {
            false
        }
    }

    /// Move o clip em arrasto preservando o ponto de pega (delta desde o DOWN),
    /// com snap de 1 beat e snap de linha inteira, e limiar de 3px.
    pub fn drag_move(&mut self, cursor: (f32, f32), m: &ClipMetrics, snap: bool) {
        if self.drag_idx.is_none() {
            return;
        }
        let dx = cursor.0 - self.drag_start_cursor.0;
        let dy = cursor.1 - self.drag_start_cursor.1;
        if !self.drag_moved && dx.abs() <= 3.0 && dy.abs() <= 3.0 {
            return; // ainda não passou do limiar — clique simples não arrasta
        }
        self.drag_moved = true;

        let raw_beat = self.drag_orig_beat + dx / m.px_per_beat.max(0.001);
        let live_beat = if snap {
            raw_beat.max(0.0).round()
        } else {
            raw_beat.max(0.0)
        };
        self.drag_live_beat = live_beat;

        let row_delta = (dy / m.row_height.max(0.001)).round();
        let n_tracks = 4;
        let live_track = (self.drag_orig_track as f32 + row_delta).clamp(0.0, (n_tracks - 1) as f32) as usize;
        self.drag_live_track = live_track;
    }

    /// Termina o arrasto e aplica a nova posição no modelo (commit no release).
    pub fn end_drag(&mut self, clips: &mut [ClipDemo], m: &ClipMetrics, snap: bool) {
        if let Some(idx) = self.drag_idx {
            if self.drag_moved {
                self.drag_move(self.drag_start_cursor, m, snap); // recalcula live final
                clips[idx].beat = self.drag_live_beat;
                clips[idx].track_index = self.drag_live_track;
            }
        }
        self.drag_idx = None;
        self.drag_moved = false;
    }
}

/// Gera os retângulos de todos os clips, com hover/fantasma/overlay.
pub fn build_clip_instances(
    clips: &[ClipDemo],
    inter: &ClipInteraction,
    m: &ClipMetrics,
    wave_bars: &[f32],
    width: f32,
) -> Vec<RectInstance> {
    let t = m.theme.clamp(0, 6) as usize;
    let header_h = 16.0;
    let bw = 1.0;

    // Parametrização de transparências do corpo do clip.
    let body_alpha_normal = 0.50;
    let body_alpha_hover = 0.32;
    let body_alpha_ghost = 0.30; // fantasma na posição antiga: apagado e translúcido
    let body_alpha_overlay = 0.28; // clip sob o mouse durante o arrasto: transparente

    let mut instances = Vec::new();

    for (idx, clip) in clips.iter().enumerate() {
        let base = clip_color(clip, t);
        let (x, y, w, _h) = clip_rect(clip, m);
        if w <= 0.0 || x + w < 0.0 || x > width {
            continue;
        }
        let draw_w = w.min(width - x).max(0.0);

        // O clip em arrasto (com `moved`) não é desenhado na origem: em seu
        // lugar fica o fantasma translúcido; o atual segue o mouse via overlay.
        let being_dragged = inter.drag_idx == Some(idx) && inter.drag_moved;

        let body_alpha = if being_dragged {
            body_alpha_ghost
        } else if inter.hover_idx == Some(idx) {
            body_alpha_hover
        } else {
            body_alpha_normal
        };
        let body_dark = if being_dragged { 0.70 } else { 0.12 };

        push_clip(&mut instances, clips, clip, base, x, y, w, draw_w, header_h, bw,
                  body_alpha, body_dark, wave_bars, idx, t, m.row_height);
    }

    // Overlay: clip seguindo o mouse durante o arrasto (moved).
    if let (Some(idx), true) = (inter.drag_idx, inter.drag_moved) {
        if let Some(clip) = clips.get(idx) {
            let live = ClipDemo {
                track_index: inter.drag_live_track,
                beat: inter.drag_live_beat,
                len: clip.len,
                is_midi: clip.is_midi,
                color: clip.color,
            };
            let (x, y, w, _h) = clip_rect(&live, m);
            if w > 0.0 && x < width && x + w > 0.0 {
                let draw_w = w.min(width - x).max(0.0);
                let base = clip_color(clip, t);
                push_clip(&mut instances, clips, &live, base, x, y, w, draw_w, header_h, bw,
                          body_alpha_overlay, 0.12, wave_bars, idx, t, m.row_height);
            }
        }
    }

    instances
}

fn clip_color(clip: &ClipDemo, t: usize) -> palette::Rgb {
    if clip.color == 0 {
        if clip.is_midi {
            palette::CLIP_MIDI_DEFAULT[t]
        } else {
            palette::CLIP_AUDIO_DEFAULT
        }
    } else {
        palette::CLIP_PALETTE[clip.color % palette::CLIP_PALETTE.len()]
    }
}

#[allow(clippy::too_many_arguments)]
fn push_clip(
    instances: &mut Vec<RectInstance>,
    clips: &[ClipDemo],
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
    wave_bars: &[f32],
    idx: usize,
    t: usize,
    row_height: f32,
) {
    // Header
    instances.push(RectInstance {
        x: x + bw, y: y + bw,
        w: (draw_w - 2.0 * bw).max(0.0),
        h: (header_h - bw).max(0.0),
        color: rgba(base, 255),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // Waveform (só para clips de áudio)
    let (wo, wc) = if clip.is_midi {
        (0u32, 0u32)
    } else {
        let bar_idx = clips[..idx].iter().filter(|c| !c.is_midi).count();
        let offset = bar_idx as u32 * 2;
        let total = wave_bars.len() as u32 / 2;
        (offset, total)
    };
    let body_h = row_height - header_h;

    // Corpo (com waveform) — alpha e escurecimento controlados por hover/drag.
    instances.push(RectInstance {
        x: x + bw, y: y + header_h,
        w: w, h: body_h,
        color: rgba_alpha(darker(base, dark_amount), body_alpha),
        wave_offset: wo, wave_count: wc, clip_w: w, clip_h: body_h, radius: 0.0,
    });

    // Divisória
    instances.push(RectInstance {
        x: x + bw, y: y + header_h - 1.0,
        w: (draw_w - 2.0 * bw).max(0.0), h: 1.0,
        color: rgba(darker(base, 0.18), 255),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // Indicador direito
    instances.push(RectInstance {
        x: x + draw_w - bw - 6.0, y: y + 5.0,
        w: 3.0, h: 8.0,
        color: rgba(palette::TEXT_PRIMARY[t], 255),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });
}
