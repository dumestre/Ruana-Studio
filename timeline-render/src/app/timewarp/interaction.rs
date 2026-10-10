//! Estado e lógica de interação do timewarp: hover, arrasto/edição de pontos
//! de tempo (BPM), adição via ferramenta Draw e remoção via Erase/Delete.

use std::time::Instant;

use daw_core::WarpMarker;

use super::{TimewarpMetrics, TempoPoint};

/// Distância (px) para o cursor "agarrar" um marcador de warp (no eixo X).
const WARP_GRAB_PX: f32 = 6.0;

/// Janela (ms) para um segundo clique contar como duplo clique.
const WARP_DOUBLE_MS: u128 = 350;

/// Estado de interação do timewarp (hover + arrasto + scrub).
#[derive(Clone, Debug)]
pub struct TimewarpInteraction {
    pub hover_idx: Option<usize>,
    pub drag_idx: Option<usize>,
    pub drag_orig_beat: f32,
    pub drag_orig_bpm: f32,
    pub drag_start_cursor: (f32, f32),
    pub drag_live_beat: f32,
    pub drag_live_bpm: f32,
    pub drag_moved: bool,
    /// True enquanto o usuário arrasta o cursor na RÉGUA do timewarp.
    pub scrubbing: bool,

    /// ===== Editor de WARP (waveform do clip de áudio) =====
    /// Índice do marcador de AudioWarp sob o cursor.
    pub warp_hover: Option<usize>,
    /// Índice do marcador de AudioWarp em arrasto.
    pub warp_drag: Option<usize>,
    /// Offset de projeto ORIGINAL do marcador arrastado (beats locais).
    pub warp_drag_orig: f32,
    /// Offset de projeto ao vivo durante o arrasto (beats locais).
    pub warp_drag_live: f32,
    /// X inicial do cursor no arrasto (margem de "moved").
    pub warp_drag_start_x: f32,
    pub warp_drag_moved: bool,
    /// Último clique no corpo do warp: (instante, x) — duplo clique.
    pub warp_last_press: Option<(Instant, f32)>,
}

impl Default for TimewarpInteraction {
    fn default() -> Self {
        Self::new()
    }
}

