//! Interação: mouse, scroll, ferramentas, clipboard, loop, foco e edge-scroll.

use winit::event::ElementState;

use crate::app::{LoopDrag, App};
use crate::clips;
use crate::fades;
use crate::tools::{current_tool, set_cursor_override, set_gain_override, set_hand_override, Tool};

impl App {
    pub fn handle_scroll(&mut self, delta: &winit::event::MouseScrollDelta, with_ctrl: bool, with_alt: bool) {
        let (dx, dy) = match *delta {
            winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
            winit::event::MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
        };
        let (mx, my) = self.last_cursor;

        if with_ctrl && dy.abs() >= 1.0 {
            // Zoom horizontal — mantém o ponto sob o cursor fixo.
            let old_px = self.px_per_beat;
            let new_px = (old_px + dy * 0.05).clamp(8.0, 640.0);
            // Beat sob o cursor antes do zoom.
            let beat_under = (mx + self.scroll_x) / old_px.max(0.01);
            self.px_per_beat = new_px;
            self.scroll_x = (beat_under * new_px - mx).max(0.0);
        } else if with_alt && dy.abs() >= 1.0 {
            // Zoom vertical — mantém a linha sob o cursor fixa.
            let old_zy = self.zoom_y;
            let new_zy = (old_zy + dy * 0.01).clamp(0.25, 6.0);
            // Linha sob o cursor antes do zoom (base 64px).
            let row_under = (my - self.ruler_px + self.scroll_y) / (64.0 * old_zy.max(0.01)).max(0.01);
            self.zoom_y = new_zy;
            self.scroll_y = (row_under * 64.0 * new_zy - (my - self.ruler_px)).max(0.0);
        } else {
            // Scroll pelos eixos reais do delta: x → horizontal, y → vertical.
            self.scroll_x = (self.scroll_x - dx).max(0.0);
            self.scroll_y = (self.scroll_y - dy).max(0.0);
        }
        self.clamp_scroll_y(self.size.height as f32);
    }

    pub fn handle_mouse_button(&mut self, state: ElementState, pos: Option<(f32, f32)>) {
        match state {
            ElementState::Pressed => {
                self.panning = true;
                if let Some(p) = pos { self.last_cursor = p; }
            }
            ElementState::Released => {
                // Se houver preview de áudio pendente, insere ao liberar o botão.
                if let Some(preview) = self.drop_preview.take() {
                    if let Some(path) = self.drop_path.take() {
                        self.load_audio_file(&path, preview.beat, preview.track);
                    }
                }
                self.panning = false;
            }
        }
    }

    /// Inicia o arrasto do loop na régua a partir da posição x do cursor.
    /// Retorna true se iniciou um arrasto de loop; false para comportamento padrão.
    pub fn begin_loop_drag(&mut self, mx: f32) -> bool {
        if !self.loop_enabled {
            return false;
        }
        let sx = self.beat_to_x(self.loop_start_beat);
        let ex = self.beat_to_x(self.loop_end_beat);
        let grab = 6.0;
        self.loop_drag_orig_start = self.loop_start_beat;
        self.loop_drag_orig_end = self.loop_end_beat;
        self.loop_drag_start_x = mx;
        if (mx - sx).abs() <= grab {
            self.loop_drag = Some(LoopDrag::Start);
        } else if (mx - ex).abs() <= grab {
            self.loop_drag = Some(LoopDrag::End);
        } else if mx >= sx && mx <= ex {
            self.loop_drag = Some(LoopDrag::Move);
        } else {
            return false;
        }
        true
    }

    /// Move a borda/seleção do loop com snap de 1 beat (arredonda o beat).
    pub fn handle_loop_drag(&mut self, mx: f32) {
        let Some(kind) = self.loop_drag else { return };
        let dx = mx - self.loop_drag_start_x;
        let delta_beats = (dx / self.px_per_beat.max(0.001)).round();
        match kind {
            LoopDrag::Start => {
                let new = (self.loop_drag_orig_start + delta_beats)
                    .max(0.0)
                    .min(self.loop_end_beat - 1.0);
                self.loop_start_beat = new.max(0.0);
            }
            LoopDrag::End => {
                let new = (self.loop_drag_orig_end + delta_beats)
                    .max(self.loop_start_beat + 1.0);
                self.loop_end_beat = new;
            }
            LoopDrag::Move => {
                let span = self.loop_drag_orig_end - self.loop_drag_orig_start;
                let new_start = (self.loop_drag_orig_start + delta_beats).max(0.0);
                self.loop_start_beat = new_start;
                self.loop_end_beat = new_start + span;
            }
        }
    }

