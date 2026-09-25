//! InteraÃ§Ã£o: mouse, scroll, ferramentas, clipboard, loop, foco e edge-scroll.

use crate::app::{ButtonState, KeyAction, LoopDrag, App};
use crate::clips;
use crate::fades;
use crate::tools::{current_tool, set_cursor_override, set_gain_override, set_hand_override, set_vresize_override, set_tool, Tool};
use std::time::Instant;


impl App {
    /// Calcula a resolução do grid (em beats) baseada no zoom atual (px_per_beat).
    /// Retorna o tamanho de cada passo do grid em beats (ex: 1.0 = 1 beat, 0.5 = 1/2 beat, 0.25 = 1/4 beat, etc).
    /// Espelha a lógica de renderização do grid em `render.rs`.
    fn grid_beat_step(&self) -> f32 {
        let px_per_bar = self.px_per_beat * 4.0;
        if px_per_bar >= 960.0 {
            1.0 / 16.0
        } else if px_per_bar >= 480.0 {
            1.0 / 8.0
        } else if px_per_bar >= 240.0 {
            1.0 / 4.0
        } else if px_per_bar >= 120.0 {
            1.0 / 2.0
        } else {
            1.0
        }
    }

    /// Atalhos de teclado do motor (mesma tecla do renderer standalone):
    /// `1..6` ferramentas, `P` piano roll, `F`/`Ctrl+F` foco/follow, `Ctrl+C/A/
    /// Z/Y/V/D`, `Delete/Backspace`, e o rastreio de modificadores. EspaÃ§o e
    /// Esc apenas sinalizam (`TogglePlay`/`ExitWindow`) â€” o motor NÃƒO toca no
    /// transporte nem fecha nada.
    pub fn handle_key(&mut self, event: &winit::event::KeyEvent) -> KeyAction {
        use winit::event::ElementState;
        use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

        let pressed = event.state == ElementState::Pressed;
        let vk = &event.logical_key;

        if pressed {
            if *vk == Key::Named(NamedKey::Space) {
                return KeyAction::TogglePlay;
            }
            if *vk == Key::Named(NamedKey::Escape) {
                return KeyAction::ExitWindow;
            }

            // P: alterna o piano roll.
            if matches!(vk, Key::Character(s) if s.eq_ignore_ascii_case("p")) {
                self.toggle_piano_roll();
                self.toolbar_hover = None;
                self.request_redraw();
            }
            // T: alterna o timewarp (exclusivo com o piano roll).
            if matches!(vk, Key::Character(s) if s.eq_ignore_ascii_case("t")) {
                self.toggle_timewarp();
                self.toolbar_hover = None;
                self.request_redraw();
            }
            // F: foca o clip sob o cursor; Ctrl+F: toggle follow playhead.
            if matches!(vk, Key::Character(s) if s.eq_ignore_ascii_case("f")) {
                if self.ctrl_pressed {
                    self.follow_playhead = !self.follow_playhead;
                } else {
                    self.toggle_clip_focus(self.size.0 as f32);
                }
                self.request_redraw();
            }
            // Ferramentas (1-6).
            if !self.ctrl_pressed && !self.alt_pressed {
                let tool = match vk {
                    Key::Character(s) if s == "1" => Some(Tool::Select),
                    Key::Character(s) if s == "2" => Some(Tool::Draw),
                    Key::Character(s) if s == "3" => Some(Tool::Stretch),
                    Key::Character(s) if s == "4" => Some(Tool::Slice),
                    Key::Character(s) if s == "5" => Some(Tool::Erase),
                    Key::Character(s) if s == "6" => Some(Tool::Glue),
                    _ => None,
                };
                if let Some(tool) = tool {
                    set_tool(tool);
                    self.request_redraw();
                }
            }
            // Clipboard/editing (Ctrl+C, Ctrl+V, Ctrl+D, Ctrl+A, Ctrl+Z,
            // Ctrl+Y, Delete, Backspace).
            if self.ctrl_pressed {
                match vk {
                    Key::Character(s) if s.eq_ignore_ascii_case("c") => {
                        self.copy_selected();
                        self.request_redraw();
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("a") => {
                        self.select_all();
                        self.request_redraw();
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("z") => {
                        if self.shift_pressed {
                            self.redo();
                        } else {
                            self.undo();
                        }
                        self.request_redraw();
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("y") => {
                        self.redo();
                        self.request_redraw();
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("v") => {
                        // Cola na posiÃ§Ã£o do playhead (com snap) na track do
                        // primeiro clip selecionado ou 0.
                        let bpm = self.bpm;
                        let pps = self.px_per_beat * bpm / 60.0;
                        let target_beat = if pps > 0.0 {
                            self.transport_seconds * bpm / 60.0
                        } else {
                            0.0
                        };
                        let target_track = self.clip_interaction.selected.iter()
                            .enumerate()
                            .find(|&(_, &s)| s)
                            .and_then(|(i, _)| self.clips.get(i))
                            .map(|c| c.track_index)
                            .unwrap_or(0);
                        self.paste_at_beat(target_beat, target_track);
                        self.request_redraw();
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("d") => {
                        self.duplicate_selected();
                        self.request_redraw();
                    }
                    _ => {}
                }
            }
            if matches!(
                vk,
                Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)
            ) {
                self.delete_selected();
                self.request_redraw();
            }
        }

        // Track ctrl/alt/shift p/ zoom e shortcuts â€” sÃ³ quando a tecla Ã‰ o
        // modificador.
        let is_ctrl_key = matches!(vk, Key::Named(NamedKey::Control))
            || matches!(event.physical_key,
                PhysicalKey::Code(code)
                    if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight));
        let is_alt_key = matches!(vk, Key::Named(NamedKey::Alt))
            || matches!(event.physical_key,
                PhysicalKey::Code(code)
                    if matches!(code, KeyCode::AltLeft | KeyCode::AltRight));
        let is_shift_key = matches!(vk, Key::Named(NamedKey::Shift))
            || matches!(event.physical_key,
                PhysicalKey::Code(code)
                    if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight));
        if is_ctrl_key {
            self.ctrl_pressed = pressed;
        }
        if is_alt_key {
            self.alt_pressed = pressed;
        }
        if is_shift_key {
            self.shift_pressed = pressed;
        }

        // Qualquer tecla pressionada marca o frame como sujo (a cena da
        // timeline re-compÃµe â€” no hosted o prÃ³ximo `tick` re-renderiza).
        if pressed {
            self.request_redraw();
        }

        KeyAction::None
    }
    pub fn handle_scroll(&mut self, dx: f32, dy: f32, with_ctrl: bool, with_alt: bool) {
        let (mx, my) = self.last_cursor;

        // Determina qual área está sob o cursor: timeline, piano roll ou
        // timewarp (cada uma com zoom/scroll INDEPENDENTES).
        let layout = self.layout(self.size.0 as f32, self.size.1 as f32);
        let (px_per_beat, scroll_x, scroll_y, in_pr_area, in_tw_area) = if self.show_timewarp {
            let tw = layout.timewarp.as_ref();
            if let Some(tw) = tw {
                if my >= tw.y && my <= tw.y + tw.height {
                    (self.timewarp_px_per_beat, self.timewarp_scroll_x, 0.0, false, true)
                } else {
                    (self.px_per_beat, self.scroll_x, self.scroll_y, false, false)
                }
            } else {
                (self.px_per_beat, self.scroll_x, self.scroll_y, false, false)
            }
        } else if self.show_piano_roll {
            let pr = layout.piano_roll.as_ref();
            if let Some(pr) = pr {
                if my >= pr.y && my <= pr.y + pr.height {
                    (self.piano_roll_px_per_beat, self.piano_roll_scroll_x, self.piano_roll_scroll_y, true, false)
                } else {
                    (self.px_per_beat, self.scroll_x, self.scroll_y, false, false)
                }
            } else {
                (self.px_per_beat, self.scroll_x, self.scroll_y, false, false)
            }
        } else {
            (self.px_per_beat, self.scroll_x, self.scroll_y, false, false)
        };

        if with_ctrl && dy.abs() >= 1.0 {
            let old_px = px_per_beat;
            let new_px = (old_px + dy * 0.05).clamp(8.0, 640.0);
            let beat_under = (mx + scroll_x) / old_px.max(0.01);
            if in_tw_area {
                self.timewarp_px_per_beat = new_px;
                self.timewarp_scroll_x = (beat_under * new_px - mx).max(0.0);
            } else if in_pr_area {
                self.piano_roll_px_per_beat = new_px;
                self.piano_roll_scroll_x = (beat_under * new_px - mx).max(0.0);
            } else {
                self.px_per_beat = new_px;
                self.scroll_x = (beat_under * new_px - mx).max(0.0);
            }
        } else if with_alt && dy.abs() >= 1.0 {
            if in_pr_area {
                let old_kh = self.piano_roll_key_height;
                let new_kh = (old_kh + dy * 0.1).clamp(2.0, 24.0);
                self.piano_roll_key_height = new_kh;
            } else if !in_tw_area {
                let old_zy = self.zoom_y;
                let new_zy = (old_zy + dy * 0.01).clamp(0.25, 6.0);
                let row_under = (my - self.ruler_px + scroll_y) / (64.0 * old_zy.max(0.01)).max(0.01);
                self.zoom_y = new_zy;
                self.scroll_y = (row_under * 64.0 * new_zy - (my - self.ruler_px)).max(0.0);
            }
        } else {
            if in_pr_area {
                self.piano_roll_scroll_y = (self.piano_roll_scroll_y - dy).max(0.0);
                self.piano_roll_scroll_x = (self.piano_roll_scroll_x - dx).max(0.0);
            } else if in_tw_area {
                // Timewarp não tem scroll vertical (faixa BPM fixa).
                self.timewarp_scroll_x = (self.timewarp_scroll_x - dx).max(0.0);
            } else {
                self.scroll_x = (self.scroll_x - dx).max(0.0);
                self.scroll_y = (self.scroll_y - dy).max(0.0);
            }
        }
        self.clamp_scroll_y(self.input_view_size().1);
        eprintln!(
            "[dbg-scroll] ctrl={with_ctrl} alt={with_alt} dx={dx} dy={dy} -> scroll_x={} scroll_y={} pps={}",
            self.scroll_x, self.scroll_y, self.px_per_beat,
        );
    }

    pub fn handle_mouse_button(&mut self, state: ButtonState, pos: Option<(f32, f32)>) {
        match state {
            ButtonState::Pressed => {
                self.panning = true;
                if let Some(p) = pos { self.last_cursor = p; }
            }
            ButtonState::Released => {
                // Se houver preview de Ã¡udio pendente, insere ao liberar o botÃ£o.
                    if let Some(preview) = self.drop_preview.take() {
                        if let Some(path) = self.drop_path.take() {
                            self.load_audio_file(&path, preview.beat, preview.track);
                            // Inicia playback a partir do ponto onde o Ã¡udio foi solto.
                            self.start_playback_at(preview.beat);
                        }
                    }
                self.panning = false;
            }
        }
    }

    /// Inicia o arrasto do loop na rÃ©gua a partir da posiÃ§Ã£o x do cursor.
    /// Retorna true se iniciou um arrasto de loop; false para comportamento padrÃ£o.
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

    /// Move a borda/seleÃ§Ã£o do loop com snap de 1 beat (arredonda o beat).
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

    /// Inicia (Pressed) ou finaliza (Released) o *scrub* do playhead na rÃ©gua.
    /// Enquanto arrasta, `handle_cursor_moved` chama `set_playhead_from_x` em
    /// tempo real para seguir o cursor.
    pub fn handle_ruler_click(&mut self, state: ButtonState, pos: (f32, f32), _ruler_px: f32) {
        match state {
            ButtonState::Pressed => {
                self.scrubbing = true;
                self.set_playhead_from_x(pos.0);
            }
            ButtonState::Released => {
                self.scrubbing = false;
            }
        }
    }

    /// Alterna o foco no clip SELECIONADO: F centraliza o clip na view e
    /// lembra o estado anterior; F de novo restaura o estado anterior ao foco.
    /// Sem seleÃ§Ã£o, usa o clip sob o cursor.
    pub fn toggle_clip_focus(&mut self, width: f32) {
        if let Some((sx, ppb, sy, zoom_y)) = self.focus_saved.take() {
            self.scroll_x = sx;
            self.px_per_beat = ppb;
            self.scroll_y = sy;
            self.zoom_y = zoom_y;
            return;
        }

        // Prioriza o primeiro clip selecionado; senÃ£o usa o hover.
        let idx = self
            .clip_interaction
            .selected
            .iter()
            .position(|&s| s)
            .or(self.clip_interaction.hover_idx);

        let Some(idx) = idx else { return };
        let Some(clip) = self.clips.get(idx) else { return };

        self.focus_saved = Some((
            self.scroll_x,
            self.px_per_beat,
            self.scroll_y,
            self.zoom_y,
        ));

        // Centraliza horizontal e verticalmente no clip selecionado.
        let clip_x = clip.beat * self.px_per_beat;
        self.scroll_x = (clip_x - width * 0.5).max(0.0);

        let row_height = 64.0 * self.zoom_y.max(0.001);
        let view_h = (self.size.1 as f32 - self.ruler_px - self.time_ruler_h).max(0.0);
        let clip_y = clip.track_index as f32 * row_height;
        self.scroll_y = (clip_y - view_h * 0.5).max(0.0);
    }

    /// Copia os clips selecionados para o clipboard, preservando posiÃ§Ãµes relativas.
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
    /// Limpa a pilha de redo (novo ramo de ediÃ§Ã£o). Deve ser chamado ANTES de
    /// qualquer mutaÃ§Ã£o estrutural (import, paste, slice, erase, etc.).
    pub fn record_undo(&mut self) {
        use crate::app::TimelineSnapshot;
        self.undo_stack.push(TimelineSnapshot {
            clips: self.clips.clone(),
            fades: self.fades.clone(),
        });
        self.redo_stack.clear();
    }

    /// Desfaz a Ãºltima ediÃ§Ã£o estrutural (clips + fades).
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

    /// Refaz a ediÃ§Ã£o que foi desfeita.
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

    /// Cola os clips do clipboard na posiÃ§Ã£o alvo (beat, track), com snap ao grid dinâmico.
    /// MantÃ©m a estrutura relativa dos clips copiados.
    pub fn paste_at_beat(&mut self, target_beat: f32, target_track: usize) {
        if self.clipboard.is_empty() {
            return;
        }
        self.record_undo();
        // Encontra o beat mÃ­nimo dos clips copiados para manter offsets relativos
        let min_beat = self.clipboard.iter().map(|c| c.beat).fold(f32::INFINITY, f32::min);
        let snap_beat = if self.snap_enabled {
            let step = self.grid_beat_step();
            (target_beat / step).round() * step
        } else {
            target_beat
        }.max(0.0);
        let beat_offset = snap_beat - min_beat;
        // Track offset: usa a track do primeiro clip copiado como referÃªncia
        let min_track = self.clipboard.iter().map(|c| c.track_index).min().unwrap_or(0);
        let track_offset = target_track as i32 - min_track as i32;
        let max_track = self.clips.iter().map(|c| c.track_index).max().unwrap_or(0);
        for clip in &self.clipboard {
            let mut new_clip = clip.clone();
            new_clip.id = 0;
            new_clip.beat = (clip.beat + beat_offset).max(0.0);
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
        self.emit_clip_write_back();
    }

    /// Duplica os clips selecionados APÃ“S o Ãºltimo clip selecionado (com snap ao grid).
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
        let snap_beat = if self.snap_enabled {
            let step = self.grid_beat_step();
            (max_end / step).ceil() * step
        } else {
            max_end
        }.max(0.0);
        let clips_to_dup: Vec<clips::ClipDemo> = selected_indices.iter().map(|&i| self.clips[i].clone()).collect();
        let min_beat = clips_to_dup.iter().map(|c| c.beat).fold(f32::INFINITY, f32::min);
        let beat_offset = snap_beat - min_beat;
        for clip in &clips_to_dup {
            let mut new_clip = clip.clone();
            new_clip.id = 0;
            new_clip.beat = (clip.beat + beat_offset).max(0.0);
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
        self.emit_clip_write_back();
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
        // Remove em ordem reversa para manter Ã­ndices vÃ¡lidos
        for &i in selected_indices.iter().rev() {
            self.clips.remove(i);
        }
        self.clip_interaction.sync_selection(self.clips.len());
        self.rebuild_wave_data();
        self.emit_clip_write_back();
    }

    /// Dispara o write-back estruturado de clips para o host (hosted).
    fn emit_clip_write_back(&mut self) {
        if let Some(cb) = &mut self.clip_write_back {
            cb(&self.clips);
        }
    }

    /// Converte posiÃ§Ã£o do cursor para beat e track com snap ao grid
    /// (respeitando `snap_enabled`: OFF mantém beat fracionário).
    /// O snap segue o grid visual dinâmico: quanto mais zoom, mais fino o grid.
    pub fn cursor_to_snapped_beat_track(&self, pos: (f32, f32)) -> (f32, usize) {
        let beat = (pos.0 + self.scroll_x) / self.px_per_beat.max(0.001);
        let snapped_beat = if self.snap_enabled {
            let step = self.grid_beat_step();
            (beat / step).round() * step
        } else {
            beat
        }.max(0.0);
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
        let len = 4.0; // comprimento padrÃ£o: 1 compasso
        self.clips.push(clips::ClipDemo {
            id: 0,
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
        self.emit_clip_write_back();
    }

    /// Estica clip (ferramenta Stretch) - arrasta borda direita.
    pub fn tool_stretch_start(&mut self, pos: (f32, f32)) -> bool {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            // Verifica se clicou na borda direita do clip (Ãºltimos 12px)
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

    /// Corta clip (ferramenta Slice â€” tecla 4).
    /// Divide o clip no ponto do clique. O clip da esquerda (original) Ã© selecionado,
    /// entÃ£o cortes consecutivos deixam o clip do meio selecionado automaticamente.
    pub fn tool_slice(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            let clip = self.clips[idx].clone();
            let click_beat = (pos.0 + self.scroll_x) / self.px_per_beat.max(0.001);
            let click_beat_snapped = if self.snap_enabled {
                let step = self.grid_beat_step();
                (click_beat / step).round() * step
            } else {
                click_beat
            }.max(clip.beat).min(clip.beat + clip.len);

            // SÃ³ divide se o clique nÃ£o estiver na borda
            if click_beat_snapped > clip.beat && click_beat_snapped < clip.beat + clip.len {
                self.record_undo();
                let left_len = click_beat_snapped - clip.beat;
                let right_len = clip.beat + clip.len - click_beat_snapped;

                // Atualiza clip original para a parte esquerda
                self.clips[idx].len = left_len;

                // Cria novo clip para a parte direita
                let mut right_clip = clip;
                right_clip.id = 0;
                right_clip.beat = click_beat_snapped;
                right_clip.len = right_len;

                // Corte VERDADEIRO da waveform: divide as amostras do clip
                // pai, proporcional ao ponto de corte. As duas peÃ§as exibem
                // exatamente o conteÃºdo original, apenas cortado â€” nunca uma
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
                // Seleciona o clip esquerdo (parte que foi cortada) â€”
                // cortes consecutivos mantÃªm o clip do meio selecionado.
                self.clip_interaction.select_only(idx);
                // A segmentaÃ§Ã£o do buffer muda (um clip virou dois): regenera
                // para o clip direito tambÃ©m ter waveform.
                self.rebuild_wave_data();
                self.emit_clip_write_back();
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
            self.emit_clip_write_back();
        }
    }

    /// Cola clips (ferramenta Glue â€” tecla 6).
    /// Clicar num clip: estica a borda direita dele (ou do Ãºltimo da cadeia)
    /// atÃ© o inÃ­cio do prÃ³ximo clip na mesma track, preenchendo o gap.
    /// Ambos permanecem como clips separados, mas adjacentes (sem gap),
    /// com a mesma cor. O seam marca o ponto de encontro para des-colagem.
    /// Clicar de novo no mesmo clip continua a cadeia grudando no prÃ³ximo.
    pub fn tool_glue(&mut self, pos: (f32, f32)) {
        let cm = self.clip_metrics();
        if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
            let track = self.clips[idx].track_index;

            // Encontra o final da cadeia de glue (Ãºltimo clip com seam)
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
                self.emit_clip_write_back();
            }
        }
    }

    /// Inicia a reproduÃ§Ã£o a partir de um beat especificado.
    fn start_playback_at(&mut self, beat: f32) {
        // Converte beat para segundos.
self.transport_seconds = beat * 60.0 / self.bpm;
        // Sincroniza o motor de Ã¡udio.
        #[cfg(feature = "audio")]
        self.sync_audio_engine();
        // Configura o transporte e inicia a reproduÃ§Ã£o.
        if let Ok(mut t) = self.transport.lock() {
            t.bpm = self.bpm as u32;
            t.loop_enabled = self.loop_enabled;
            if self.loop_enabled {
                t.loop_start_ticks = (self.loop_start_beat * t.ppq as f32) as u64;
                t.loop_end_ticks = (self.loop_end_beat * t.ppq as f32) as u64;
            }
            let beats = self.transport_seconds * self.bpm / 60.0;
            t.position_ticks = (beats * t.ppq as f32) as u64;
            t.playing = true;
        }
self.play_start_seconds = self.transport_seconds;
        self.playing = true;
        self.last_play_head_update = Instant::now();
        self.request_redraw();
    }


    pub fn handle_primary_button(&mut self, state: ButtonState, pos: (f32, f32)) {
        // ====== Piano roll: Released sempre finaliza drags de loop/scrub ======
        // (coloquei ANTES dos "early returns de arrasto ativo" porque, se um
        //  drag estiver ativo, Released tem que encerrÃ¡-lo SEMPRE.)
        if self.show_piano_roll && state == ButtonState::Released {
            if self.pr_loop_drag.is_some() {
                self.pr_loop_drag = None;
            }
            if self.pr_scrubbing {
                self.pr_handle_ruler_click(state, pos);
            }
        }
        if self.show_timewarp && state == ButtonState::Released {
            if self.tw_scrubbing {
                self.tw_handle_ruler_click(state, pos);
            }
        }

        // Intercepta arrasto da splitter da área inferior (PR ou timewarp)
        if (self.show_piano_roll || self.show_timewarp) && self.splitter_drag {
            if state == ButtonState::Released {
                self.end_splitter_drag();
            } else {
                self.splitter_drag_move(pos.1);
            }
            return;
        }

        // Piano roll: durante arrasto de loop ativo (Pressed) â€” nada a fazer,
        // Released jÃ¡ foi tratado acima. Para garantir que nÃ£o re-ativamos por
        // engano, damos return quando hÃ¡ um loop drag ativo (sÃ³ importa no
        // Pressed, mas no Released o bloco acima jÃ¡ o zerou).
        if self.show_piano_roll && self.pr_loop_drag.is_some() {
            return;
        }

        // Intercepta clique no SCRUBBING do piano roll (sÃ³ durante arrasto ativo / Pressed)
        if self.show_piano_roll && self.pr_scrubbing && state == ButtonState::Pressed {
            // Durante scrubbing + Pressed: realinha a playhead (redundante com
            // cursor_moved, mas garante boa UX em clicks rÃ¡pidos).
            self.pr_handle_ruler_click(state, pos);
            return;
        }

        // Intercepta clique dentro da Ã¡rea do piano roll ou splitter (quando visÃ­vel)
        if self.show_piano_roll {
            let height = self.size.1 as f32;
            let width = self.size.0 as f32;
            let layout = self.layout(width, height);
            // Clique na splitter
            if self.splitter_hit_test(pos.1, height) {
                self.splitter_drag = true;
                return;
            }
            // Clique na RÃ‰GUA do piano roll (loop drag + scrub) â€” SEPARADO do conteÃºdo
            if let Some(pr_ruler) = &layout.piano_ruler {
                let ruler_top = pr_ruler.y;
                let ruler_bottom = pr_ruler.y + pr_ruler.height;
                if pos.1 >= ruler_top && pos.1 < ruler_bottom {
                    // Na rÃ©gua: PRIMEIRO tenta loop drag (apenas alÃ§as 6px,
                    // nÃ£o o centro). Se falhar â†’ scrub/playhead.
                    match state {
                        ButtonState::Pressed => {
                            if !self.pr_begin_loop_drag(pos.0) {
                                self.pr_handle_ruler_click(state, pos);
                            }
                        }
                        ButtonState::Released => {
                            // Released jÃ¡ foi tratado no topo da funÃ§Ã£o (fecha
                            // scrub/loop), mas garantimos aqui que, caso o
                            // usuÃ¡rio tenha feito um clique rÃ¡pido (Pressed +
                            // Released sem mover), o pr_handle_ruler_click seja
                            // chamado no Released caso o scrub ainda nÃ£o existia
                            // (Pressed caiu aqui via begin_loop_drag=false e
                            // setou pr_scrubbing = true).
                            // Esse bloco Ã© safety-net â€” o bloco do topo jÃ¡
                            // cobre o caso geral.
                            self.pr_handle_ruler_click(state, pos);
                        }
                    }
                    return;
                }
            }
            // Clique no CONTEÃšDO do piano roll (abaixo da rÃ©gua)
            if let Some(pr_ruler) = &layout.piano_ruler {
                if pos.1 >= pr_ruler.y + pr_ruler.height {
                    self.handle_piano_roll_click(state, pos);
                    return;
                }
            }
        }

        // Intercepta clique dentro da área do TIMEWARP ou splitter (visível)
        if self.show_timewarp {
            let height = self.size.1 as f32;
            let width = self.size.0 as f32;
            let layout = self.layout(width, height);
            if self.splitter_hit_test(pos.1, height) {
                self.splitter_drag = true;
                return;
            }
            // Clique na régua do timewarp (scrub da playhead própria)
            if let Some(tw_ruler) = &layout.timewarp_ruler {
                if pos.1 >= tw_ruler.y && pos.1 < tw_ruler.y + tw_ruler.height {
                    self.tw_handle_ruler_click(state, pos);
                    return;
                }
            }
            // Clique no CONTEÚDO do timewarp (abaixo da régua)
            if let Some(tw) = &layout.timewarp {
                if pos.1 >= tw.y {
                    self.handle_timewarp_click(state, pos);
                    return;
                }
            }
        }

        self.clip_interaction.sync_selection(self.clips.len());
        let cm = self.clip_metrics();
        let tool = current_tool();

        match state {
            ButtonState::Pressed => {
                if pos.1 < self.ruler_px {
                    // RÃ©gua â†’ loop drag ou scrub (funciona em qualquer ferramenta)
                    if !self.begin_loop_drag(pos.0) {
                        self.handle_ruler_click(state, pos, self.ruler_px);
                    }
                } else {
                    // InteraÃ§Ã£o com bolinhas de fade (in/out) e volume/ganho
                    if let Some((clip_idx, _kind)) = self.fade_interaction.hovered {
                        if let Some(clip) = self.clips.get(clip_idx) {
                            let (cx, cy, cw, ch) = clips::clip_rect(clip, &cm);
                            let header_h = 16.0;
                            let body_x = cx;
                            let body_y = cy + header_h;
                            let body_w = cw;
                            let body_h = ch - header_h;
                            if body_h > 0.0 && body_w > 0.0 {
                                if self.fade_interaction.begin_drag(
                                    pos,
                                    &self.fades,
                                    self.px_per_beat,
                                    body_x, body_y, body_w, body_h,
                                ) {
                                    return;
                                }
                            }
                        }
                    }
                    // Seam hover â†’ un-glue (split) â€” funciona em qualquer ferramenta
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
                // Insere Ã¡udio arrastado se houver preview pendente.
                    if let Some(preview) = self.drop_preview.take() {
                        if let Some(path) = self.drop_path.take() {
                            self.load_audio_file(&path, preview.beat, preview.track);
                            // Inicia playback a partir do ponto onde o Ã¡udio foi solto.
                            self.start_playback_at(preview.beat);
                        }
                    } else {
                    // comportamento padrÃ£o jÃ¡ existente
                    {
                        let pps = self.px_per_beat.max(0.001);
                        let hb = (pos.0 + self.scroll_x) / pps;
                        let ht = ((pos.1 - self.ruler_px + self.scroll_y) / cm.row_height).floor().max(0.0) as usize;
                        let h = self.clip_interaction.hit_test(&self.clips, pos, &cm);
                        eprintln!(
                            "[dbg-hit] pos={pos:?} -> beat={hb:.2} track={ht} n={} ruler={} pps={pps:.1} hit={h:?}",
                            self.clips.len(),
                            self.ruler_px,
                        );
                    }
                    if let Some(idx) = self.clip_interaction.hit_test(&self.clips, pos, &cm) {
                        // multiseleÃ§Ã£o (arrasta todos selecionados). SenÃ£o,
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
                            // Tenta iniciar stretch na borda direita; senÃ£o, seleciona
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
            ButtonState::Released => {
                if self.fade_interaction.is_dragging() {
                    self.fade_interaction.end_drag();
                    return;
                }
                if self.loop_drag.is_some() {
                    self.loop_drag = None;
                    // Região de loop editada na régua → Transport compartilhado
                    // (motor de áudio + espelho do host usam a MESMA faixa).
                    self.emit_loop_write_back();
                } else if self.clip_interaction.is_dragging() {
                    let snap = self.snap_enabled;
                    let cm = self.clip_metrics();
                    self.clip_interaction.drag_move(pos, &cm, snap);
                    // Registra undo apenas se houve movimento real
                    if self.clip_interaction.did_drag_move() {
                        self.record_undo();
                    }
                    self.clip_interaction.end_drag(&mut self.clips, &cm, snap);
                    // Enganche o write-back estruturado de clips (hosted).
                    self.emit_clip_write_back();
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
                    self.emit_clip_write_back();
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
        // ====== CRÍTICO (winit 0.30): atualiza last_cursor ANTES de qualquer
        // lógica. O evento MouseInput (clique) NÃO traz a posição — ele usa
        // `app.last_cursor` para saber onde o clique foi. O CursorMoved
        // atualiza last_cursor; garanta que ele rode antes do primeiro clique.
        self.last_cursor = pos;

        // Se a splitter estÃ¡ sendo arrastada, redimensiona o piano roll em tempo real
        if self.splitter_drag {
            self.splitter_drag_move(pos.1);
            return;
        }

        // ====== Piano roll: loop drag / scrubbing (INDEPENDENTES da timeline) ======
        if self.show_piano_roll {
            if self.pr_loop_drag.is_some() {
                self.pr_handle_loop_drag(pos.0);
            } else if self.pr_scrubbing {
                self.pr_set_playhead_from_x(pos.0);
            }
        }

        // ====== Timewarp: scrub na régua / arrasto/roving de pontos ======
        if self.show_timewarp {
            if self.tw_scrubbing {
                self.tw_set_scrub_beat(pos.0);
                self.request_redraw();
            } else if self.timewarp_inter.is_dragging() {
                let m = self.timewarp_metrics();
                // Segurar Alt desliga o snap (beat/BPM livre).
                self.timewarp_inter.drag_move(pos, &m, !self.alt_pressed);
                self.request_redraw();
            } else {
                let m = self.timewarp_metrics();
                let points = self.tw_points();
                self.timewarp_inter.update_hover(&points, pos, &m);
            }
        }

        let cm = self.clip_metrics();
        if self.fade_interaction.is_dragging() {
            if let Some(ref drag) = self.fade_interaction.active_drag {
                // MantÃ©m o cursor de override enquanto arrasta o handle.
                match drag.kind {
                    fades::FadeHandleKind::GainLevel => set_gain_override(true),
                    _ => set_hand_override(true),
                }
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
                    // Se jÃ¡ estÃ¡ tocando, sincroniza ganho em tempo real.
                    #[cfg(feature = "audio")]
                    if self.playing {
                        self.sync_audio_engine();
                    }
                }
            }
        } else if self.loop_drag.is_some() {
            self.handle_loop_drag(pos.0);
        } else if self.clip_interaction.is_dragging() {
            let snap = self.snap_enabled;
            self.clip_interaction.drag_move(pos, &cm, snap);
            // Edge scroll Ã© tratado no RedrawRequested (roda todo frame)
        } else if self.clip_interaction.is_drawing() {
            self.clip_interaction.draw_move(pos, &cm);
        } else if self.clip_interaction.is_marqueeing() {
            self.clip_interaction.marquee_move(pos);
        } else if self.scrubbing {
            self.set_playhead_from_x(pos.0);
        }
        self.clip_interaction.update_hover(&self.clips, pos, &cm);

        // Atualiza detecÃ§Ã£o de hover sobre bolinhas de fade e ganho
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

        // Snap magnÃ©tico: quando o cursor chega perto de um handle, puxa-o
        // para o centro exato da bolinha â€” uma vez sÃ³. ApÃ³s o snap, o cursor
        // pode sair livremente (sem resistÃªncia) atÃ© perder o hover.
        if !self.scrubbing
            && !self.clip_interaction.is_dragging()
            && !self.fade_interaction.is_dragging()
        {
            if let Some((clip_idx, kind)) = self.fade_interaction.hovered {
                if let Some((clip, fade)) =
                    self.clips.get(clip_idx).zip(self.fades.get(clip_idx))
                {
                    let (cx, cy, cw, ch) = clips::clip_rect(clip, &cm);
                    let header_h = 16.0;
                    let body_x = cx;
                    let body_y = cy + header_h;
                    let body_w = cw;
                    let body_h = ch - header_h;
                    if let Some((hx, hy)) = fades::handle_position(
                        kind,
                        fade,
                        body_x,
                        body_y,
                        body_w,
                        body_h,
                        self.px_per_beat,
                    ) {
                        let dist_sq =
                            (pos.0 - hx) * (pos.0 - hx) + (pos.1 - hy) * (pos.1 - hy);
                        let snap_threshold = 14.0;
// Snap apenas se ainda nÃ£o foi disparado nesta sessÃ£o de hover
                        if !self.fade_interaction.snap_active
                            && dist_sq <= snap_threshold * snap_threshold
                        {
                            if let Some(window) = &self.window {
                                use winit::dpi::LogicalPosition;
                                let _ = window.set_cursor_position(LogicalPosition::new(hx, hy));
                            }
                            self.cursor_warp_pending = true;
                            self.last_cursor = (hx, hy);
                            self.fade_interaction.snap_active = true;
                        }
                    }
                }
            }
        } else {
            // Cursor saiu do hover â€” reseta o flag para permitir snap novamente
            self.fade_interaction.snap_active = false;
        }



// - Enquanto arrasta um handle, forÃ§a o cursor apropriado (hand ou gain).
// - Quando nÃ£o estÃ¡ arrastando, usa hover para escolher o cursor.
// - Caso contrÃ¡rio, limpa quaisquer sobrescriÃ§Ãµes de cursor.
// Primeiro limpa ambos para garantir estado consistente.
set_gain_override(false);
set_hand_override(false);
if self.fade_interaction.is_dragging() {
    if let Some(ref drag) = self.fade_interaction.active_drag {
        match drag.kind {
            fades::FadeHandleKind::GainLevel => set_gain_override(true),
            _ => set_hand_override(true),
        }
    }
} else if let Some((_, kind)) = self.fade_interaction.hovered {
    if kind == fades::FadeHandleKind::GainLevel {
        set_gain_override(true);
    } else {
        set_hand_override(true);
    }
        } else if self.clip_interaction.is_seam_hovered() {
    set_cursor_override(Tool::Slice);
}

    // Cursor de resize vertical sobre a splitter do piano roll
    if self.splitter_drag || self.splitter_hit_test(self.last_cursor.1, self.size.1 as f32) {
        set_vresize_override(true);
    } else {
        set_vresize_override(false);
    }

    if self.panning && !self.scrubbing && !self.clip_interaction.is_dragging() && !self.clip_interaction.is_drawing() && !self.clip_interaction.is_stretching() {
            let (lx, ly) = self.last_cursor;
            self.scroll_x = (self.scroll_x - (pos.0 - lx)).max(0.0);
            self.scroll_y = (self.scroll_y - (pos.1 - ly)).max(0.0);
            self.clamp_scroll_y(self.input_view_size().1);
        }
        // Atualiza a posiÃ§Ã£o do preview de Ã¡udio arrastado
        if self.drop_preview.is_some() {
            let (beat, track) = self.cursor_to_snapped_beat_track(pos);
            if let Some(preview) = self.drop_preview.as_mut() {
                preview.beat = beat;
                preview.track = track;
            }
        }

        let mut constrained = pos;
        if self.fade_interaction.is_dragging() {
            if let Some(ref drag) = self.fade_interaction.active_drag {
                match drag.axis_lock {
                    Some(fades::DragAxisLock::Horizontal) => {
                        // fixa a coordenada Y ao inÃ­cio do arrasto
                        constrained.1 = drag.start_cursor.1;
                    }
                    Some(fades::DragAxisLock::Vertical) => {
                        // fixa a coordenada X ao inÃ­cio do arrasto
                        constrained.0 = drag.start_cursor.0;
                    }
                    None => {}
                }

            }
        }
        self.last_cursor = constrained;

        // O cursor fÃ­sico NÃƒO Ã© repospicionado durante o arrasto (trava de eixo).
        // O lock jÃ¡ Ã© aplicado em `update_drag` (dx/dy = 0) e aqui via `constrained`
        // para o cÃ¡lculo de panning. Warpar o cursor a cada frame causa jitter
        // por loop de feedback (warp â†’ CursorMoved â†’ warp â†’ ...).
    }

    /// Detecta quando o cursor estÃ¡ perto das bordas da janela durante um
    /// arrasto de clip e ajusta a velocidade de auto-scroll proporcional Ã 
    /// proximidade (quanto mais perto da borda, mais rÃ¡pido rola).
    /// Zona de trigger: 40px de cada borda. Velocidade mÃ¡xima: 24px/frame.
    pub fn update_edge_scroll(&mut self, pos: (f32, f32)) {
        let (width, height) = self.input_view_size();
        let edge_zone = 40.0;
        let max_speed = 24.0;

        // Borda esquerda / direita (scroll horizontal)
        if pos.0 < edge_zone {
            // Quanto mais prÃ³ximo da borda, mais rÃ¡pido (0..1)
            let proximity = 1.0 - (pos.0 / edge_zone).clamp(0.0, 1.0);
            self.edge_scroll_x = proximity * max_speed;
            self.edge_scroll_dir = -1; // rola para esquerda (conteÃºdo entra da direita)
        } else if pos.0 > width - edge_zone {
            let proximity = 1.0 - ((width - pos.0) / edge_zone).clamp(0.0, 1.0);
            self.edge_scroll_x = proximity * max_speed;
            self.edge_scroll_dir = 1; // rola para direita
        } else {
            self.edge_scroll_x = 0.0;
        }

        // Borda top / bottom (scroll vertical)
        // SÃ³ rola verticalmente se o cursor estiver abaixo da rÃ©gua
        let body_top = self.ruler_px;
        if pos.1 < body_top + edge_zone {
            // Top edge: rola para cima
            let proximity_y = if pos.1 < body_top {
                // Cursor na rÃ©gua: proximidade mÃ¡xima
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
    /// de scroll horizontal e vertical. Compensa a posiÃ§Ã£o do clip em arrasto
    /// (drag_live_beat) para que o overlay continue colado ao cursor mesmo
    /// apÃ³s a viewport rolar.
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
        let prev_sy = self.scroll_y;
        self.scroll_y = (self.scroll_y + dy).max(0.0);
        self.clamp_scroll_y(self.input_view_size().1);

        // Compensa o beat virtual do clip em arrasto para que ele nÃ£o "salte"
        // na tela. O overlay Ã© renderizado em:
        //   x = live_beat * px_per_beat - scroll_x
        // Se aumentamos scroll_x em `dx`, precisamos aumentar live_beat em
        // `dx / px_per_beat` para manter a mesma posiÃ§Ã£o na tela.
        // Usa o delta efetivo (nÃ£o o pretendido) â€” se scroll_x foi clampado,
        // o clip nÃ£o deve ser empurrado alÃ©m do limite.
        //
        // CRÃTICO: `drag_move` recalcula live_beat/live_track a partir de
        // `drag_start_cursor` (âncora) a cada evento de move. Compensar apenas
        // `drag_live_beat` seria apagado no move seguinte (o clip "para" na
        // timeline enquanto a view continua rolando). Por isso o âncora também
        // é compensado em PX (na direção oposta ao scroll), fazendo o delta
        // acumular o scroll e o clip permanecer colado ao cursor.
        if self.clip_interaction.is_dragging() && !self.clip_interaction.is_stretching() {
            let pps = self.px_per_beat.max(0.001);
            let actual_dx = self.scroll_x - prev_sx;
            let actual_dy = self.scroll_y - prev_sy;
            if actual_dx != 0.0 {
                self.clip_interaction.drag_start_cursor.0 -= actual_dx;
                self.clip_interaction.drag_live_beat += actual_dx / pps;
            }
            if actual_dy != 0.0 {
                self.clip_interaction.drag_start_cursor.1 -= actual_dy;
            }
        }
        // Mesma compensação de âncora para os handles de fade/ganho: o
        // `update_drag` recalcula o delta a partir de `start_cursor` a cada
        // move, então o âncora precisa acompanhar o scroll para o knob
        // permanecer colado ao cursor mesmo com auto-scroll nas bordas.
        if self.fade_interaction.is_dragging() {
            if let Some(ref mut drag) = self.fade_interaction.active_drag {
                let actual_dx = self.scroll_x - prev_sx;
                let actual_dy = self.scroll_y - prev_sy;
                if actual_dx != 0.0 {
                    drag.start_cursor.0 -= actual_dx;
                }
                if actual_dy != 0.0 {
                    drag.start_cursor.1 -= actual_dy;
                }
            }
        }
    }

/// Atualiza o pan (botão do meio) consultando a posição atual do mouse via Win32.
/// Útil quando o SO não envia eventos CursorMoved durante o drag do botão do meio.
pub fn update_panning_from_mouse(&mut self) {
    if let Some(ref win) = self.window {
        if let Some(pos) = crate::app::window::current_cursor_pos(win) {
            let (lx, ly) = self.last_cursor;
            self.scroll_x = (self.scroll_x - (pos.0 - lx)).max(0.0);
            self.scroll_y = (self.scroll_y - (pos.1 - ly)).max(0.0);
            self.clamp_scroll_y(self.input_view_size().1);
            self.last_cursor = pos;
            self.request_redraw();
        }
    }
}

}

// ===== Timewarp interaction helpers =====

impl App {
    fn timewarp_metrics(&self) -> super::timewarp::TimewarpMetrics {
        let height = self.size.1 as f32;
        let width = self.size.0 as f32;
        let layout = self.layout(width, height);
        let tw_ruler = layout.timewarp_ruler.unwrap();
        let tw_rect = layout.timewarp.unwrap();
        super::timewarp::TimewarpMetrics {
            px_per_beat: self.timewarp_px_per_beat,
            scroll_x: self.timewarp_scroll_x,
            ruler_px: tw_ruler.y,
            ruler_h: tw_ruler.height,
            body_top: tw_rect.y,
            body_h: tw_rect.height,
            theme: self.theme.clamp(0, 6),
        }
    }

    /// Pontos de tempo atuais (espelho das mudanças do `TempoMap`).
    fn tw_points(&self) -> Vec<super::timewarp::TempoPoint> {
        self.tempo_map
            .changes
            .iter()
            .map(|c| super::timewarp::TempoPoint { beat: c.beat, bpm: c.bpm })
            .collect()
    }

    /// Aplica uma lista de pontos editada de volta ao `TempoMap` e escreve no
    /// projeto (via callback de write-back).
    fn tw_apply_points(&mut self, points: &[super::timewarp::TempoPoint]) {
        for p in points {
            self.tempo_map.set_change(p.beat, p.bpm);
        }
        self.tw_write_back();
    }

    /// Dispara o callback de write-back (host grava no projeto) e re-renderiza.
    fn tw_write_back(&mut self) {
        if let Some(cb) = &self.tempo_write_back {
            cb(&self.tempo_map);
        }
        self.request_redraw();
    }

    /// Clique na régua do TIMEWARP: inicia/finaliza o scrub da playhead.
    pub fn tw_handle_ruler_click(&mut self, state: ButtonState, pos: (f32, f32)) {
        match state {
            ButtonState::Pressed => {
                self.tw_scrubbing = true;
                self.tw_set_scrub_beat(pos.0);
                self.request_redraw();
            }
            ButtonState::Released => {
                self.tw_scrubbing = false;
                self.request_redraw();
            }
        }
    }

    /// Clique no corpo do timewarp: edição de pontos de tempo
    /// (Select = mover, Draw = adicionar, Erase = remover).
    pub fn handle_timewarp_click(&mut self, state: ButtonState, pos: (f32, f32)) {
        let m = self.timewarp_metrics();
        let tool = current_tool();

        match state {
            ButtonState::Pressed => {
                if self.splitter_hit_test(pos.1, self.size.1 as f32) {
                    self.splitter_drag = true;
                    return;
                }

                match tool {
                    Tool::Select => {
                        let points = self.tw_points();
                        let _ = self.timewarp_inter.begin_drag(&points, pos, &m);
                    }
                    Tool::Draw => {
                        let (mut beat, bpm) = m.cursor_to_beat_bpm(pos);
                        if !self.alt_pressed {
                            beat = beat.round();
                        }
                        self.tempo_map.set_change(beat, bpm);
                        self.tw_write_back();
                        return;
                    }
                    Tool::Erase => {
                        let points = self.tw_points();
                        if let Some(idx) = self.timewarp_inter.hit_test(&points, pos, &m) {
                            if let Some(p) = points.get(idx) {
                                self.tempo_map.remove_change(p.beat);
                                self.tw_write_back();
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
            ButtonState::Released => {
                if self.splitter_drag {
                    self.end_splitter_drag();
                    return;
                }
                match tool {
                    Tool::Select => {
                        if self.timewarp_inter.is_dragging() {
                            let mut points = self.tw_points();
                            self.timewarp_inter.end_drag(&mut points);
                            self.tw_apply_points(&points);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.request_redraw();
    }
}

// ===== Piano roll interaction helpers =====

impl App {
    fn piano_roll_metrics(&self) -> super::piano_roll::PianoRollMetrics {
        let height = self.size.1 as f32;
        let width = self.size.0 as f32;
        let layout = self.layout(width, height);
        let pr_ruler = layout.piano_ruler.unwrap();
        let pr_rect = layout.piano_roll.unwrap();

        let m = super::piano_roll::PianoRollMetrics {
            px_per_beat: self.piano_roll_px_per_beat,
            scroll_x: self.piano_roll_scroll_x,
            scroll_y: self.piano_roll_scroll_y,
            ruler_px: pr_ruler.y + super::piano_roll::PIANO_RULER_H,
            row_height: 6.0,
            theme: self.theme.clamp(0, 6),
            top_pitch: 96,
            bottom_pitch: 0,
            key_height: self.piano_roll_key_height,
            roll_height: pr_rect.height,
        };
        m
    }

    fn handle_piano_roll_click(&mut self, state: ButtonState, pos: (f32, f32)) {
        let m = self.piano_roll_metrics();
        let tool = current_tool();

        match state {
            ButtonState::Pressed => {
                if self.splitter_hit_test(pos.1, self.size.1 as f32) {
                    self.splitter_drag = true;
                    return;
                }

                self.piano_roll_inter.sync_selection(self.piano_roll_notes.len());

                match tool {
                    Tool::Select => {
                        if self.piano_roll_inter.is_dragging() {
                            self.piano_roll_inter.drag_move(pos, &m);
                        } else if !self.piano_roll_inter.begin_drag(&self.piano_roll_notes, pos, &m) {
                            self.piano_roll_inter.begin_marquee(pos);
                        }
                    }
                    Tool::Draw => {
                        self.piano_roll_inter.begin_draw(pos, &m);
                    }
                    _ => {}
                }
            }
            ButtonState::Released => {
                if self.splitter_drag {
                    self.end_splitter_drag();
                    return;
                }

                match tool {
                    Tool::Select => {
                        if self.piano_roll_inter.is_dragging() {
                            self.piano_roll_inter.end_drag(&mut self.piano_roll_notes, &m);
                            self.sync_piano_roll_to_core();
                        } else if self.piano_roll_inter.is_marqueeing() {
                            self.piano_roll_inter.end_marquee(&self.piano_roll_notes, &m);
                        }
                    }
                    Tool::Draw => {
                        if self.piano_roll_inter.is_drawing() {
                            let created = self.piano_roll_inter.end_draw(&mut self.piano_roll_notes, &m);
                            for &idx in &created {
                                if let Some(note) = self.piano_roll_notes.get(idx) {
                                    self.note_editor.add(note.pitch, note.start, note.length, note.velocity);
                                }
                            }
                            self.sync_piano_roll_from_core();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

