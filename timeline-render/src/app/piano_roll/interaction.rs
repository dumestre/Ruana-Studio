//! Estado e lógica de interação do piano roll: hover, arrasto de notas,
//! marquee, desenho contínuo e seleção.

use super::{PianoRollMetrics, PianoRollNote, NOTE_HEIGHT};

/// Estado de interação do piano roll (hover + arrasto + seleção).
#[derive(Clone, Debug)]
pub struct PianoRollInteraction {
    pub hover_idx: Option<usize>,
    pub drag_idx: Option<usize>,
    pub drag_orig_start: f32,
    pub drag_orig_pitch: i32,
    pub drag_orig_length: f32,
    pub drag_start_cursor: (f32, f32),
    pub drag_live_start: f32,
    pub drag_live_pitch: i32,
    pub drag_moved: bool,
    pub selected: Vec<bool>,
    pub marquee_active: bool,
    pub marquee_start: (f32, f32),
    pub marquee_cur: (f32, f32),
    pub draw_active: bool,
    pub draw_start: (f32, i32),
    pub draw_current: (f32, i32),
}

impl Default for PianoRollInteraction {
    fn default() -> Self {
        Self::new()
    }
}

impl PianoRollInteraction {
    pub fn new() -> Self {
        Self {
            hover_idx: None,
            drag_idx: None,
            drag_orig_start: 0.0,
            drag_orig_pitch: 0,
            drag_orig_length: 0.0,
            drag_start_cursor: (0.0, 0.0),
            drag_live_start: 0.0,
            drag_live_pitch: 0,
            drag_moved: false,
            selected: Vec::new(),
            marquee_active: false,
            marquee_start: (0.0, 0.0),
            marquee_cur: (0.0, 0.0),
            draw_active: false,
            draw_start: (0.0, 0),
            draw_current: (0.0, 0),
        }
    }