    /// Alterna o foco no clip sob o cursor: F centraliza o clip na view e
    /// lembra o estado anterior; F de novo restaura o estado anterior ao foco.
    pub fn toggle_clip_focus(&mut self, width: f32) {
        if let Some((sx, ppb, sy)) = self.focus_saved.take() {
            self.scroll_x = sx;
            self.px_per_beat = ppb;
            self.scroll_y = sy;
            return;
        }
        let Some(idx) = self.clip_interaction.hover_idx else { return };
        let Some(clip) = self.clips.get(idx) else { return };
        self.focus_saved = Some((self.scroll_x, self.px_per_beat, self.scroll_y));
        let clip_x = clip.beat * self.px_per_beat;
        self.scroll_x = (clip_x - width * 0.5).max(0.0);
    }

    /// Copia os clips selecionados para o clipboard, preservando posições relativas.
    pub fn copy_selected(&mut self) {
        let selected_clips: Vec<clips::ClipDemo> = self
            .clips
            .iter()
            .enumerate()
            .filter(|(i, _)| i < &self.clip_interaction.selected.len() && self.clip_interaction.selected[*i])
            .map(|(_, c)| c.clone())
            .collect();
        if !selected_clips.is_empty() {
            self.clipboard = selected_clips;
        }
    }

    /// Seleciona todos os clips da timeline.
    pub fn select_all(&mut self) {
        self.clip_interaction.sync_selection(self.clips.len());
        self.clip_interaction.select_all();
    }

    /// Registra um snapshot do estado atual (clips + fades) na pilha de undo.
    /// Limpa a pilha de redo (novo ramo de edição). Deve ser chamado ANTES de
    /// qualquer mutação estrutural (import, paste, slice, erase, etc.).
    pub fn record_undo(&mut self) {
        use crate::app::TimelineSnapshot;
        self.undo_stack.push(TimelineSnapshot {
            clips: self.clips.clone(),
            fades: self.fades.clone(),
        });
        self.redo_stack.clear();
    }

    /// Desfaz a última edição estrutural (clips + fades).
    pub fn undo(&mut self) {
        use crate::app::TimelineSnapshot;
        if let Some(snap) = self.undo_stack.pop() {
            // O estado atual vai para o redo (permite refazer).
            self.redo_stack.push(TimelineSnapshot {
                clips: self.clips.clone(),
                fades: self.fades.clone(),
            });
            self.clips = snap.clips;
            self.fades = snap.fades;
            self.clip_interaction.sync_selection(self.clips.len());
            self.rebuild_wave_data();
        }
    }

