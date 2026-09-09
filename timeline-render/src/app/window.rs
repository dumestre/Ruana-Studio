//! Laço de eventos winit: `AppHandler` e o ponto de entrada `run`.

use std::sync::Arc;
use std::time::Instant;

use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::Foundation::HWND;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{WindowId};
use winit::window;

use crate::app::{current_cursor_pos, App, DropPreview};
use crate::tools::{self, current_tool, set_tool, Tool};

// ============================================================================
// Event handler (winit 0.30)
// ============================================================================

struct AppHandler {
    app: Option<App>,
    window_id: WindowId,
}

impl AppHandler {
    fn new() -> Self {
        Self {
            app: None,
            window_id: WindowId::dummy(),
        }
    }
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none() {
            let window = event_loop
                .create_window(
                    window::Window::default_attributes()
                        .with_title("timeline-rendered — wgpu standalone")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
                )
                .expect("failed to create window");
            self.window_id = window.id();
            let win = Arc::new(window);
            self.app = Some(App::new(win).expect("app init failed"));
            // Initialize tools first (creates the ToolManager singleton),
            // THEN set the HWND so the ToolManager can use SetClassLongPtrW.
            if let Some(app) = &self.app {
                app.init_tools();
                app.window.request_redraw();
            }
            // Get HWND for Win32 cursor class override (MUST be after init_tools)
            if let Some(app) = &self.app {
                if let Ok(handle) = app.window.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                        crate::tools::set_hwnd(h.hwnd.get() as HWND);
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if window_id != self.window_id { return; }
        let app = self.app.as_mut().expect("app not initialized");
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                app.resize(size);
            }
            WindowEvent::RedrawRequested => {
                app.update_playhead();
                // Importa em lote quando o drag & drop terminar (período de silêncio)
                if app.pending_drop_import {
                    app.process_pending_drop_import();
                }
                // Auto-scroll contínuo nas bordas durante drag ativo (mesmo com
                // cursor parado). A detecção usa a última posição do cursor conhecida.
                if app.clip_interaction.is_dragging() {
                    app.update_edge_scroll(app.last_cursor);
                    app.apply_edge_scroll();
                }
                // Redesenha continuamente enquanto a splitter do piano roll está sendo arrastada
                if app.show_piano_roll && app.splitter_drag {
                    app.window.request_redraw();
                }
                // Redesenha continuamente enquanto houver scrub ou loop drag no
                // piano roll (INDEPENDENTES). Evita que o arrasto pare de atualizar
                // se o cursor parar de se mover (por exemplo, durante um snap).
                if app.show_piano_roll && (app.pr_loop_drag.is_some() || app.pr_scrubbing) {
                    app.window.request_redraw();
                }
                // Se um arquivo estiver sendo arrastado, atualiza a posição do preview
                // usando a posição global do cursor (já que o OS não envia CursorMoved
                // durante drag de arquivo).
                if app.is_dragging_file {
                    if let Some(pos) = current_cursor_pos(&app.window) {
                        app.last_cursor = pos;
                        let (beat, track) = app.cursor_to_snapped_beat_track(pos);
                        if let Some(preview) = app.drop_preview.as_mut() {
                            preview.beat = beat;
                            preview.track = track;
                        }
                    }
                    app.window.request_redraw();
                }
                app.render();
                // Continua o loop de redraw enquanto o drag está ativo e há
                // scroll ativo (para rolar continuamente na borda)
                if app.clip_interaction.is_dragging()
                    && (app.edge_scroll_x > 0.0 || app.edge_scroll_y > 0.0)
                {
                    app.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel {
                device_id: _,
                delta,
                phase: _,
            } => {
                app.handle_scroll(&delta, app.ctrl_pressed, app.alt_pressed);
                app.window.request_redraw();
            }
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                ..
            } => {
                let pressed = event.state == ElementState::Pressed;
                let vk = event.logical_key;
                if pressed && vk == Key::Named(NamedKey::Space) {
                    if app.playing {
                        if let Ok(mut t) = app.transport.lock() {
                            t.playing = false;
                        }
                        app.playing = false;
                        // Guarda o ponto onde o play foi interrompido para,
                        // ao retomar com espaço, pular de volta pro início do
                        // último play (comportamento de DAW comum).
                        app.last_play_start = app.play_start_seconds;
                    } else {
                        // Se houve um play anterior interrompido por espaço,
                        // o próximo espaço volta ao início daquele play.
                        app.transport_seconds = app.last_play_start;
                        // Sincroniza o projeto com o motor de áudio antes de tocar.
                        app.sync_audio_engine();
                        // Sincroniza o estado do loop no transporte compartilhado.
                        if let Ok(mut t) = app.transport.lock() {
                            t.bpm = app.bpm as u32;
                            t.loop_enabled = app.loop_enabled;
                            if app.loop_enabled {
                                t.loop_start_ticks = (app.loop_start_beat * t.ppq as f32) as u64;
                                t.loop_end_ticks = (app.loop_end_beat * t.ppq as f32) as u64;
                            }
                            // Posiciona no playhead atual.
                            let beats = app.transport_seconds * app.bpm / 60.0;
                            t.position_ticks = (beats * t.ppq as f32) as u64;
                            t.playing = true;
                        }
                        app.play_start_seconds = app.transport_seconds;
                        app.playing = true;
                    }
                    app.last_play_head_update = Instant::now();
                    app.window.request_redraw();
                }
                if pressed && vk == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
                if pressed && matches!(vk, Key::Character(ref s) if s.eq_ignore_ascii_case("p")) {
                    app.toggle_piano_roll();
                    app.toolbar_hover = None;
                    if app.show_piano_roll && app.note_editor.notes().is_empty() {
                        app.note_editor.add(60, 0.0, 1.0, 100);
                        app.note_editor.add(64, 1.0, 0.5, 80);
                        app.note_editor.add(67, 2.0, 1.5, 100);
                        app.sync_piano_roll_from_core();
                    }
                    app.window.request_redraw();
                }
                // F: foca o clip sob o cursor; Ctrl+F: toggle follow playhead.
                if pressed && matches!(vk, Key::Character(ref s) if s.eq_ignore_ascii_case("f")) {
                    if app.ctrl_pressed {
                        app.follow_playhead = !app.follow_playhead;
                    } else {
                        app.toggle_clip_focus(app.size.width as f32);
                    }
                    app.window.request_redraw();
                }
                // Tool switching keys (1-6)
                if pressed && !app.ctrl_pressed && !app.alt_pressed {
                    match vk {
                        Key::Character(ref s) if s == "1" => { set_tool(Tool::Select); app.window.request_redraw(); }
                        Key::Character(ref s) if s == "2" => { set_tool(Tool::Draw); app.window.request_redraw(); }
                        Key::Character(ref s) if s == "3" => { set_tool(Tool::Stretch); app.window.request_redraw(); }
                        Key::Character(ref s) if s == "4" => { set_tool(Tool::Slice); app.window.request_redraw(); }
                        Key::Character(ref s) if s == "5" => { set_tool(Tool::Erase); app.window.request_redraw(); }
                        Key::Character(ref s) if s == "6" => { set_tool(Tool::Glue); app.window.request_redraw(); }
                        _ => {}
                    }
                }
                // Clipboard/editing shortcuts (Ctrl+C, Ctrl+V, Ctrl+D, Ctrl+A,
                // Ctrl+Z, Ctrl+Y, Delete, Backspace)
                if pressed && app.ctrl_pressed {
                    match vk {
                        Key::Character(ref s) if s.eq_ignore_ascii_case("c") => {
                            app.copy_selected();
                            app.window.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("a") => {
                            app.select_all();
                            app.window.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("z") => {
                            if app.shift_pressed {
                                app.redo();
                            } else {
                                app.undo();
                            }
                            app.window.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("y") => {
                            app.redo();
                            app.window.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("v") => {
                            // Cola na posição do playhead (com snap) na track do primeiro clip selecionado ou 0
                            let bpm = app.bpm;
                            let pps = app.px_per_beat * bpm / 60.0;
                            let target_beat = if pps > 0.0 {
                                app.transport_seconds * bpm / 60.0
                            } else {
                                0.0
                            };
                            // Tenta usar a track do primeiro clip selecionado
                            let target_track = app.clip_interaction.selected.iter()
                                .enumerate()
                                .find(|&(_, &s)| s)
                                .and_then(|(i, _)| app.clips.get(i))
                                .map(|c| c.track_index)
                                .unwrap_or(0);
                            app.paste_at_beat(target_beat, target_track);
                            app.window.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("d") => {
                            app.duplicate_selected();
                            app.window.request_redraw();
                        }
                        _ => {}
                    }
                }
                if pressed && matches!(vk, Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)) {
                    app.delete_selected();
                    app.window.request_redraw();
                }
                // Track ctrl/alt/shift para zoom e shortcuts — só atualiza quando a tecla É o modificador.
                let is_ctrl_key = matches!(vk, Key::Named(NamedKey::Control))
                    || matches!(event.physical_key, PhysicalKey::Code(code) if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight));
                let is_alt_key = matches!(vk, Key::Named(NamedKey::Alt))
                    || matches!(event.physical_key, PhysicalKey::Code(code) if matches!(code, KeyCode::AltLeft | KeyCode::AltRight));
                let is_shift_key = matches!(vk, Key::Named(NamedKey::Shift))
                    || matches!(event.physical_key, PhysicalKey::Code(code) if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight));
                if is_ctrl_key {
                    app.ctrl_pressed = pressed;
                }
                if is_alt_key {
                    app.alt_pressed = pressed;
                }
                if is_shift_key {
                    app.shift_pressed = pressed;
                }
            }
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
                ..
            } => {
                if app.toolbar_hover.is_none() {
                    tools::update_cursor();
                }
                // Inicia/finaliza arrasto da splitter do piano roll
                if state == ElementState::Pressed && app.show_piano_roll
                    && app.splitter_hit_test(app.last_cursor.1, app.size.height as f32)
                {
                    app.splitter_drag = true;
                }
                if state == ElementState::Released && app.splitter_drag {
                    app.splitter_drag = false;
                }
                if button == winit::event::MouseButton::Middle {
                    let pos = Some(app.last_cursor);
                    app.handle_mouse_button(state, pos);
                } else if button == winit::event::MouseButton::Left {
                    // Edit toolbar: clique muda a ferramenta e NÃO propaga.
                    if state == ElementState::Pressed {
                        if let Some(idx) = app.toolbar_hit(
                            app.size.width as f32,
                            app.size.height as f32,
                            app.last_cursor,
                        ) {
                            if let Some(tool) = crate::app::edit_toolbar::tool_from_index(idx) {
                                set_tool(tool);
                            }
                            app.window.request_redraw();
                            return;
                        }
                    }
                    // Shift+Left Click: cola o clipboard na posição do cursor (qualquer track)
                    if state == ElementState::Pressed && app.shift_pressed {
                        let pos = app.last_cursor;
                        if pos.1 >= app.ruler_px {
                            // Converte cursor para beat e track com snap
                            let beat = (pos.0 + app.scroll_x) / app.px_per_beat.max(0.001);
                            let row_height = 64.0 * app.zoom_y.max(0.001);
                            let track = ((pos.1 - app.ruler_px + app.scroll_y) / row_height)
                                .floor()
                                .max(0.0) as usize;
                            app.paste_at_beat(beat.round().max(0.0), track);
                            app.window.request_redraw();
                } else {
                    app.handle_primary_button(state, app.last_cursor);
                }
                    } else {
                        app.handle_primary_button(state, app.last_cursor);
                    }
                }
                app.window.request_redraw();
            }
            WindowEvent::CursorMoved {
                device_id: _,
                position,
                ..
            } => {
                // Ignora evento gerado pelo warp de travamento de eixo (set_cursor_position).
                if app.cursor_warp_pending {
                    app.cursor_warp_pending = false;
                } else {
                app.handle_cursor_moved((position.x as f32, position.y as f32));
                // Hit-test da edit toolbar — tem prioridade sobre o resto.
                let hit = app.toolbar_hit(
                    app.size.width as f32,
                    app.size.height as f32,
                    (position.x as f32, position.y as f32),
                );
                if hit != app.toolbar_hover {
                    app.toolbar_hover = hit;
                    app.window.request_redraw();
                }
                if app.toolbar_hover.is_some() {
                    tools::set_hand_override(true);
                }
                // Mantém o cursor da ferramenta ativa persistente (evita voltar pro
                // cursor padrão do SO a cada movimento).
                // Só chama update_cursor() se NÃO houver override ativo (seam/fade hover),
                // pois o handle_cursor_moved já fez set_cursor_override.
                if app.toolbar_hover.is_none()
                    && !app.clip_interaction.is_seam_hovered()
                    && app.fade_interaction.hovered.is_none()
                {
                    tools::update_cursor();
                }
                if app.loop_drag.is_some() || app.scrubbing || app.clip_interaction.is_dragging()
                    || app.clip_interaction.is_marqueeing() || app.clip_interaction.hover_idx.is_some()
                    || app.clip_interaction.is_drawing()
                    || app.clip_interaction.is_seam_hovered()
                    || app.fade_interaction.hovered.is_some()
                    || app.fade_interaction.is_dragging()
                    || app.drop_preview.is_some()
                    || current_tool() == Tool::Slice || current_tool() == Tool::Stretch
                    // ====== Piano roll: drag/scrub independentes ======
                    // Sem essas flags, não há request_redraw no cursor
                    // moved e a playhead/loop do PR parece "travada" (só
                    // atualiza no clique/release).
                    || app.pr_loop_drag.is_some()
                    || app.pr_scrubbing {
                    app.window.request_redraw();
                }
                }
            }
            WindowEvent::HoveredFile(path) => {
                if let Some(p) = path.to_str() {
                    let ext = std::path::Path::new(p)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .unwrap_or_default();
                    if ["wav", "mp3", "flac", "ogg", "oga"].contains(&ext.as_str()) {
                        // acumula o path para importação em lote
                        if !app.drop_paths.contains(&p.to_string()) {
                            app.drop_paths.push(p.to_string());
                        }
                        // calcula beat/track onde o cursor está
                        let (beat, track) = app.cursor_to_snapped_beat_track(app.last_cursor);
                        app.drop_path = Some(p.to_string());
                        app.drop_preview = Some(DropPreview { beat, track, len_beats: 0.0 });
                        app.is_dragging_file = true;
                        app.window.request_redraw();
                    }
                }
            },
            WindowEvent::HoveredFileCancelled => {
                // Limpa preview se o usuário abandona o arrasto
                app.drop_preview = None;
                app.drop_path = None;
                app.drop_paths.clear();
                app.pending_drop_import = false;
                app.is_dragging_file = false;
                app.window.request_redraw();
            },
            WindowEvent::DroppedFile(path) => {
                if let Some(p) = path.to_str() {
                    let ext = std::path::Path::new(p)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .unwrap_or_default();
                    if ["wav", "mp3", "flac", "ogg", "oga"].contains(&ext.as_str()) {
                        // acumula; importa em lote após o fim do arrasto
                        if !app.drop_paths.contains(&p.to_string()) {
                            app.drop_paths.push(p.to_string());
                        }
                        app.pending_drop_import = true;
                        app.last_drop_instant = Instant::now();
                        // Limpa o estado de drag de arquivo
                        app.drop_preview = None;
                        app.drop_path = None;
                        app.is_dragging_file = false;
                        app.window.request_redraw();
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                if !focused {
                    app.playing = false;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            // Reaplica o cursor da ferramenta a cada ciclo (evita reset do SO
            // após clique/solta, WM_SETCURSOR, etc.).
            // Não sobrescreve overrides de seam/fade hover.
            if app.toolbar_hover.is_none()
                && !app.clip_interaction.is_seam_hovered()
                && app.fade_interaction.hovered.is_none()
            {
                tools::update_cursor();
            }
            if app.playing || app.clip_interaction.is_drawing() {
                app.window.request_redraw();
            }
        }
    }
}

// ============================================================================
// Entry point
// ============================================================================

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut handler = AppHandler::new();
    event_loop.run_app(&mut handler)?;
    Ok(())
}