    pub fn sync_selection(&mut self, note_count: usize) {
        if self.selected.len() != note_count {
            self.selected.resize(note_count, false);
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_idx.is_some()
    }

    pub fn did_drag_move(&self) -> bool {
        self.drag_moved
    }

    pub fn is_marqueeing(&self) -> bool {
        self.marquee_active
    }

    pub fn is_drawing(&self) -> bool {
        self.draw_active
    }

    pub fn select_only(&mut self, idx: usize) {
        for s in self.selected.iter_mut() {
            *s = false;
        }
        if idx < self.selected.len() {
            self.selected[idx] = true;
        }
    }

    /// Hit-test: qual nota está sob o cursor (ordem reversa — última por cima).
    pub fn hit_test(
        &self,
        notes: &[PianoRollNote],
        cursor: (f32, f32),
        m: &PianoRollMetrics,
    ) -> Option<usize> {
        for (i, note) in notes.iter().enumerate().rev() {
            let (nx, ny, nw, nh) = note_rect(note, m, NOTE_HEIGHT);
            if cursor.0 >= nx && cursor.0 <= nx + nw && cursor.1 >= ny && cursor.1 <= ny + nh {
                return Some(i);
            }
        }
        None
    }

    /// Atualiza o hover conforme o cursor.
    pub fn update_hover(
        &mut self,
        notes: &[PianoRollNote],
        cursor: (f32, f32),
        m: &PianoRollMetrics,
    ) {
        if self.is_dragging() || self.is_drawing() {
            return;
        }
        self.hover_idx = self.hit_test(notes, cursor, m);
    }

    /// Inicia arrasto de uma nota.
    pub fn begin_drag(
        &mut self,
        notes: &[PianoRollNote],
        cursor: (f32, f32),
        m: &PianoRollMetrics,
    ) -> bool {
        if let Some(idx) = self.hit_test(notes, cursor, m) {
            let note = &notes[idx];
            self.drag_idx = Some(idx);
            self.drag_orig_start = note.start;
            self.drag_orig_pitch = note.pitch;
            self.drag_orig_length = note.length;
            self.drag_start_cursor = cursor;
            self.drag_live_start = note.start;
            self.drag_live_pitch = note.pitch;
            self.drag_moved = false;
            true
        } else {
            false
        }
    }

    /// Move uma nota em arrasto.
    pub fn drag_move(&mut self, cursor: (f32, f32), m: &PianoRollMetrics) {
        if self.drag_idx.is_none() {
            return;
        }

        let dx = cursor.0 - self.drag_start_cursor.0;
        let dy = cursor.1 - self.drag_start_cursor.1;

        if !self.drag_moved && dx.abs() <= 3.0 && dy.abs() <= 3.0 {
            return;
        }

        self.drag_moved = true;

        let snap_step = 4.0 / 16.0; // 1/16 de compasso
        let delta_beats = (dx / m.px_per_beat.max(0.001) / snap_step).round() * snap_step;
        let delta_pitch = (-dy / m.key_height).round() as i32;

        self.drag_live_start = (self.drag_orig_start + delta_beats).max(0.0);
        self.drag_live_pitch = (self.drag_orig_pitch + delta_pitch)
            .clamp(m.bottom_pitch, m.top_pitch);
    }

    /// Finaliza o arrasto e aplica a nova posição.
    pub fn end_drag(&mut self, notes: &mut [PianoRollNote]) {
        if let Some(idx) = self.drag_idx {
            if self.drag_moved {
                if let Some(note) = notes.get_mut(idx) {
                    note.start = self.drag_live_start;
                    note.pitch = self.drag_live_pitch;
                }
            }
        }

        self.drag_idx = None;
        self.drag_moved = false;
    }

    /// Inicia marquee.
    pub fn begin_marquee(&mut self, cursor: (f32, f32)) {
        self.marquee_active = true;
        self.marquee_start = cursor;
        self.marquee_cur = cursor;
    }

    /// Atualiza marquee.
    pub fn marquee_move(&mut self, cursor: (f32, f32)) {
        if self.marquee_active {
            self.marquee_cur = cursor;
        }
    }

    /// Finaliza marquee e seleciona notas dentro da área.
    pub fn end_marquee(
        &mut self,
        notes: &[PianoRollNote],
        m: &PianoRollMetrics,
    ) {
        if !self.marquee_active {
            return;
        }

        let (mx, my, mw, mh) = self.marquee_rect();

        for s in self.selected.iter_mut() {
            *s = false;
        }

        if mw <= 3.0 && mh <= 3.0 {
            self.marquee_active = false;
            return;
        }

        for (i, note) in notes.iter().enumerate() {
            let (nx, ny, nw, nh) = note_rect(note, m, NOTE_HEIGHT);
            let overlap_x = nx < mx + mw && nx + nw > mx;
            let overlap_y = ny < my + mh && ny + nh > my;
            if overlap_x && overlap_y && i < self.selected.len() {
                self.selected[i] = true;
            }
        }

        self.marquee_active = false;
    }

    /// Retorna o retângulo do marquee normalizado.
    pub fn marquee_rect(&self) -> (f32, f32, f32, f32) {
        let (sx, sy) = self.marquee_start;
        let (cx, cy) = self.marquee_cur;
        (
            sx.min(cx),
            sy.min(cy),
            (sx - cx).abs(),
            (sy - cy).abs(),
        )
    }

    /// Inicia desenho contínuo (criar notas ao arrastar).
    pub fn begin_draw(&mut self, cursor: (f32, f32), m: &PianoRollMetrics) {
        let (beat, pitch) = m.cursor_to_beat_pitch(cursor);
        let snapped_start = snap_beat(beat, m);
        let snapped_pitch = snap_pitch(pitch);
        self.draw_active = true;
        self.draw_start = (snapped_start, snapped_pitch);
        self.draw_current = (snapped_start, snapped_pitch);
    }

    /// Atualiza desenho contínuo.
    pub fn draw_move(&mut self, cursor: (f32, f32), m: &PianoRollMetrics) {
        if self.draw_active {
            let (beat, pitch) = m.cursor_to_beat_pitch(cursor);
            let snapped_start = snap_beat(beat, m);
            let snapped_pitch = snap_pitch(pitch);
            self.draw_current = (snapped_start, snapped_pitch);
        }
    }

    /// Finaliza desenho e retorna IDs das notas criadas.
    pub fn end_draw(
        &mut self,
        notes: &mut Vec<PianoRollNote>,
        m: &PianoRollMetrics,
    ) -> Vec<usize> {
        let mut created = Vec::new();
        if self.draw_active {
            let start_beat = self.draw_start.0.min(self.draw_current.0);
            let end_beat = self.draw_start.0.max(self.draw_current.0);
            let len = (end_beat - start_beat).max(0.25);

            let min_pitch = self.draw_start.1.min(self.draw_current.1).max(m.bottom_pitch);
            let max_pitch = self.draw_start.1.max(self.draw_current.1).min(m.top_pitch);

            for pitch in min_pitch..=max_pitch {
                let note = PianoRollNote {
                    id: -(created.len() as i32) - 1,
                    pitch,
                    start: start_beat,
                    length: len,
                    velocity: 100,
                    selected: false,
                };
                notes.push(note);
                created.push(notes.len() - 1);
            }

            self.sync_selection(notes.len());
            for &i in &created {
                if i < self.selected.len() {
                    self.selected[i] = true;
                }
            }
        }
        self.draw_active = false;
        created
    }
}

/// Retângulo de uma nota em coordenadas de janela.
pub fn note_rect(
    note: &PianoRollNote,
    m: &PianoRollMetrics,
    note_height: f32,
) -> (f32, f32, f32, f32) {
    let (x, y) = m.beat_pitch_to_xy(note.start, note.pitch);
    let w = note.length * m.px_per_beat;
    (x, y, w, note_height)
}

/// Snap do beat ao grid (1/16 = 0.25 beat).
pub fn snap_beat(beat: f32, _m: &PianoRollMetrics) -> f32 {
    (beat * 4.0).round() / 4.0
}

/// Snap do pitch ao grid (inteiro).
pub fn snap_pitch(pitch: i32) -> i32 {
    pitch
}
