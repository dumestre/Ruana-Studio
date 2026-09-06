//! Estado e lógica de interação com os clips: hover, arrasto, stretch,
//! marquee, desenho contínuo e glue.

use super::{clip_rect, ClipDemo, ClipMetrics};

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
    pub selected: Vec<bool>,
    pub marquee_active: bool,
    pub marquee_start: (f32, f32),
    pub marquee_cur: (f32, f32),
    pub draw_state: Option<DrawToolState>,
    pub stretch_edge_idx: Option<usize>,
    pub stretch_mode: bool,
    pub stretch_orig_len: f32,
    pub stretch_live_len: f32,
    pub seam_hover_idx: Option<usize>,
    pub drag_group: Vec<(usize, f32, usize)>,
}

/// Estado de desenho contínuo com a ferramenta de desenho (tecla 2).
#[derive(Clone, Debug)]
pub struct DrawToolState {
    pub active: bool,
    pub start_beat: f32,
    pub start_track: usize,
    pub current_beat: f32,
    pub current_track: usize,
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
            selected: Vec::new(),
            marquee_active: false,
            marquee_start: (0.0, 0.0),
            marquee_cur: (0.0, 0.0),
            draw_state: None,
            stretch_edge_idx: None,
            stretch_mode: false,
            stretch_orig_len: 0.0,
            stretch_live_len: 0.0,
            seam_hover_idx: None,
            drag_group: Vec::new(),
        }
    }

    /// Garante que o vetor de seleção tenha o tamanho da lista de clips.
    pub fn sync_selection(&mut self, clip_count: usize) {
        if self.selected.len() != clip_count {
            self.selected.resize(clip_count, false);
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_idx.is_some()
    }

    pub fn is_marqueeing(&self) -> bool {
        self.marquee_active
    }

    pub fn is_drawing(&self) -> bool {
        self.draw_state
            .as_ref()
            .map_or(false, |s| s.active)
    }

    pub fn is_stretching(&self) -> bool {
        self.stretch_mode
    }

    pub fn is_seam_hovered(&self) -> bool {
        self.seam_hover_idx.is_some()
    }

    /// Seleciona apenas o clip `idx`, apagando as demais seleções.
    pub fn select_only(&mut self, idx: usize) {
        for s in self.selected.iter_mut() {
            *s = false;
        }

        if idx < self.selected.len() {
            self.selected[idx] = true;
        }
    }

    /// Seleciona todos os clips da cadeia de glue (grupo).
    pub fn select_chain(
        &mut self,
        _clips: &[ClipDemo],
        chain: &[usize],
    ) {
        for s in self.selected.iter_mut() {
            *s = false;
        }

        for &i in chain {
            if i < self.selected.len() {
                self.selected[i] = true;
            }
        }
    }

    /// Des-cola um clip: restaura o len original (abre o gap) e devolve
    /// a cor original do clip da frente. O seam é removido.
    pub fn unglue_seam(
        &mut self,
        clips: &mut Vec<ClipDemo>,
        idx: usize,
    ) {
        if clips[idx].seam_beat.is_some() {
            if let Some(orig_len) = clips[idx].pre_glue_len {
                let seam_beat = clips[idx].seam_beat.unwrap();
                let track = clips[idx].track_index;

                // Restaura cor original do clip da frente.
                for c in clips.iter_mut() {
                    if c.track_index == track
                        && (c.beat - seam_beat).abs() < 0.01
                        && c.pre_glue_color.is_some()
                    {
                        c.color = c.pre_glue_color.unwrap();
                        c.pre_glue_color = None;
                    }
                }

                // Restaura len original do clip atual.
                clips[idx].len = orig_len;
                clips[idx].seam_beat = None;
                clips[idx].pre_glue_len = None;

                self.select_only(idx);
            }
        }
    }

    /// Hit-test: qual clip está sob o cursor, na ordem (último por cima).
    pub fn hit_test(
        &self,
        clips: &[ClipDemo],
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) -> Option<usize> {
        let (beat, track) = m.cursor_to_beat_track(cursor);

        clips
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| {
                c.track_index == track
                    && beat >= c.beat
                    && beat < c.beat + c.len
            })
            .map(|(i, _)| i)
    }

    /// Retângulo do marquee em coordenadas normalizadas.
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

    /// Atualiza o hover conforme o cursor.
    pub fn update_hover(
        &mut self,
        clips: &[ClipDemo],
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) {
        if self.is_dragging() || self.is_drawing() {
            self.stretch_edge_idx = None;
            self.seam_hover_idx = None;
            return;
        }

        self.hover_idx = self.hit_test(clips, cursor, m);

        // Detecta se o cursor está perto da borda direita.
        self.stretch_edge_idx = self.hover_idx.and_then(|idx| {
            let clip = &clips[idx];

            let (cx, _cy, cw, _ch) = clip_rect(clip, m);

            if cursor.0 >= cx + cw - 12.0 {
                Some(idx)
            } else {
                None
            }
        });

        // Detecta hover sobre seam.
        self.seam_hover_idx =
            clips.iter().enumerate().rev().find_map(
                |(idx, clip)| {
                    if let Some(seam_beat) = clip.seam_beat {
                        let seam_x =
                            seam_beat * m.px_per_beat - m.scroll_x;

                        let (_, cy, _, ch) =
                            clip_rect(clip, m);

                        if cursor.0 >= seam_x - 8.0
                            && cursor.0 <= seam_x + 8.0
                            && cursor.1 >= cy
                            && cursor.1 <= cy + ch
                        {
                            return Some(idx);
                        }
                    }

                    None
                },
            );
    }

    /// Encontra todos os clips ligados por glue.
    pub fn find_group(
        clips: &[ClipDemo],
        start_idx: usize,
    ) -> Vec<usize> {
        let mut group = vec![start_idx];
        let track = clips[start_idx].track_index;

        // Walk forward.
        let mut current = start_idx;

        loop {
            let clip = &clips[current];

            if let Some(seam_beat) = clip.seam_beat {
                if let Some(idx) = clips
                    .iter()
                    .enumerate()
                    .find(|(i, c)| {
                        c.track_index == track
                            && !group.contains(i)
                            && (c.beat - seam_beat).abs() < 0.01
                    })
                    .map(|(i, _)| i)
                {
                    group.push(idx);
                    current = idx;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Walk backward.
        loop {
            let target_beats: Vec<f32> = group
                .iter()
                .map(|&i| clips[i].beat)
                .collect();

            if let Some(idx) = clips
                .iter()
                .enumerate()
                .find(|(i, c)| {
                    c.track_index == track
                        && !group.contains(i)
                        && c.seam_beat.map_or(false, |sb| {
                            target_beats
                                .iter()
                                .any(|&tb| (sb - tb).abs() < 0.01)
                        })
                })
                .map(|(i, _)| i)
            {
                group.push(idx);
            } else {
                break;
            }
        }

        group.sort();
        group
    }

    /// Inicia um possível arrasto.
    pub fn begin_drag(
        &mut self,
        clips: &[ClipDemo],
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) -> bool {
        if let Some(idx) = self.hit_test(clips, cursor, m) {
            let clip = &clips[idx];

            self.drag_idx = Some(idx);
            self.drag_orig_beat = clip.beat;
            self.drag_orig_track = clip.track_index;
            self.drag_start_cursor = cursor;
            self.drag_live_beat = clip.beat;
            self.drag_live_track = clip.track_index;
            self.drag_moved = false;
            self.stretch_mode = false;
            self.hover_idx = None;

            let group = Self::find_group(clips, idx);

            self.drag_group = group
                .iter()
                .map(|&i| {
                    (
                        i,
                        clips[i].beat,
                        clips[i].track_index,
                    )
                })
                .collect();

            true
        } else {
            false
        }
    }

    /// Inicia um stretch na borda direita do clip.
    pub fn begin_stretch(
        &mut self,
        clips: &[ClipDemo],
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) -> bool {
        if let Some(idx) = self.hit_test(clips, cursor, m) {
            let clip = &clips[idx];

            let (cx, _cy, cw, _ch) =
                clip_rect(clip, m);

            if cursor.0 >= cx + cw - 12.0 {
                self.drag_idx = Some(idx);
                self.drag_orig_beat = clip.beat;
                self.drag_orig_track = clip.track_index;
                self.drag_start_cursor = cursor;
                self.drag_live_beat = clip.beat;
                self.drag_live_track = clip.track_index;
                self.drag_moved = false;
                self.stretch_mode = true;
                self.stretch_orig_len = clip.len;
                self.stretch_live_len = clip.len;
                self.hover_idx = None;
                self.drag_group.clear();

                return true;
            }
        }

        false
    }

    /// Move o clip em arrasto preservando o ponto de pega.
    pub fn drag_move(
        &mut self,
        cursor: (f32, f32),
        m: &ClipMetrics,
        snap: bool,
    ) {
        if self.drag_idx.is_none() {
            return;
        }

        let dx = cursor.0 - self.drag_start_cursor.0;
        let dy = cursor.1 - self.drag_start_cursor.1;

        if !self.drag_moved
            && dx.abs() <= 3.0
            && dy.abs() <= 3.0
        {
            return;
        }

        self.drag_moved = true;

        if self.stretch_mode {
            let delta_beats =
                dx / m.px_per_beat.max(0.001);

            let new_len = if snap {
                (self.stretch_orig_len + delta_beats)
                    .round()
                    .max(0.25)
            } else {
                (self.stretch_orig_len + delta_beats)
                    .max(0.25)
            };

            self.stretch_live_len = new_len;
        } else {
            let delta_beats =
                dx / m.px_per_beat.max(0.001);

            let min_group_beat =
                if self.drag_group.is_empty() {
                    self.drag_orig_beat
                } else {
                    self.drag_group
                        .iter()
                        .map(|(_, ob, _)| *ob)
                        .fold(f32::INFINITY, f32::min)
                };

            let clamped_delta =
                delta_beats.max(-min_group_beat);

            let live_beat = if snap {
                (self.drag_orig_beat + clamped_delta)
                    .max(0.0)
                    .round()
            } else {
                (self.drag_orig_beat + clamped_delta)
                    .max(0.0)
            };

            self.drag_live_beat = live_beat;

            let row_delta =
                (dy / m.row_height.max(0.001)).round();

            let n_tracks = 4;

            let live_track =
                (self.drag_orig_track as f32 + row_delta)
                    .clamp(
                        0.0,
                        (n_tracks - 1) as f32,
                    ) as usize;

            self.drag_live_track = live_track;
        }
    }

    /// Termina o arrasto e aplica a nova posição.
    pub fn end_drag(
        &mut self,
        clips: &mut [ClipDemo],
        _m: &ClipMetrics,
        snap: bool,
    ) {
        if let Some(idx) = self.drag_idx {
            if self.drag_moved {
                if self.stretch_mode {
                    clips[idx].len =
                        self.stretch_live_len;
                } else {
                    let final_beat = if snap {
                        self.drag_live_beat
                            .round()
                            .max(0.0)
                    } else {
                        self.drag_live_beat
                    };

                    clips[idx].beat = final_beat;
                    clips[idx].track_index =
                        self.drag_live_track;

                    let beat_delta =
                        final_beat - self.drag_orig_beat;

                    let track_delta =
                        self.drag_live_track as i32
                            - self.drag_orig_track as i32;

                    let min_group_beat =
                        if self.drag_group.is_empty() {
                            self.drag_orig_beat
                        } else {
                            self.drag_group
                                .iter()
                                .map(|(_, ob, _)| *ob)
                                .fold(f32::INFINITY, f32::min)
                        };

                    let clamped_delta =
                        beat_delta.max(-min_group_beat);

                    if let Some(sb) =
                        clips[idx].seam_beat
                    {
                        clips[idx].seam_beat =
                            Some((sb + clamped_delta).max(0.0));
                    }

                    for (gi, orig_beat, orig_track)
                        in self.drag_group.iter()
                    {
                        if *gi == idx {
                            continue;
                        }

                        clips[*gi].beat =
                            (orig_beat + clamped_delta)
                                .max(0.0);

                        let new_track =
                            (*orig_track as i32
                                + track_delta)
                                .clamp(0, 3)
                                as usize;

                        clips[*gi].track_index =
                            new_track;

                        if let Some(sb) =
                            clips[*gi].seam_beat
                        {
                            clips[*gi].seam_beat =
                                Some((sb + clamped_delta).max(0.0));
                        }
                    }
                }
            }
        }

        self.drag_idx = None;
        self.drag_moved = false;
        self.stretch_mode = false;
        self.stretch_orig_len = 0.0;
        self.stretch_live_len = 0.0;
        self.drag_group.clear();
    }

    /// Inicia o desenho contínuo.
    pub fn begin_draw(
        &mut self,
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) -> bool {
        let (beat, track) =
            m.cursor_to_beat_track(cursor);

        let snapped_beat =
            beat.round().max(0.0);

        self.draw_state =
            Some(DrawToolState {
                active: true,
                start_beat: snapped_beat,
                start_track: track,
                current_beat: snapped_beat,
                current_track: track,
            });

        true
    }

    /// Atualiza o desenho contínuo.
    pub fn draw_move(
        &mut self,
        cursor: (f32, f32),
        m: &ClipMetrics,
    ) {
        if let Some(draw_state) =
            &mut self.draw_state
        {
            if draw_state.active {
                let (beat, track) =
                    m.cursor_to_beat_track(cursor);

                draw_state.current_beat =
                    beat.round().max(0.0);

                draw_state.current_track =
                    track;
            }
        }
    }

    /// Finaliza o desenho contínuo e cria o clip.
    pub fn end_draw(
        &mut self,
        clips: &mut Vec<ClipDemo>,
        _m: &ClipMetrics,
    ) -> Option<ClipDemo> {
        if let Some(draw_state) =
            self.draw_state.take()
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
                    draw_state.start_track;

                let clip = ClipDemo {
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

                clips.push(clip.clone());

                self.select_only(clips.len() - 1);

                return Some(clip);
            }
        }

        None
    }

    /// Inicia um marquee.
    pub fn begin_marquee(
        &mut self,
        cursor: (f32, f32),
    ) {
        self.marquee_active = true;
        self.marquee_start = cursor;
        self.marquee_cur = cursor;
        self.drag_group.clear();
    }

    /// Atualiza o marquee.
    pub fn marquee_move(
        &mut self,
        cursor: (f32, f32),
    ) {
        if self.marquee_active {
            self.marquee_cur = cursor;
        }
    }

    /// Finaliza o marquee.
    pub fn end_marquee(
        &mut self,
        clips: &[ClipDemo],
        m: &ClipMetrics,
    ) {
        if !self.marquee_active {
            return;
        }

        let (mx, my, mw, mh) =
            self.marquee_rect();

        for s in self.selected.iter_mut() {
            *s = false;
        }

        if mw <= 3.0 && mh <= 3.0 {
            self.marquee_active = false;
            return;
        }

        for (i, clip) in clips.iter().enumerate() {
            let (bx, by, bw, bh) =
                clip_rect(clip, m);

            let overlap_x =
                bx < mx + mw
                    && bx + bw > mx;

            let overlap_y =
                by < my + mh
                    && by + bh > my;

            if overlap_x
                && overlap_y
                && i < self.selected.len()
            {
                self.selected[i] = true;
            }
        }

        self.marquee_active = false;
    }
}