    /// Refaz a edição que foi desfeita.
    pub fn redo(&mut self) {
        use crate::app::TimelineSnapshot;
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(TimelineSnapshot {
                clips: self.clips.clone(),
                fades: self.fades.clone(),
            });
            self.clips = snap.clips;
            self.fades = snap.fades;
            self.clip_interaction.sync_selection(self.clips.len());
            self.rebuild_wave_data();
        }
    }

    /// Cola os clips do clipboard na posição alvo (beat, track), com snap ao grid (1 beat).
    /// Mantém a estrutura relativa dos clips copiados.
    pub fn paste_at_beat(&mut self, target_beat: f32, target_track: usize) {
        if self.clipboard.is_empty() {
            return;
        }
        self.record_undo();
        // Encontra o beat mínimo dos clips copiados para manter offsets relativos
        let min_beat = self.clipboard.iter().map(|c| c.beat).fold(f32::INFINITY, f32::min);
        let snap_beat = target_beat.round().max(0.0);
        let beat_offset = snap_beat - min_beat.round();
        // Track offset: usa a track do primeiro clip copiado como referência
        let min_track = self.clipboard.iter().map(|c| c.track_index).min().unwrap_or(0);
        let track_offset = target_track as i32 - min_track as i32;
        let max_track = self.clips.iter().map(|c| c.track_index).max().unwrap_or(0);
        for clip in &self.clipboard {
            let mut new_clip = clip.clone();
            new_clip.beat = (clip.beat + beat_offset).max(0.0).round();
            new_clip.track_index = (clip.track_index as i32 + track_offset)
                .clamp(0, (max_track + 1).max(1) as i32 - 1) as usize;
            self.clips.push(new_clip);
        }
        // Seleciona os clips colados
        self.clip_interaction.sync_selection(self.clips.len());
        for s in self.clip_interaction.selected.iter_mut() {
            *s = false;
        }
        let pasted_count = self.clipboard.len();
        for i in (self.clips.len() - pasted_count)..self.clips.len() {
            if i < self.clip_interaction.selected.len() {
                self.clip_interaction.selected[i] = true;
            }
        }
        self.rebuild_wave_data();
    }

    /// Duplica os clips selecionados APÓS o último clip selecionado (com snap ao grid).
    pub fn duplicate_selected(&mut self) {
        let selected_indices: Vec<usize> = self
            .clip_interaction
            .selected
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s)
            .map(|(i, _)| i)
            .collect();
        if selected_indices.is_empty() {
            return;
        }
        self.record_undo();
        // Encontra o clip que termina mais tarde (maior beat + len)
        let max_end = selected_indices.iter()
            .map(|&i| self.clips[i].beat + self.clips[i].len)
            .fold(0.0_f32, f32::max);
        let snap_beat = max_end.round().max(0.0);
        let clips_to_dup: Vec<clips::ClipDemo> = selected_indices.iter().map(|&i| self.clips[i].clone()).collect();
        let min_beat = clips_to_dup.iter().map(|c| c.beat).fold(f32::INFINITY, f32::min);
        let beat_offset = snap_beat - min_beat.round();
        for clip in &clips_to_dup {
            let mut new_clip = clip.clone();
            new_clip.beat = (clip.beat + beat_offset).max(0.0).round();
            self.clips.push(new_clip);
        }
        // Seleciona os duplicados
        self.clip_interaction.sync_selection(self.clips.len());
        for s in self.clip_interaction.selected.iter_mut() {
            *s = false;
        }
        let dup_count = clips_to_dup.len();
        for i in (self.clips.len() - dup_count)..self.clips.len() {
            if i < self.clip_interaction.selected.len() {
                self.clip_interaction.selected[i] = true;
            }
        }
        self.rebuild_wave_data();
    }

    /// Deleta os clips selecionados.
    pub fn delete_selected(&mut self) {
        let selected_indices: Vec<usize> = self
            .clip_interaction
            .selected
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s)
            .map(|(i, _)| i)
            .collect();
        if selected_indices.is_empty() {
            return;
        }
        self.record_undo();
        // Remove em ordem reversa para manter índices válidos
        for &i in selected_indices.iter().rev() {
            self.clips.remove(i);
        }
        self.clip_interaction.sync_selection(self.clips.len());
        self.rebuild_wave_data();
    }

    /// Converte posição do cursor para beat e track com snap ao grid.
    pub fn cursor_to_snapped_beat_track(&self, pos: (f32, f32)) -> (f32, usize) {
        let beat = (pos.0 + self.scroll_x) / self.px_per_beat.max(0.001);
        let snapped_beat = beat.round().max(0.0);
        let row_height = 64.0 * self.zoom_y.max(0.001);
        let track = ((pos.1 - self.ruler_px + self.scroll_y) / row_height)
            .floor()
            .max(0.0) as usize;
        (snapped_beat, track)
    }

    /// Desenha novo clip (ferramenta Draw/Pencil).
    #[allow(dead_code)]
    pub fn tool_draw(&mut self, pos: (f32, f32)) {
        let (beat, track) = self.cursor_to_snapped_beat_track(pos);
        let len = 4.0; // comprimento padrão: 1 compasso
        self.clips.push(clips::ClipDemo {
            track_index: track,
            beat,
            len,
            is_midi: false,
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
        });
        self.clip_interaction.sync_selection(self.clips.len());
        self.rebuild_wave_data();
    }

    /// Estica clip (ferramenta Stretch) - arrasta borda direita.
    pub fn tool_stretch_start(&mut self, pos: (f32, f32)) -> bool {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            // Verifica se clicou na borda direita do clip (últimos 12px)
            let (cx, _, cw, _) = clips::clip_rect(&self.clips[idx], &cm);
            if pos.0 >= cx + cw - 12.0 {
                self.clip_interaction.begin_stretch(&self.clips, pos, &cm);
                self.clip_interaction.select_only(idx);
                self.edge_scroll_x = 0.0;
                self.edge_scroll_y = 0.0;
                self.edge_scroll_dir = 0;
                return true;
            }
        }
        false
    }

    /// Corta clip (ferramenta Slice — tecla 4).
    /// Divide o clip no ponto do clique. O clip da esquerda (original) é selecionado,
    /// então cortes consecutivos deixam o clip do meio selecionado automaticamente.
    pub fn tool_slice(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            let clip = self.clips[idx].clone();
            let click_beat = (pos.0 + self.scroll_x) / self.px_per_beat.max(0.001);
            let click_beat_snapped = click_beat.round().max(clip.beat).min(clip.beat + clip.len);

            // Só divide se o clique não estiver na borda
            if click_beat_snapped > clip.beat && click_beat_snapped < clip.beat + clip.len {
                self.record_undo();
                let left_len = click_beat_snapped - clip.beat;
                let right_len = clip.beat + clip.len - click_beat_snapped;

                // Atualiza clip original para a parte esquerda
                self.clips[idx].len = left_len;

                // Cria novo clip para a parte direita
                let mut right_clip = clip;
                right_clip.beat = click_beat_snapped;
                right_clip.len = right_len;

                // Corte VERDADEIRO da waveform: divide as amostras do clip
                // pai, proporcional ao ponto de corte. As duas peças exibem
                // exatamente o conteúdo original, apenas cortado — nunca uma
                // onda regenerada.
                let parent_len = left_len + right_len;
                if left_len >= 0.25 && right_len >= 0.25 && self.clips[idx].wave.len() > 1 {
                    let n = self.clips[idx].wave.len();
                    let frac = (left_len / parent_len).clamp(0.0, 1.0);
                    let split_at = ((n as f32 * frac).round() as usize).clamp(1, n - 1);
                    let right_wave = self.clips[idx].wave[split_at..].to_vec();
                    self.clips[idx].wave.truncate(split_at);
                    right_clip.wave = right_wave;
                } else {
                    right_clip.wave = Vec::new();
                }
                self.clips.push(right_clip);

                self.clip_interaction.sync_selection(self.clips.len());
                // Seleciona o clip esquerdo (parte que foi cortada) —
                // cortes consecutivos mantêm o clip do meio selecionado.
                self.clip_interaction.select_only(idx);
                // A segmentação do buffer muda (um clip virou dois): regenera
                // para o clip direito também ter waveform.
                self.rebuild_wave_data();
            }
        }
    }

    /// Apaga clip (ferramenta Erase) - clica para deletar.
    pub fn tool_erase(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            self.record_undo();
            self.clips.remove(idx);
            self.clip_interaction.sync_selection(self.clips.len());
            self.rebuild_wave_data();
        }
    }

    /// Cola clips (ferramenta Glue — tecla 6).
    /// Clicar num clip: estica a borda direita dele (ou do último da cadeia)
    /// até o início do próximo clip na mesma track, preenchendo o gap.
    /// Ambos permanecem como clips separados, mas adjacentes (sem gap),
    /// com a mesma cor. O seam marca o ponto de encontro para des-colagem.
    /// Clicar de novo no mesmo clip continua a cadeia grudando no próximo.
    pub fn tool_glue(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            let track = self.clips[idx].track_index;

            // Encontra o final da cadeia de glue (último clip com seam)
            let chain = clips::ClipInteraction::find_group(&self.clips, idx);
            let chain_end_idx = *chain.last().unwrap();

            let end_clip = self.clips[chain_end_idx].clone();
            let current_end = end_clip.beat + end_clip.len;

            let next_idx = self.clips.iter().enumerate()
                .filter(|(i, c)| c.track_index == track && !chain.contains(i) && c.beat >= current_end - 0.01)
                .min_by_key(|(_, c)| (c.beat * 1000.0) as i32)
                .map(|(i, _)| i);

            if let Some(next_idx) = next_idx {
                self.record_undo();
                let next_beat = self.clips[next_idx].beat;
                if self.clips[chain_end_idx].pre_glue_len.is_none() {
                    self.clips[chain_end_idx].pre_glue_len = Some(end_clip.len);
                }
                self.clips[chain_end_idx].seam_beat = Some(next_beat);
                if self.clips[next_idx].pre_glue_color.is_none() {
                    self.clips[next_idx].pre_glue_color = Some(self.clips[next_idx].color);
                }
                self.clips[next_idx].color = end_clip.color;
                self.clips[chain_end_idx].len = (next_beat - end_clip.beat).max(0.25);
                self.clip_interaction.select_chain(&self.clips, &chain);
                self.rebuild_wave_data();
            }
        }
    }

    pub fn handle_ruler_click(&mut self, state: ElementState, pos: (f32, f32), ruler_h: f32) {
        match state {
            ElementState::Pressed => {
                // Só inicia o scrub se o clique for dentro da régua.
                if pos.1 < 0.0 || pos.1 > ruler_h { return; }
                self.scrubbing = true;
                self.set_playhead_from_x(pos.0);
            }
            ElementState::Released => {
                // Finaliza o scrub sempre, mesmo se soltar fora da régua
                // (o playhead "dispara" na posição atual como se soltasse dentro).
                if !self.scrubbing { return; }
                self.scrubbing = false;
                self.set_playhead_from_x(pos.0);
            }
        }
    }

    pub fn handle_primary_button(&mut self, state: ElementState, pos: (f32, f32)) {
        self.clip_interaction.sync_selection(self.clips.len());
        let cm = self.clip_metrics();
        let tool = current_tool();

        match state {
            ElementState::Pressed => {
                if pos.1 < self.ruler_px {
                    // Régua → loop drag ou scrub (funciona em qualquer ferramenta)
                    if !self.begin_loop_drag(pos.0) {
                        self.handle_ruler_click(state, pos, self.ruler_px);
                    }
                } else {
                    // Interação com bolinhas de fade (in/out) e volume/ganho
                    if self.fade_interaction.hovered.is_some() {
                        if self.fade_interaction.begin_drag(pos, &self.fades) {
                            return;
                        }
                    }
                    // Seam hover → un-glue (split) — funciona em qualquer ferramenta
                    if let Some(seam_idx) = self.clip_interaction.seam_hover_idx {
                        if self.clip_interaction.is_seam_hovered() {
                            self.record_undo();
                            self.clip_interaction.unglue_seam(&mut self.clips, seam_idx);
                            self.rebuild_wave_data();
                            return;
                        }
                    }
                    match tool {
                        Tool::Select => {
                // Insere áudio arrastado se houver preview pendente.
                if let Some(preview) = self.drop_preview.take() {
                    if let Some(path) = self.drop_path.take() {
                        self.load_audio_file(&path, preview.beat, preview.track);
                    }
                } else {
                    // comportamento padrão já existente
                    if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
                        // Se o clip clicado já está selecionado, mantém a
                        // multiseleção (arrasta todos selecionados). Senão,
                        // colapsa para o grupo de glue.
                        let already_selected = self
                            .clip_interaction
                            .selected
                            .get(idx)
                            .copied()
                            .unwrap_or(false);
                        self.clip_interaction.begin_drag(&self.clips, pos, &cm);
                        if !already_selected {
                            let chain = clips::ClipInteraction::find_group(&self.clips, idx);
                            self.clip_interaction.select_chain(&self.clips, &chain);
                        }
                        self.edge_scroll_x = 0.0;
                        self.edge_scroll_y = 0.0;
                        self.edge_scroll_dir = 0;
                    } else {
                        self.clip_interaction.begin_marquee(pos);
                    }
                }
                        }
                        Tool::Draw => {
                            if pos.1 >= self.ruler_px {
                                let cm = self.clip_metrics();
                                self.clip_interaction.begin_draw(pos, &cm);
                            }
                        }
                        Tool::Stretch => {
                            // Tenta iniciar stretch na borda direita; senão, seleciona
                            if !self.tool_stretch_start(pos) {
                                if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
                                    self.clip_interaction.select_only(idx);
                                } else {
                                    self.clip_interaction.begin_marquee(pos);
                                }
                            }
                        }
                        Tool::Slice => {
                            // Corta clip no ponto do clique
                            self.tool_slice(pos);
                        }
                        Tool::Erase => {
                            // Apaga clip clicado
                            self.tool_erase(pos);
                        }
                        Tool::Glue => {
                            // Cola clips adjacentes
                            self.tool_glue(pos);
                        }
                    }
                }
            }
            ElementState::Released => {
                if self.fade_interaction.is_dragging() {
                    self.fade_interaction.end_drag();
                    return;
                }
                if self.loop_drag.is_some() {
                    self.loop_drag = None;
                } else if self.clip_interaction.is_dragging() {
                    let snap = true;
                    let cm = self.clip_metrics();
                    self.clip_interaction.drag_move(pos, &cm, snap);
                    // Registra undo apenas se houve movimento real
                    if self.clip_interaction.did_drag_move() {
                        self.record_undo();
                    }
                    self.clip_interaction.end_drag(&mut self.clips, &cm, snap);
                    // Reseta auto-scroll ao finalizar o arrasto.
                    self.edge_scroll_x = 0.0;
                    self.edge_scroll_y = 0.0;
                    self.edge_scroll_dir = 0;
                } else if self.clip_interaction.is_drawing() {
                    let cm = self.clip_metrics();
                    self.record_undo();
                    self.clip_interaction.end_draw(&mut self.clips, &cm);
                    self.clip_interaction.sync_selection(self.clips.len());
                    self.rebuild_wave_data();
                } else if self.clip_interaction.is_marqueeing() {
                    self.clip_interaction.marquee_move(pos);
                    let cm = self.clip_metrics();
                    self.clip_interaction.end_marquee(&self.clips, &cm);
                } else if self.scrubbing {
                    self.handle_ruler_click(state, pos, self.ruler_px);
                }
            }
        }
    }

    pub fn handle_cursor_moved(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if self.fade_interaction.is_dragging() {
            if let Some(ref drag) = self.fade_interaction.active_drag {
                if let Some(clip) = self.clips.get(drag.clip_index) {
                    let (_, _, cw, ch) = clips::clip_rect(clip, &cm);
                    let body_w = cw - 2.0;
                    let body_h = ch - 16.0;
                    self.fade_interaction.update_drag(
                        pos,
                        &mut self.fades,
                        self.px_per_beat,
                        clip.len,
                        body_w,
                        body_h,
                    );
                    self.rebuild_wave_data();
                }
            }
        } else if self.loop_drag.is_some() {
            self.handle_loop_drag(pos.0);
        } else if self.clip_interaction.is_dragging() {
            let snap = true;
            self.clip_interaction.drag_move(pos, &cm, snap);
            // Edge scroll é tratado no RedrawRequested (roda todo frame)
        } else if self.clip_interaction.is_drawing() {
            self.clip_interaction.draw_move(pos, &cm);
        } else if self.clip_interaction.is_marqueeing() {
            self.clip_interaction.marquee_move(pos);
        } else if self.scrubbing {
            self.set_playhead_from_x(pos.0);
        }
        self.clip_interaction.update_hover(&self.clips, pos, &cm);

        // Atualiza detecção de hover sobre bolinhas de fade e ganho
        if !self.clip_interaction.is_dragging() && !self.fade_interaction.is_dragging() {
            self.fade_interaction.hovered = None;
            for (i, clip) in self.clips.iter().enumerate() {
                if !clip.is_midi {
                    if let Some(fade) = self.fades.get(i) {
                        let (cx, cy, cw, ch) = clips::clip_rect(clip, &cm);
                        let header_h = 16.0;
                        let body_y = cy + header_h;
                        let body_h = ch - header_h;
                        if let Some(kind) = fades::hit_test_clip_handles(
                            i,
                            fade,
                            cx,
                            body_y,
                            cw,
                            body_h,
                            self.px_per_beat,
                            pos,
                        ) {
                            self.fade_interaction.hovered = Some((i, kind));
                            break;
                        }
                    }
                }
            }
        }

// Cursor override: seam hover → cursor de slice (visual feedback de des-colagem);
// hover na área de ganho → cursor de ganho (gain.cur);
// hover em bolinhas de fade → cursor de mão (hand.cur).
if let Some((_, kind)) = self.fade_interaction.hovered {
    if kind == fades::FadeHandleKind::GainLevel {
        set_gain_override(true);
    } else {
        set_hand_override(true);
    }
} else if self.clip_interaction.is_seam_hovered() {
    set_cursor_override(Tool::Slice);
} else {
    set_gain_override(false);
    set_hand_override(false);
}

        if self.panning && !self.scrubbing && !self.clip_interaction.is_dragging() && !self.clip_interaction.is_drawing() && !self.clip_interaction.is_stretching() {
            let (lx, ly) = self.last_cursor;
            self.scroll_x = (self.scroll_x - (pos.0 - lx)).max(0.0);
            self.scroll_y = (self.scroll_y - (pos.1 - ly)).max(0.0);
            self.clamp_scroll_y(self.size.height as f32);
        }
        // Atualiza a posição do preview de áudio arrastado
        if self.drop_preview.is_some() {
            let (beat, track) = self.cursor_to_snapped_beat_track(pos);
            if let Some(preview) = self.drop_preview.as_mut() {
                preview.beat = beat;
                preview.track = track;
            }
        }

        self.last_cursor = pos;
    }

    /// Detecta quando o cursor está perto das bordas da janela durante um
    /// arrasto de clip e ajusta a velocidade de auto-scroll proporcional à
    /// proximidade (quanto mais perto da borda, mais rápido rola).
    /// Zona de trigger: 40px de cada borda. Velocidade máxima: 24px/frame.
    pub fn update_edge_scroll(&mut self, pos: (f32, f32)) {
        let width = self.size.width as f32;
        let height = self.size.height as f32;
        let edge_zone = 40.0;
        let max_speed = 24.0;

        // Borda esquerda / direita (scroll horizontal)
        if pos.0 < edge_zone {
            // Quanto mais próximo da borda, mais rápido (0..1)
            let proximity = 1.0 - (pos.0 / edge_zone).clamp(0.0, 1.0);
            self.edge_scroll_x = proximity * max_speed;
            self.edge_scroll_dir = -1; // rola para esquerda (conteúdo entra da direita)
        } else if pos.0 > width - edge_zone {
            let proximity = 1.0 - ((width - pos.0) / edge_zone).clamp(0.0, 1.0);
            self.edge_scroll_x = proximity * max_speed;
            self.edge_scroll_dir = 1; // rola para direita
        } else {
            self.edge_scroll_x = 0.0;
        }

        // Borda top / bottom (scroll vertical)
        // Só rola verticalmente se o cursor estiver abaixo da régua
        let body_top = self.ruler_px;
        if pos.1 < body_top + edge_zone {
            // Top edge: rola para cima
            let proximity_y = if pos.1 < body_top {
                // Cursor na régua: proximidade máxima
                1.0
            } else {
                1.0 - ((pos.1 - body_top) / edge_zone).clamp(0.0, 1.0)
            };
            self.edge_scroll_y = proximity_y * max_speed * 0.6; // velocidade Y menor
        } else if pos.1 > height - edge_zone {
            // Bottom edge: rola para baixo
            let proximity_y = 1.0 - ((height - pos.1) / edge_zone).clamp(0.0, 1.0);
            self.edge_scroll_y = proximity_y * max_speed * 0.6;
        } else {
            self.edge_scroll_y = 0.0;
        }
    }

    /// Aplica o auto-scroll calculado no frame atual, atualizando os offsets
    /// de scroll horizontal e vertical. Compensa a posição do clip em arrasto
    /// (drag_live_beat) para que o overlay continue colado ao cursor mesmo
    /// após a viewport rolar.
    pub fn apply_edge_scroll(&mut self) {
        if self.edge_scroll_x == 0.0 && self.edge_scroll_y == 0.0 {
            return;
        }
        let dx = self.edge_scroll_x * self.edge_scroll_dir as f32;
        let dy = self.edge_scroll_y;

        // Ajusta scroll horizontal
        let prev_sx = self.scroll_x;
        self.scroll_x = (self.scroll_x + dx).max(0.0);
        // Ajusta scroll vertical
        self.scroll_y = (self.scroll_y + dy).max(0.0);
        self.clamp_scroll_y(self.size.height as f32);

        // Compensa o beat virtual do clip em arrasto para que ele não "salte"
        // na tela. O overlay é renderizado em:
        //   x = live_beat * px_per_beat - scroll_x
        // Se aumentamos scroll_x em `dx`, precisamos aumentar live_beat em
        // `dx / px_per_beat` para manter a mesma posição na tela.
        // Usa o delta efetivo (não o pretendido) — se scroll_x foi clampado,
        // o clip não deve ser empurrado além do limite.
        if self.clip_interaction.is_dragging() && !self.clip_interaction.is_stretching() {
            let pps = self.px_per_beat.max(0.001);
            let actual_dx = self.scroll_x - prev_sx;
            self.clip_interaction.drag_live_beat += actual_dx / pps;
        }
    }
}