impl TimewarpInteraction {
    pub fn new() -> Self {
        Self {
            hover_idx: None,
            drag_idx: None,
            drag_orig_beat: 0.0,
            drag_orig_bpm: 120.0,
            drag_start_cursor: (0.0, 0.0),
            drag_live_beat: 0.0,
            drag_live_bpm: 120.0,
            drag_moved: false,
            scrubbing: false,

            warp_hover: None,
            warp_drag: None,
            warp_drag_orig: 0.0,
            warp_drag_live: 0.0,
            warp_drag_start_x: 0.0,
            warp_drag_moved: false,
            warp_last_press: None,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_idx.is_some()
    }

    /// Hit-test: qual ponto de tempo está sob o cursor (raio de 7px).
    pub fn hit_test(
        &self,
        points: &[TempoPoint],
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) -> Option<usize> {
        const RADIUS: f32 = 7.0;
        for (i, p) in points.iter().enumerate() {
            let px = m.beat_to_x(p.beat);
            let py = m.bpm_to_y(p.bpm);
            let dx = cursor.0 - px;
            let dy = cursor.1 - py;
            if dx * dx + dy * dy <= RADIUS * RADIUS {
                return Some(i);
            }
        }
        None
    }

    /// Atualiza o hover conforme o cursor.
    pub fn update_hover(
        &mut self,
        points: &[TempoPoint],
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) {
        if self.is_dragging() {
            return;
        }
        self.hover_idx = self.hit_test(points, cursor, m);
    }

    /// Inicia o arrasto de um ponto de tempo.
    pub fn begin_drag(
        &mut self,
        points: &[TempoPoint],
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) -> bool {
        if let Some(idx) = self.hit_test(points, cursor, m) {
            let p = &points[idx];
            self.drag_idx = Some(idx);
            self.drag_orig_beat = p.beat;
            self.drag_orig_bpm = p.bpm;
            self.drag_start_cursor = cursor;
            self.drag_live_beat = p.beat;
            self.drag_live_bpm = p.bpm;
            self.drag_moved = false;
            true
        } else {
            false
        }
    }

    /// Move um ponto em arrasto. `snap` liga o grid musical (1/16) e BPM
    /// inteiro; sem snap, valor livre.
    pub fn drag_move(&mut self, cursor: (f32, f32), m: &TimewarpMetrics, snap: bool) {
        if self.drag_idx.is_none() {
            return;
        }
        let dx = cursor.0 - self.drag_start_cursor.0;
        let dy = cursor.1 - self.drag_start_cursor.1;

        if !self.drag_moved && dx.abs() <= 3.0 && dy.abs() <= 3.0 {
            return;
        }
        self.drag_moved = true;

        let (_, orig_bpm) = (self.drag_orig_beat, self.drag_orig_bpm);
        let mut beat = self.drag_orig_beat + dx / m.px_per_beat.max(0.001);
        if snap {
            beat = (beat * 4.0).round() / 4.0;
        }
        let mut bpm = self.y_to_bpm_from(orig_bpm, dy, m);
        if snap {
            bpm = bpm.round();
        }
        self.drag_live_beat = beat.max(0.0);
        self.drag_live_bpm = bpm;
    }

    /// BPM alvo a partir de um deslocamento vertical `dy` em px.
    fn y_to_bpm_from(&self, orig_bpm: f32, dy: f32, m: &TimewarpMetrics) -> f32 {
        let per_px = (super::BPM_MAX - super::BPM_MIN) / m.body_h.max(1.0);
        (orig_bpm - dy * per_px).clamp(super::BPM_MIN, super::BPM_MAX)
    }

    /// Finaliza o arrasto e aplica a nova posição.
    pub fn end_drag(&mut self, points: &mut [TempoPoint]) {
        if let Some(idx) = self.drag_idx {
            if self.drag_moved {
                if let Some(p) = points.get_mut(idx) {
                    p.beat = self.drag_live_beat.max(0.0);
                    p.bpm = self.drag_live_bpm.clamp(super::BPM_MIN, super::BPM_MAX);
                }
            }
        }
        self.drag_idx = None;
        self.drag_moved = false;
    }

    // ================= Editor de WARP (waveform) =================

    pub fn warp_is_dragging(&self) -> bool {
        self.warp_drag.is_some()
    }

    pub fn warp_moved_live(&self) -> bool {
        self.warp_drag.is_some() && self.warp_drag_moved
    }

    /// X de janela de um marcador de warp (offset LOCAL ao clip).
    fn warp_marker_x(offset: f32, clip_beat: f32, m: &TimewarpMetrics) -> f32 {
        m.beat_to_x(clip_beat + offset)
    }

    /// Hit-test do marcador de warp sob o cursor (só no eixo X, dentro do corpo).
    pub fn warp_hit_test(
        &self,
        markers: &[WarpMarker],
        clip_beat: f32,
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) -> Option<usize> {
        if cursor.1 < m.body_top - 4.0 || cursor.1 > m.body_top + m.body_h + 4.0 {
            return None;
        }
        for (i, mk) in markers.iter().enumerate() {
            let x = Self::warp_marker_x(mk.project_offset_beats, clip_beat, m);
            if (cursor.0 - x).abs() <= WARP_GRAB_PX {
                return Some(i);
            }
        }
        None
    }

    pub fn warp_update_hover(
        &mut self,
        markers: &[WarpMarker],
        clip_beat: f32,
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) {
        if self.warp_is_dragging() {
            return;
        }
        self.warp_hover = self.warp_hit_test(markers, clip_beat, cursor, m);
    }

    /// Inicia o arrasto de um marcador de warp sob o cursor. Devolve `true`
    /// se agarrou algum marcador.
    pub fn warp_begin_drag(
        &mut self,
        markers: &[WarpMarker],
        clip_beat: f32,
        cursor: (f32, f32),
        m: &TimewarpMetrics,
    ) -> bool {
        if let Some(idx) = self.warp_hit_test(markers, clip_beat, cursor, m) {
            let orig = markers[idx].project_offset_beats.max(0.0);
            self.warp_drag = Some(idx);
            self.warp_drag_orig = orig;
            self.warp_drag_live = orig;
            self.warp_drag_start_x = cursor.0;
            self.warp_drag_moved = false;
            true
        } else {
            false
        }
    }

    /// Move o marcador em arrasto (snap opcional ao grid musical de 1/16).
    pub fn warp_drag_move(&mut self, cursor: (f32, f32), m: &TimewarpMetrics, snap: bool, clip_len: f32) {
        if self.warp_drag.is_none() {
            return;
        }
        let dx = cursor.0 - self.warp_drag_start_x;
        if !self.warp_drag_moved && dx.abs() <= 3.0 {
            return;
        }
        self.warp_drag_moved = true;

        let mut offset = self.warp_drag_orig + dx / m.px_per_beat.max(0.001);
        if snap {
            offset = (offset * 4.0).round() / 4.0;
        }
        // Mantém o marcador DENTRO do clip, com folga para não colapsar.
        let hi = (clip_len - 1e-3).max(1e-3);
        self.warp_drag_live = offset.clamp(1e-3, hi);
    }

    /// Finaliza o arrasto do marcador; devolve `(índice, offset_live)` quando
    /// houve movimento real.
    pub fn warp_end_drag(&mut self) -> Option<(usize, f32)> {
        let result = match (self.warp_drag, self.warp_drag_moved) {
            (Some(idx), true) => Some((idx, self.warp_drag_live)),
            _ => None,
        };
        self.warp_drag = None;
        self.warp_drag_moved = false;
        result
    }

    /// Registra um clique no corpo do warp e devolve `true` quando é um
    /// DUPLO clique na mesma região (para criar um marcador).
    pub fn warp_register_press(&mut self, x: f32) -> bool {
        let now = Instant::now();
        let is_double = match self.warp_last_press {
            Some((t, last_x)) => {
                now.duration_since(t).as_millis() <= WARP_DOUBLE_MS
                    && (x - last_x).abs() <= 8.0
            }
            None => false,
        };
        self.warp_last_press = if is_double { None } else { Some((now, x)) };
        is_double
    }
}
