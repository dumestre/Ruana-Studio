//! Clips da timeline: estruturas de dados, estado de hover/arrasto e
//! geração dos retângulos de render (corpo, header, waveform, fantasma e
//! overlay do clip em arrasto).

use super::{rgba, rgba_alpha, darker, RectInstance, palette, build_text_instances};

/// Um clip posicionado numa faixa (track) da timeline.
#[derive(Clone, Debug)]
pub struct ClipDemo {
    pub track_index: usize,
    pub beat: f32,
    pub len: f32,
    pub is_midi: bool,
    pub color: usize,
    pub name: Option<String>,
    pub seam_beat: Option<f32>,
    pub pre_glue_len: Option<f32>,
    pub pre_glue_color: Option<usize>,
    /// Amostras reais da waveform deste clip ([-1,1], uma por coluna), a
    /// fonte da verdade do buffer `wave_bars`. Slice divide este vetor ao
    /// meio (corte verdadeiro); as peças NUNCA são regeneradas.
    pub wave: Vec<f32>,
}

/// Métricas de conversão cursor ↔ conteúdo, espelhadas do App.
#[derive(Clone)]
pub struct ClipMetrics {
    pub px_per_beat: f32,
    /// Segmentos (offset, count) no buffer `wave_bars`, FIXOS por id do clip
    /// (calculados uma vez com os lens originais). Stretch/move nunca mudam
    /// o buffer, então NUNCA recomputar a partir dos lens vivos — senão o
    /// stretch de um clip corrompe a waveform dos clips seguintes.
    pub wave_segments: Vec<(u32, u32)>,
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
pub fn clip_rect(clip: &ClipDemo, m: &ClipMetrics) -> (f32, f32, f32, f32) {
    let x = clip.beat * m.px_per_beat - m.scroll_x;
    // SEM largura mínima artificial: a borda direita deve espelhar
    // exatamente `len * px_per_beat`, senão no zoom baixo o clip fica mais
    // largo que sua duração e "escorrega"/sobrepõe o vizinho (aviso amarelo).
    let w = clip.len * m.px_per_beat;
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

/// Estado de desenho contínuo com a ferramenta de desenho (tecla 2)
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
        self.draw_state.as_ref().map_or(false, |s| s.active)
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
    pub fn select_chain(&mut self, _clips: &[ClipDemo], chain: &[usize]) {
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
    pub fn unglue_seam(&mut self, clips: &mut Vec<ClipDemo>, idx: usize) {
        if clips[idx].seam_beat.is_some() {
            if let Some(orig_len) = clips[idx].pre_glue_len {
                let seam_beat = clips[idx].seam_beat.unwrap();
                let track = clips[idx].track_index;

                // Restaura cor original do clip da frente
                for c in clips.iter_mut() {
                    if c.track_index == track && (c.beat - seam_beat).abs() < 0.01
                        && c.pre_glue_color.is_some() {
                        c.color = c.pre_glue_color.unwrap();
                        c.pre_glue_color = None;
                    }
                }

                // Restaura len original do clip atual
                clips[idx].len = orig_len;
                clips[idx].seam_beat = None;
                clips[idx].pre_glue_len = None;
                self.select_only(idx);
            }
        }
    }

    /// Hit-test: qual clip está sob o cursor, na ordem (último por cima).
    pub fn hit_test(&self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) -> Option<usize> {
        let (beat, track) = m.cursor_to_beat_track(cursor);
        clips.iter().enumerate().rev().find(|(_, c)| {
            c.track_index == track && beat >= c.beat && beat < c.beat + c.len
        }).map(|(i, _)| i)
    }

    /// Retângulo do marquee em coordenadas normalizadas (x0<=x1, y0<=y1).
    pub fn marquee_rect(&self) -> (f32, f32, f32, f32) {
        let (sx, sy) = self.marquee_start;
        let (cx, cy) = self.marquee_cur;
        (sx.min(cx), sy.min(cy), (sx - cx).abs(), (sy - cy).abs())
    }

    /// Atualiza o hover conforme o cursor (quando não arrastando).
    /// Também detecta proximidade da borda direita (Stretch) e dos seams (Glue).
    pub fn update_hover(&mut self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) {
        if self.is_dragging() || self.is_drawing() {
            self.stretch_edge_idx = None;
            self.seam_hover_idx = None;
            return;
        }
        self.hover_idx = self.hit_test(clips, cursor, m);

        // Detecta se o cursor está perto da borda direita do clip hoverado
        // (usado pela ferramenta Stretch para feedback visual).
        self.stretch_edge_idx = self.hover_idx.and_then(|idx| {
            let clip = &clips[idx];
            let (cx, _cy, cw, _ch) = clip_rect(clip, m);
            if cursor.0 >= cx + cw - 12.0 {
                Some(idx)
            } else {
                None
            }
        });

        // Detecta hover sobre seam (linha pontilhada de des-colagem Glue).
        // Procura o clip com seam_beat sob o cursor, dentro de 8px do seam_x.
        self.seam_hover_idx = clips.iter().enumerate().rev().find_map(|(idx, clip)| {
            if let Some(seam_beat) = clip.seam_beat {
                let seam_x = seam_beat * m.px_per_beat - m.scroll_x;
                let (_, cy, _, ch) = clip_rect(clip, m);
                // Cursor precisa estar dentro do clip verticalmente e perto do seam
                if cursor.0 >= seam_x - 8.0 && cursor.0 <= seam_x + 8.0
                    && cursor.1 >= cy && cursor.1 <= cy + ch {
                    return Some(idx);
                }
            }
            None
        });
    }

    /// Encontra todos os clips ligados por glue (seam_beat) ao clip idx,
    /// caminhando tanto para frente quanto para trás.
    pub fn find_group(clips: &[ClipDemo], start_idx: usize) -> Vec<usize> {
        let mut group = vec![start_idx];
        let track = clips[start_idx].track_index;

        // Walk forward: seguir cadeias de seam_beat
        let mut current = start_idx;
        loop {
            let clip = &clips[current];
            if let Some(seam_beat) = clip.seam_beat {
                if let Some(idx) = clips.iter().enumerate()
                    .find(|(i, c)| c.track_index == track
                        && !group.contains(i)
                        && (c.beat - seam_beat).abs() < 0.01)
                    .map(|(i, _)| i) {
                    group.push(idx);
                    current = idx;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Walk backward: encontrar clips cujo seam_beat aponta para algum membro
        loop {
            let target_beats: Vec<f32> = group.iter()
                .map(|&i| clips[i].beat)
                .collect();
            if let Some(idx) = clips.iter().enumerate()
                .find(|(i, c)| c.track_index == track
                    && !group.contains(i)
                    && c.seam_beat.map_or(false, |sb| target_beats.iter().any(|&tb| (sb - tb).abs() < 0.01)))
                .map(|(i, _)| i) {
                group.push(idx);
            } else {
                break;
            }
        }

        group.sort();
        group
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
            self.stretch_mode = false;
            self.hover_idx = None;

            // Detecta grupo grudado (seam) — todos os membros se movem juntos
            let group = Self::find_group(clips, idx);
            self.drag_group = group.iter().map(|&i| {
                (i, clips[i].beat, clips[i].track_index)
            }).collect();
            true
        } else {
            false
        }
    }

    /// Inicia um stretch (esticamento) na borda direita do clip.
    pub fn begin_stretch(&mut self, clips: &[ClipDemo], cursor: (f32, f32), m: &ClipMetrics) -> bool {
        if let Some(idx) = self.hit_test(clips, cursor, m) {
            let clip = &clips[idx];
            let (cx, _cy, cw, _ch) = clip_rect(clip, m);
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

    /// Move o clip em arrasto preservando o ponto de pega (delta desde o DOWN),
    /// com snap de 1 beat e snap de linha inteira, e limiar de 3px.
    /// No modo stretch (stretch_mode), mantém a posição e varia apenas o `len`.
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

        if self.stretch_mode {
            let delta_beats = dx / m.px_per_beat.max(0.001);
            let new_len = if snap {
                (self.stretch_orig_len + delta_beats).round().max(0.25)
            } else {
                (self.stretch_orig_len + delta_beats).max(0.25)
            };
            self.stretch_live_len = new_len;
        } else {
            let delta_beats = dx / m.px_per_beat.max(0.001);

            // Clamp do delta para todo o grupo: o clip mais à esquerda define o limite.
            // Se empurramos para a esquerda e o leftmost chegar a beat 0,
            // o grupo inteiro para — mantém a adjacência do glue.
            let min_group_beat = if self.drag_group.is_empty() {
                self.drag_orig_beat
            } else {
                self.drag_group.iter()
                    .map(|(_, ob, _)| *ob)
                    .fold(f32::INFINITY, f32::min)
            };
            let clamped_delta = delta_beats.max(-min_group_beat);

            let live_beat = if snap {
                (self.drag_orig_beat + clamped_delta).max(0.0).round()
            } else {
                (self.drag_orig_beat + clamped_delta).max(0.0)
            };
            self.drag_live_beat = live_beat;

            let row_delta = (dy / m.row_height.max(0.001)).round();
            let n_tracks = 4;
            let live_track = (self.drag_orig_track as f32 + row_delta).clamp(0.0, (n_tracks - 1) as f32) as usize;
            self.drag_live_track = live_track;
        }
    }

    /// Termina o arrasto e aplica a nova posição no modelo (commit no release).
    pub fn end_drag(&mut self, clips: &mut [ClipDemo], _m: &ClipMetrics, snap: bool) {
        if let Some(idx) = self.drag_idx {
            if self.drag_moved {
                if self.stretch_mode {
                    clips[idx].len = self.stretch_live_len;
                } else {
                    let final_beat = if snap {
                        self.drag_live_beat.round().max(0.0)
                    } else {
                        self.drag_live_beat
                    };
                    // Move primary clip
                    clips[idx].beat = final_beat;
                    clips[idx].track_index = self.drag_live_track;

                    // Move todos os membros do grupo pelo mesmo delta
                    let beat_delta = final_beat - self.drag_orig_beat;
                    let track_delta = self.drag_live_track as i32 - self.drag_orig_track as i32;
                    // Clamp do delta pro leftmost do grupo não ir abaixo de 0
                    let min_group_beat = if self.drag_group.is_empty() {
                        self.drag_orig_beat
                    } else {
                        self.drag_group.iter()
                            .map(|(_, ob, _)| *ob)
                            .fold(f32::INFINITY, f32::min)
                    };
                    let clamped_delta = beat_delta.max(-min_group_beat);
                    // Atualiza seam_beat do primary (é beat absoluto, precisa seguir o movimento)
                    if let Some(sb) = clips[idx].seam_beat {
                        clips[idx].seam_beat = Some((sb + clamped_delta).max(0.0));
                    }
                    for (gi, orig_beat, orig_track) in self.drag_group.iter() {
                        if *gi == idx { continue; }
                        clips[*gi].beat = (orig_beat + clamped_delta).max(0.0);
                        let new_track = (*orig_track as i32 + track_delta).clamp(0, 3) as usize;
                        clips[*gi].track_index = new_track;
                        // Atualiza seam_beat para refletir a nova posição (seam é beat absoluto)
                        if let Some(sb) = clips[*gi].seam_beat {
                            clips[*gi].seam_beat = Some((sb + clamped_delta).max(0.0));
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

    /// Inicia o desenho contínuo com a ferramenta de desenho (tecla 2)
    pub fn begin_draw(&mut self, cursor: (f32, f32), m: &ClipMetrics) -> bool {
        let (beat, track) = m.cursor_to_beat_track(cursor);
        let snapped_beat = beat.round().max(0.0);
        self.draw_state = Some(DrawToolState {
            active: true,
            start_beat: snapped_beat,
            start_track: track,
            current_beat: snapped_beat,
            current_track: track,
        });
        true
    }

    /// Atualiza o desenho contínuo durante o movimento do mouse
    pub fn draw_move(&mut self, cursor: (f32, f32), m: &ClipMetrics) {
        if let Some(draw_state) = &mut self.draw_state {
            if draw_state.active {
                let (beat, track) = m.cursor_to_beat_track(cursor);
                draw_state.current_beat = beat.round().max(0.0);
                draw_state.current_track = track;
            }
        }
    }

    /// Finaliza o desenho contínuo e cria o clip
    pub fn end_draw(&mut self, clips: &mut Vec<ClipDemo>, _m: &ClipMetrics) -> Option<ClipDemo> {
        if let Some(draw_state) = self.draw_state.take() {
            if draw_state.active {
                let start_beat = draw_state.start_beat.min(draw_state.current_beat);
                let end_beat = draw_state.start_beat.max(draw_state.current_beat);
                let len = (end_beat - start_beat).max(0.25);
                let track = draw_state.start_track;

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
                };

                clips.push(clip.clone());
                self.select_only(clips.len() - 1);
                return Some(clip);
            }
        }
        None
    }

    /// Inicia um marquee a partir de um ponto da janela (clique em área vazia).
    pub fn begin_marquee(&mut self, cursor: (f32, f32)) {
        self.marquee_active = true;
        self.marquee_start = cursor;
        self.marquee_cur = cursor;
        self.drag_group.clear();
    }

    /// Atualiza o canto atual do marquee durante o movimento.
    pub fn marquee_move(&mut self, cursor: (f32, f32)) {
        if self.marquee_active {
            self.marquee_cur = cursor;
        }
    }

    /// Finaliza o marquee: seleciona os clips cujo retângulo cruza a caixa.
    /// Clique simples (tamanho ~0) em área vazia limpa a seleção.
    pub fn end_marquee(&mut self, clips: &[ClipDemo], m: &ClipMetrics) {
        if !self.marquee_active {
            return;
        }
        let (mx, my, mw, mh) = self.marquee_rect();
        for s in self.selected.iter_mut() {
            *s = false;
        }
        // Clique simples (sem área real) apenas limpa.
        if mw <= 3.0 && mh <= 3.0 {
            self.marquee_active = false;
            return;
        }
        for (i, clip) in clips.iter().enumerate() {
            let (bx, by, bw, bh) = clip_rect(clip, m);
            // Cruzamento em coordenadas de janela/box.
            let overlap_x = bx < mx + mw && bx + bw > mx;
            let overlap_y = by < my + mh && by + bh > my;
            if overlap_x && overlap_y && i < self.selected.len() {
                self.selected[i] = true;
            }
        }
        self.marquee_active = false;
    }
}

/// Desenha o nome do clip no header, alinhado à esquerda, em preto.
fn push_clip_name(
    instances: &mut Vec<RectInstance>,
    clip: &ClipDemo,
    x: f32, y: f32, w: f32,
    header_h: f32, bw: f32,
) {
    if let Some(ref name) = clip.name {
        let scale = 1.0;
        let char_w = 8.0 * scale;
        let gap = 1.0 * scale;
        let total_w = (name.len() as f32 - 1.0) * gap + name.len() as f32 * char_w;
        let available = w - 2.0 * bw;
        if total_w <= available {
            let text_y = y + header_h / 2.0 - 4.0 * scale;
            let start_x = x + bw + 4.0;
            let center_x = start_x + total_w / 2.0;
            let text_color = [0u8, 0, 0, 255];
            instances.extend(build_text_instances(name, center_x, text_y, text_color, scale));
        }
    }
}

/// Gera os retângulos de todos os clips, com hover/fantasma/overlay.
pub fn build_clip_instances(
    clips: &[ClipDemo],
    inter: &ClipInteraction,
    m: &ClipMetrics,
    width: f32,
) -> Vec<RectInstance> {
    let t = m.theme.clamp(0, 6) as usize;
    let header_h = 16.0;
    let bw = 1.0;

    // Parametrização de transparências do corpo do clip.
    let body_alpha_normal = 0.50;
    let body_alpha_hover = 0.32;
    let body_alpha_ghost = 0.08; // fantasma na posição antiga: quase imperceptível
    let body_alpha_overlay = 0.40; // clip arrastado: visível mas translúcido

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
        let in_group = inter.drag_group.iter().any(|(gi, _, _)| *gi == idx);
        let being_dragged = in_group && inter.drag_moved && !inter.stretch_mode;

        // Checa se o clip está dentro do marquee ativo (real-time feedback)
        let in_marquee = if inter.marquee_active {
            let (mx, my, mw, mh) = inter.marquee_rect();
            x < mx + mw && x + w > mx && y < my + mh && (y + m.row_height) > my
        } else {
            false
        };

        let body_alpha = if being_dragged {
            body_alpha_ghost
        } else if inter.hover_idx == Some(idx) || in_marquee {
            body_alpha_hover
        } else {
            body_alpha_normal
        };
        let body_dark = if being_dragged { 0.70 } else { 0.12 };

        push_clip(&mut instances, clip, base, x, y, w, draw_w, header_h, bw,
                  body_alpha, body_dark, &m.wave_segments, idx, t, m.row_height, w);
        push_clip_name(&mut instances, clip, x, y, w, header_h, bw);

// Borda de seleção simples (4 tiras), espessura 4px, cor da waveform
        // (#262626).
        if !being_dragged && (idx < inter.selected.len() && inter.selected[idx] || in_marquee) {
            let ac = [0x26, 0x26, 0x26, 255];
            let thin = 4.0;
            // superior
            instances.push(RectInstance { x, y, w, h: thin, color: ac,
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0 });
            // inferior
            instances.push(RectInstance { x, y: y + m.row_height - thin, w, h: thin, color: ac,
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0 });
            // esquerda
            instances.push(RectInstance { x, y, w: thin, h: m.row_height, color: ac,
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            // direita
            instances.push(RectInstance { x: x + w - thin, y, w: thin, h: m.row_height, color: ac,
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
        }

        // Linha pontilhada do seam (ponto de costura Glue) — permite des-colar
        if let Some(seam_beat) = clip.seam_beat {
            let seam_x = seam_beat * m.px_per_beat - m.scroll_x;
            if seam_x >= x && seam_x <= x + draw_w {
                // Cor da waveform (#262626)
                let seam_col = (0x26, 0x26, 0x26);
                let is_hovered = inter.seam_hover_idx == Some(idx);
                let alpha: u8 = if is_hovered { 200 } else { 110 };
                let dash_w = 2.0;
                let dash_h = 5.0;
                let gap = 3.0;
                let mut dy = y;
                while dy + dash_h <= y + m.row_height {
                    instances.push(RectInstance {
                        x: seam_x - dash_w / 2.0, y: dy,
                        w: dash_w, h: dash_h,
                        color: [seam_col.0, seam_col.1, seam_col.2, alpha],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                    dy += dash_h + gap;
                }
            }
        }
    }

    // Overlay: clip seguindo o mouse durante o arrasto (moved).
    if let (Some(idx), true) = (inter.drag_idx, inter.drag_moved) {
        if let Some(clip) = clips.get(idx) {
            let live = if inter.stretch_mode {
                // Stretch: posição original, len varia
                ClipDemo {
                    track_index: inter.drag_orig_track,
                    beat: inter.drag_orig_beat,
                    len: inter.stretch_live_len,
                    is_midi: clip.is_midi,
                    color: clip.color,
                    name: clip.name.clone(),
                    seam_beat: clip.seam_beat,
                    pre_glue_len: clip.pre_glue_len,
                    pre_glue_color: clip.pre_glue_color,
                    wave: clip.wave.clone(),
                }
            } else {
                ClipDemo {
                    track_index: inter.drag_live_track,
                    beat: inter.drag_live_beat,
                    len: clip.len,
                    is_midi: clip.is_midi,
                    color: clip.color,
                    name: clip.name.clone(),
                    seam_beat: clip.seam_beat,
                    pre_glue_len: clip.pre_glue_len,
                    pre_glue_color: clip.pre_glue_color,
                    wave: clip.wave.clone(),
                }
            };
            let (x, y, w, _h) = clip_rect(&live, m);
            // A waveform estica junto com o clip em tempo real: os samples são
            // os ORIGINAIS (wave_len) mapeados sobre a largura VIVA (w).
            let wave_natural_w = w;
            if w > 0.0 && x < width && x + w > 0.0 {
                let draw_w = w.min(width - x).max(0.0);
                let base = clip_color(clip, t);
                push_clip(&mut instances, &live, base, x, y, w, draw_w, header_h, bw,
                          body_alpha_overlay, 0.12, &m.wave_segments, idx, t, m.row_height, wave_natural_w);
                push_clip_name(&mut instances, &live, x, y, w, header_h, bw);
                // Contorno visível no overlay do clip arrastado
                let border_col = if t == 6 { (0xDC, 0x14, 0x3C) } else { super::palette::ACCENT[t] };
                let th = 2.0;
                instances.push(RectInstance { x, y, w: draw_w, h: th, color: [border_col.0, border_col.1, border_col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0 });
                instances.push(RectInstance { x, y: y + m.row_height - th, w: draw_w, h: th, color: [border_col.0, border_col.1, border_col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
                instances.push(RectInstance { x, y, w: th, h: m.row_height, color: [border_col.0, border_col.1, border_col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
                instances.push(RectInstance { x: x + draw_w - th, y, w: th, h: m.row_height, color: [border_col.0, border_col.1, border_col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            }
        }

        // Overlay dos membros do grupo (todos se movem juntos pelo mesmo delta)
        if !inter.stretch_mode && inter.drag_moved && !inter.drag_group.is_empty() {
            let beat_delta = inter.drag_live_beat - inter.drag_orig_beat;
            let track_delta = inter.drag_live_track as i32 - inter.drag_orig_track as i32;
            // Clamp pro leftmost do grupo não ir abaixo de 0 (mesmo do drag_move/end_drag)
            let min_group_beat = inter.drag_group.iter()
                .map(|(_, ob, _)| *ob)
                .fold(f32::INFINITY, f32::min);
            let clamped_delta = beat_delta.max(-min_group_beat);
            for (gi, orig_beat, orig_track) in inter.drag_group.iter() {
                if Some(*gi) == inter.drag_idx { continue; } // primary já renderizado acima
                if let Some(clip) = clips.get(*gi) {
                    let live_beat = (orig_beat + clamped_delta).max(0.0);
                    let live_track = (*orig_track as i32 + track_delta).clamp(0, 3) as usize;
                    let live = ClipDemo {
                        track_index: live_track,
                        beat: live_beat,
                        ..clip.clone()
                    };
                    let (x, y, w, _h) = clip_rect(&live, m);
                    if w > 0.0 && x < width && x + w > 0.0 {
                        let draw_w = w.min(width - x).max(0.0);
                        let base = clip_color(clip, t);
                        push_clip(&mut instances, &live, base, x, y, w, draw_w, header_h, bw,
                                  body_alpha_overlay, 0.12, &m.wave_segments, *gi, t, m.row_height, w);
                        push_clip_name(&mut instances, &live, x, y, w, header_h, bw);
                        // Contorno visível
                        let border_col = if t == 6 { (0xDC, 0x14, 0x3C) } else { super::palette::ACCENT[t] };
                        let th = 2.0;
                        instances.push(RectInstance { x, y, w: draw_w, h: th, color: [border_col.0, border_col.1, border_col.2, 255],
                            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0 });
                    }
                }
            }
        }
    }
 
     // Desenho contínuo com a ferramenta de desenho (tecla 2)
    if let Some(draw_state) = &inter.draw_state {
        if draw_state.active {
            let start_beat = draw_state.start_beat.min(draw_state.current_beat);
            let end_beat = draw_state.start_beat.max(draw_state.current_beat);
            let len = (end_beat - start_beat).max(0.25);
            let track = draw_state.start_track.min(draw_state.current_track);

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
            };

            let (x, y, w, _h) = clip_rect(&temp_clip, m);
            if w > 0.0 && x < width && x + w > 0.0 {
                let draw_w = w.min(width - x).max(0.0);
                let base = clip_color(&temp_clip, t);
                push_clip(&mut instances, &temp_clip, base, x, y, w, draw_w, header_h, bw,
                          body_alpha_overlay, 0.12, &m.wave_segments, clips.len(), t, m.row_height, w);
            }
        }
    }

     // Marquee de multiseleção (caixa accent).
    // Na cor da WAVEFORM (#262626) — mesma identidade da borda de seleção.
    if inter.marquee_active {
        let (mx, my, mw, mh) = inter.marquee_rect();
        if mw > 0.0 && mh > 0.0 {
            // preenchimento leve
            instances.push(RectInstance { x: mx, y: my, w: mw, h: mh, color: [0x26, 0x26, 0x26, 60],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            // borda de 1px
            let thin = 1.0;
            instances.push(RectInstance { x: mx, y: my, w: mw, h: thin, color: [0x26, 0x26, 0x26, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            instances.push(RectInstance { x: mx, y: my + mh - thin, w: mw, h: thin, color: [0x26, 0x26, 0x26, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            instances.push(RectInstance { x: mx, y: my, w: thin, h: mh, color: [0x26, 0x26, 0x26, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
            instances.push(RectInstance { x: mx + mw - thin, y: my, w: thin, h: mh, color: [0x26, 0x26, 0x26, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
        }
    }

    // Sobreposição de clips: cada região de contato entre clips que se cruzam
    // recebe listras diagonais grossas (modo diagonal no shader).
    for i in 0..clips.len() {
        for j in (i + 1)..clips.len() {
            let (xi, yi, wi, hi) = clip_rect(&clips[i], m);
            let (xj, yj, wj, hj) = clip_rect(&clips[j], m);
            let ix0 = xi.max(xj);
            let ix1 = (xi + wi).min(xj + wj);
            let iy0 = yi.max(yj);
            let iy1 = (yi + hi).min(yj + hj);
            if ix1 > ix0 && iy1 > iy0 {
                let ov = (0xFFu8, 0x8Cu8, 0x00u8); // laranja de alerta
                instances.push(RectInstance {
                    x: ix0, y: iy0, w: ix1 - ix0, h: iy1 - iy0,
                    color: [ov.0, ov.1, ov.2, 220],
                    wave_offset: 0, wave_count: 0,
                    clip_w: -1.0, clip_h: 0.0, radius: 0.0,
                });
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
    t: usize,
    row_height: f32,
    wave_natural_w: f32,
) {
    // Raio leve nos cantos do clip para quebrar o visual quadrado.
    let clip_r = 6.0;

    // Alpha do header/divisor/indicador: quando o corpo é muito translúcido
    // (ghost durante drag), escurece os elementos decorativos.
    // Clip normal mantém 100% opaco; ghost fica mais pálido mas visível.
    let decor_alpha: u8 = if body_alpha < 0.15 {
        ((body_alpha * 255.0 * 6.0).round() as u8).min(180) // ghost: ~48 alpha
    } else {
        255
    };

    // Header
    instances.push(RectInstance {
        x: x + bw, y: y + bw,
        w: (draw_w - 2.0 * bw).max(0.0),
        h: (header_h - bw).max(0.0),
        color: rgba(base, decor_alpha),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: clip_r,
    });

    // Waveform (só para clips de áudio) — segmento FIXO por id (calculado na
    // geração do buffer com os lens originais). Nunca derive de lens vivos.
    let (wo, wc) = if clip.is_midi {
        (0u32, 0u32)
    } else if let Some(&(o, c)) = segs.get(idx) {
        (o, c)
    } else {
        (0u32, 0u32)
    };
    let body_h = row_height - header_h;

    // Corpo (com waveform) — alpha e escurecimento controlados por hover/drag.
    instances.push(RectInstance {
        x: x + bw, y: y + header_h,
        w: w, h: body_h,
        color: rgba_alpha(darker(base, dark_amount), body_alpha),
        wave_offset: wo, wave_count: wc,         clip_w: wave_natural_w, clip_h: body_h, radius: clip_r,
    });

    // Divisória
    instances.push(RectInstance {
        x: x + bw, y: y + header_h - 1.0,
        w: (draw_w - 2.0 * bw).max(0.0), h: 1.0,
        color: rgba(darker(base, 0.18), decor_alpha),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // Indicador direito
    instances.push(RectInstance {
        x: x + draw_w - bw - 6.0, y: y + 5.0,
        w: 3.0, h: 8.0,
        color: rgba(palette::TEXT_PRIMARY[t], decor_alpha),
        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
    });

    // Ghost: outline wireframe (posição original durante drag)
    if body_alpha < 0.15 {
        let ghost_border = rgba(darker(base, 0.15), 55);
        let th = 1.0;
        instances.push(RectInstance { x, y, w: draw_w, h: th, color: ghost_border,
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: clip_r });
        instances.push(RectInstance { x, y: y + row_height - th, w: draw_w, h: th, color: ghost_border,
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
        instances.push(RectInstance { x, y, w: th, h: row_height, color: ghost_border,
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
        instances.push(RectInstance { x: x + draw_w - th, y, w: th, h: row_height, color: ghost_border,
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0 });
    }
}
