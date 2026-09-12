//! Laço de eventos winit: `AppHandler` e os pontos de entrada `run` (binário
//! standalone) e `run_native_window` (janela nativa embeddável pelo app).

use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use daw_core::transport::Transport;
use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Especificação do modo "encaixado": a janela da timeline vira uma janela
/// winit borderless que rastreia a janela principal (do app Makepad/Slint) e
/// fica visualmente dentro dela — um painel com margens abaixo do cabeçalho,
/// acima da reserva de rodapé (ex.: futura barra de status/mixer).
#[derive(Clone)]
pub struct DockSpec {
    /// Título da janela a rastrear (usado com `FindWindowW`).
    pub parent_title: String,
    /// Altura do cabeçalho (px lógicos) que fica acima da timeline.
    pub top: i32,
    /// Margem esquerda (px lógicos).
    pub left: i32,
    /// Margem direita (px lógicos).
    pub right: i32,
    /// Reserva abaixo da timeline (px lógicos; altura de uma futura barra).
    pub bottom: i32,
}
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window;
use winit::window::WindowId;

use crate::app::{App, DropPreview};
use crate::clips::ClipDemo;
use crate::tools::{self, Tool, current_tool, set_tool};

/// Comandos externos (thread do app → thread da janela da timeline).
/// Enviados pela janela nativa do app para espelhar clips/bpm no engine; o
/// engine desenha e apresenta direto na surface (sem readback).
#[derive(Clone, Debug)]
pub enum WindowCommand {
    /// Substitui o espelho de clips (read-only) + regenera wave bars/segments.
    SetClips(Vec<ClipDemo>),
    /// BPM do projeto (usado na conversão tempo→pixels).
    SetBpm(f32),
    /// Índice do tema (paleta).
    SetTheme(usize),
}

// Obtém a posição atual do cursor em coordenadas lógicas da janela, usando
// `GetCursorPos` (Win32) porque durante o arrasto de um arquivo o sistema
// operacional não envia eventos `CursorMoved` para a janela.
fn current_cursor_pos(window: &winit::window::Window) -> Option<(f32, f32)> {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) != 0 {
            let window_pos = window.outer_position().ok()?;
            let scale = window.scale_factor();
            let x = (pt.x - window_pos.x) as f32 / scale as f32;
            let y = (pt.y - window_pos.y) as f32 / scale as f32;
            Some((x, y))
        } else {
            None
        }
    }
}

// ============================================================================
// Event handler (winit 0.30)
// ============================================================================

struct AppHandler {
    app: Option<App>,
    window_id: WindowId,
    transport: Option<Arc<Mutex<Transport>>>,
    rx: Option<mpsc::Receiver<WindowCommand>>,
    dock: Option<DockSpec>,
}

impl AppHandler {
    fn new() -> Self {
        Self {
            app: None,
            window_id: WindowId::dummy(),
            transport: None,
            rx: None,
            dock: None,
        }
    }

    fn with_transport(transport: Arc<Mutex<Transport>>, rx: mpsc::Receiver<WindowCommand>) -> Self {
        Self {
            app: None,
            window_id: WindowId::dummy(),
            transport: Some(transport),
            rx: Some(rx),
            dock: None,
        }
    }

    fn with_dock(
        transport: Arc<Mutex<Transport>>,
        rx: mpsc::Receiver<WindowCommand>,
        dock: DockSpec,
    ) -> Self {
        Self {
            app: None,
            window_id: WindowId::dummy(),
            transport: Some(transport),
            rx: Some(rx),
            dock: Some(dock),
        }
    }

    /// Aplica comandos externos (clips/bpm/tema) e redesenha quando mudar.
    fn drain_commands(&mut self) {
        let (Some(app), Some(rx)) = (&mut self.app, &self.rx) else {
            return;
        };
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WindowCommand::SetClips(clips) => {
                    app.clips = clips;
                    app.rebuild_wave_data();
                    app.request_redraw();
                }
                WindowCommand::SetBpm(bpm) => {
                    app.bpm = bpm;
                }
                WindowCommand::SetTheme(theme) => {
                    app.theme = theme;
                    app.request_redraw();
                }
            }
        }
    }
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none() {
            let title = if self.transport.is_some() {
                "RuanaStudio — Timeline"
            } else {
                "timeline-rendered — wgpu standalone"
            };
            let mut attrs = window::Window::default_attributes()
                .with_title(title)
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
            // Modo dock: sem decoração, sem taskbar, invisível até o tracker
            // encaixar e mostrar na posição certa (evita piscar na origem).
            if self.dock.is_some() {
                attrs = attrs.with_decorations(false).with_visible(false);
                #[cfg(windows)]
                {
                    use winit::platform::windows::WindowAttributesExtWindows;
                    attrs = attrs.with_skip_taskbar(true);
                }
            }
            let window = event_loop
                .create_window(attrs)
                .expect("failed to create window");
            self.window_id = window.id();
            let inner = window.inner_size();
            let win: Arc<winit::window::Window> = Arc::new(window);
            self.app = Some(
                App::new(
                    Some(win),
                    (inner.width, inner.height),
                    self.transport.clone(),
                )
                .expect("app init failed"),
            );
            // Initialize tools first (creates the ToolManager singleton),
            // THEN set the HWND so the ToolManager can use SetClassLongPtrW.
            if let Some(app) = &self.app {
                app.init_tools();
                app.request_redraw();
            }
            // Get HWND for Win32 cursor class override (MUST be after init_tools)
            if let Some(app) = &self.app {
                if let Some(window) = &app.window {
                    if let Ok(handle) = window.window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                            crate::tools::set_hwnd(h.hwnd.get() as HWND);
                        }
                    }
                }
            }
            // Modo dock: thread que mantém a janela encaixada na janela mãe.
            if let (Some(dock), Some(app)) = (self.dock.clone(), &self.app) {
                if let Some(window) = &app.window {
                    if let Ok(handle) = window.window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                            dock::spawn_dock_tracker(h.hwnd.get() as HWND, dock);
                        }
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
        if window_id != self.window_id {
            return;
        }
        let app = self.app.as_mut().expect("app not initialized");
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                app.resize((size.width, size.height));
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
                    app.request_redraw();
                }
                // Redesenha continuamente enquanto houver scrub ou loop drag no
                // piano roll (INDEPENDENTES). Evita que o arrasto pare de atualizar
                // se o cursor parar de se mover (por exemplo, durante um snap).
                if app.show_piano_roll && (app.pr_loop_drag.is_some() || app.pr_scrubbing) {
                    app.request_redraw();
                }
                // Se um arquivo estiver sendo arrastado, atualiza a posição do preview
                // usando a posição global do cursor (já que o OS não envia CursorMoved
                // durante drag de arquivo).
                if app.is_dragging_file {
                    if let Some(pos) = app.window.as_ref().and_then(|w| current_cursor_pos(w)) {
                        app.last_cursor = pos;
                        let (beat, track) = app.cursor_to_snapped_beat_track(pos);
                        if let Some(preview) = app.drop_preview.as_mut() {
                            preview.beat = beat;
                            preview.track = track;
                        }
                    }
                    app.request_redraw();
                }
                app.render();
                // Continua o loop de redraw enquanto o drag está ativo e há
                // scroll ativo (para rolar continuamente na borda)
                if app.clip_interaction.is_dragging()
                    && (app.edge_scroll_x > 0.0 || app.edge_scroll_y > 0.0)
                {
                    app.request_redraw();
                }
            }
            WindowEvent::MouseWheel {
                device_id: _,
                delta,
                phase: _,
            } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    winit::event::MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                app.handle_scroll(dx, dy, app.ctrl_pressed, app.alt_pressed);
                app.request_redraw();
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
                        #[cfg(feature = "audio")]
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
                    app.request_redraw();
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
                    app.request_redraw();
                }
                // F: foca o clip sob o cursor; Ctrl+F: toggle follow playhead.
                if pressed && matches!(vk, Key::Character(ref s) if s.eq_ignore_ascii_case("f")) {
                    if app.ctrl_pressed {
                        app.follow_playhead = !app.follow_playhead;
                    } else {
                        app.toggle_clip_focus(app.size.0 as f32);
                    }
                    app.request_redraw();
                }
                // Tool switching keys (1-6)
                if pressed && !app.ctrl_pressed && !app.alt_pressed {
                    match vk {
                        Key::Character(ref s) if s == "1" => {
                            set_tool(Tool::Select);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s == "2" => {
                            set_tool(Tool::Draw);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s == "3" => {
                            set_tool(Tool::Stretch);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s == "4" => {
                            set_tool(Tool::Slice);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s == "5" => {
                            set_tool(Tool::Erase);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s == "6" => {
                            set_tool(Tool::Glue);
                            app.request_redraw();
                        }
                        _ => {}
                    }
                }
                // Clipboard/editing shortcuts (Ctrl+C, Ctrl+V, Ctrl+D, Ctrl+A,
                // Ctrl+Z, Ctrl+Y, Delete, Backspace)
                if pressed && app.ctrl_pressed {
                    match vk {
                        Key::Character(ref s) if s.eq_ignore_ascii_case("c") => {
                            app.copy_selected();
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("a") => {
                            app.select_all();
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("z") => {
                            if app.shift_pressed {
                                app.redo();
                            } else {
                                app.undo();
                            }
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("y") => {
                            app.redo();
                            app.request_redraw();
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
                            let target_track = app
                                .clip_interaction
                                .selected
                                .iter()
                                .enumerate()
                                .find(|&(_, &s)| s)
                                .and_then(|(i, _)| app.clips.get(i))
                                .map(|c| c.track_index)
                                .unwrap_or(0);
                            app.paste_at_beat(target_beat, target_track);
                            app.request_redraw();
                        }
                        Key::Character(ref s) if s.eq_ignore_ascii_case("d") => {
                            app.duplicate_selected();
                            app.request_redraw();
                        }
                        _ => {}
                    }
                }
                if pressed
                    && matches!(
                        vk,
                        Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)
                    )
                {
                    app.delete_selected();
                    app.request_redraw();
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
                if state == ElementState::Pressed
                    && app.show_piano_roll
                    && app.splitter_hit_test(app.last_cursor.1, app.size.1 as f32)
                {
                    app.splitter_drag = true;
                }
                if state == ElementState::Released && app.splitter_drag {
                    app.splitter_drag = false;
                }
                if button == winit::event::MouseButton::Middle {
                    let pos = Some(app.last_cursor);
                    app.handle_mouse_button(state.into(), pos);
                } else if button == winit::event::MouseButton::Left {
                    // Edit toolbar: clique muda a ferramenta e NÃO propaga.
                    if state == ElementState::Pressed {
                        if let Some(idx) =
                            app.toolbar_hit(app.size.0 as f32, app.size.1 as f32, app.last_cursor)
                        {
                            if let Some(tool) = crate::app::edit_toolbar::tool_from_index(idx) {
                                set_tool(tool);
                            }
                            app.request_redraw();
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
                            app.request_redraw();
                        } else {
                            app.handle_primary_button(state.into(), app.last_cursor);
                        }
                    } else {
                        app.handle_primary_button(state.into(), app.last_cursor);
                    }
                }
                app.request_redraw();
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
                        app.size.0 as f32,
                        app.size.1 as f32,
                        (position.x as f32, position.y as f32),
                    );
                    if hit != app.toolbar_hover {
                        app.toolbar_hover = hit;
                        app.request_redraw();
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
                    || app.pr_scrubbing
                    {
                        app.request_redraw();
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
                        app.drop_preview = Some(DropPreview {
                            beat,
                            track,
                            len_beats: 0.0,
                        });
                        app.is_dragging_file = true;
                        app.request_redraw();
                    }
                }
            }
            WindowEvent::HoveredFileCancelled => {
                // Limpa preview se o usuário abandona o arrasto
                app.drop_preview = None;
                app.drop_path = None;
                app.drop_paths.clear();
                app.pending_drop_import = false;
                app.is_dragging_file = false;
                app.request_redraw();
            }
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
                        app.request_redraw();
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
        // Comandos vindos do app (espelho de clips/bpm/tema) têm prioridade.
        self.drain_commands();
        if let Some(app) = &mut self.app {
            // Em modo janela nativa (transport compartilhado com o app), segue
            // o estado/posição do transporte mesmo sem interação local — o
            // playhead pode ser movido pelo clock do app ou outro thread.
            if let Some(transport) = &self.transport {
                if let Ok(t) = transport.lock() {
                    let pos = t.position_seconds() as f32;
                    if (app.transport_seconds - pos).abs() > 0.00005 || app.playing != t.playing {
                        app.transport_seconds = pos;
                        app.playing = t.playing;
                        app.request_redraw();
                    }
                }
            }
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
                app.request_redraw();
            }
        }
    }
}

// ============================================================================
// Entry points
// ============================================================================

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut handler = AppHandler::new();
    event_loop.run_app(&mut handler)?;
    Ok(())
}

/// Janela nativa da timeline (embeddável pelo app).
///
/// Cria a `EventLoop`/window AQUI (na thread atual), abre a janela com o
/// engine apresentando direto na surface wgpu (sem readback) e roda até a
/// janela fechar. Comandos externos (espelho de clips/bpm) chegam por `rx`
/// e são aplicados no `about_to_wait`.
pub fn run_native_window(
    transport: Arc<Mutex<Transport>>,
    rx: mpsc::Receiver<WindowCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Criada numa thread dedicada (não é a main) — winit 0.30 exige o nome
    // disso explicitamente no Windows, senão o event loop panica.
    let mut builder = EventLoop::builder();
    #[cfg(windows)]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let event_loop = builder.build()?;
    let mut handler = AppHandler::with_transport(transport, rx);
    event_loop.run_app(&mut handler)?;
    Ok(())
}

/// Timeline embutida na janela principal (Windows): janela winit borderless
/// que rastreia a janela `parent_title` e se mantém encaixada abaixo do
/// cabeçalho (`spec.top`), com margens. Fora do Windows, cai para
/// `run_native_window` (janela separada).
#[cfg(windows)]
pub fn run_docked_window(
    transport: Arc<Mutex<Transport>>,
    rx: mpsc::Receiver<WindowCommand>,
    spec: DockSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = EventLoop::builder();
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let event_loop = builder.build()?;
    let mut handler = AppHandler::with_dock(transport, rx, spec);
    event_loop.run_app(&mut handler)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn run_docked_window(
    transport: Arc<Mutex<Transport>>,
    rx: mpsc::Receiver<WindowCommand>,
    _spec: DockSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    run_native_window(transport, rx)
}

/// Mantém a janela `child` encaixada na janela mãe (por título), como painel
/// com margens. Roda numa thread própria. A filha é registrada como janela
/// OWNED da mãe (`GWLP_HWNDPARENT`): o Windows a esconde/restaura junto com o
/// minimize da main e ela nunca fica atrás do app em z-order. Para não haver
/// "arrasto" a mais que a main se move/redimensiona, um hook `WH_CALLWNDPROC`
/// é instalado NA THREAD da janela mãe e reposiciona a filha sincronamente em
/// `WM_MOVE`/`WM_SIZE`/`WM_WINDOWPOSCHANGED`; o loop de ~150ms só fica como
/// fallback (DPI/monitor, estado inicial).
#[cfg(windows)]
mod dock {
    use std::sync::Mutex;

    use super::{DockSpec, HWND, POINT};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CWPSTRUCT, CallNextHookEx, FindWindowW, GWLP_HWNDPARENT, GetClientRect, GetWindowRect,
        GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindow, IsWindowVisible, SW_HIDE,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW,
        ShowWindow, UnhookWindowsHookEx, WH_CALLWNDPROC, WM_MOVE, WM_SIZE, WM_WINDOWPOSCHANGED,
    };

    struct HookState {
        child: usize,
        parent: usize,
        spec: DockSpec,
        hook: usize,
    }

    static HOOK_STATE: Mutex<Option<HookState>> = Mutex::new(None);

    /// Geometria da filha encaixada a partir do client area da mãe; `None`
    /// quando a mãe está oculta/minimizada ou não deu pra encaixar.
    fn dock_layout(parent: usize, spec: &DockSpec) -> Option<(i32, i32, i32, i32)> {
        let parent = parent as HWND;
        if unsafe { IsWindowVisible(parent) } == 0 || unsafe { IsIconic(parent) } != 0 {
            return None;
        }
        let mut client = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut origin = POINT { x: 0, y: 0 };
        unsafe {
            GetClientRect(parent, &mut client);
            ClientToScreen(parent, &mut origin);
        }
        let dpi = unsafe { GetDpiForWindow(parent) };
        let top = (spec.top as i32 * dpi as i32 / 96).max(0);
        let left = (spec.left as i32 * dpi as i32 / 96).max(0);
        let right = (spec.right as i32 * dpi as i32 / 96).max(0);
        let bottom = (spec.bottom as i32 * dpi as i32 / 96).max(0);
        let cw = client.right - client.left;
        let ch = client.bottom - client.top;
        let x = origin.x + left;
        let y = origin.y + top;
        let w = cw.saturating_sub(left + right);
        let h = ch.saturating_sub(top + bottom);
        if w > 0 && h > 0 {
            Some((x, y, w, h))
        } else {
            None
        }
    }

    /// Reposiciona/mostra/esconde a filha conforme a mãe.
    fn dock_apply(child: usize, parent: usize, spec: &DockSpec) {
        let child = child as HWND;
        match dock_layout(parent, spec) {
            None => {
                if unsafe { IsWindowVisible(child) } != 0 {
                    unsafe { ShowWindow(child, SW_HIDE) };
                }
            }
            Some((x, y, w, h)) => {
                let mut r = windows_sys::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(child, &mut r) };
                let same =
                    x == r.left && y == r.top && w == r.right - r.left && h == r.bottom - r.top;
                if !same || unsafe { IsWindowVisible(child) } == 0 {
                    unsafe {
                        SetWindowPos(child, HWND_TOP, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    }
                }
            }
        }
    }

    /// Hook em `WH_CALLWNDPROC` na thread da janela mãe: reposiciona a filha no
    /// mesmo ciclo de mensagem em que a main move/redimensiona — sem polling.
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let cw = lparam as *const CWPSTRUCT;
            let msg = unsafe { (*cw).message };
            let hwnd = unsafe { (*cw).hwnd as usize };
            let snapshot = HOOK_STATE.lock().ok().and_then(|g| {
                let st = g.as_ref()?;
                let active = st.hook != 0;
                if active
                    && hwnd == st.parent
                    && (msg == WM_MOVE || msg == WM_SIZE || msg == WM_WINDOWPOSCHANGED)
                {
                    Some((st.child, st.parent, st.spec.clone()))
                } else {
                    None
                }
            });
            if let Some((child, parent, spec)) = snapshot {
                dock_apply(child, parent, &spec);
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    pub fn spawn_dock_tracker(child: HWND, spec: DockSpec) {
        // `HWND` é um ponteiro cru (não `Send`); vira `usize` ANTES de mover pra
        // thread, e é convertido de volta nos calls.
        let child = child as usize;
        std::thread::Builder::new()
            .name("timeline-dock".into())
            .spawn(move || {
                let to_wide =
                    |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
                let parent_title = to_wide(&spec.parent_title);
                let ptr = |v: usize| v as HWND;

                // Espera a mãe (pode não existir ainda no 1º frame do app).
                let parent = loop {
                    if unsafe { IsWindow(ptr(child)) } == 0 {
                        return;
                    }
                    let p = unsafe { FindWindowW(std::ptr::null::<u16>(), parent_title.as_ptr()) }
                        as usize;
                    if p != 0 {
                        break p;
                    }
                    unsafe { ShowWindow(ptr(child), SW_HIDE) };
                    std::thread::sleep(std::time::Duration::from_millis(40));
                };

                // Owned window: minimize/restore + z-order controlados pela main.
                unsafe {
                    SetWindowLongPtrW(ptr(child), GWLP_HWNDPARENT, parent as isize);
                }
                let thread_id =
                    unsafe { GetWindowThreadProcessId(ptr(parent), std::ptr::null_mut()) };
                let hook = unsafe {
                    SetWindowsHookExW(
                        WH_CALLWNDPROC,
                        Some(hook_proc),
                        std::ptr::null_mut(),
                        thread_id,
                    )
                };
                let hook_usize = hook as usize;
                if let Ok(mut g) = HOOK_STATE.lock() {
                    *g = Some(HookState {
                        child,
                        parent,
                        spec: spec.clone(),
                        hook: hook_usize,
                    });
                }

                // Fallback (~150ms): reposiciona/apresenta no 1º ciclo, corrige
                // DPI/monitor e garante limpeza quando a filha é destruída.
                loop {
                    if unsafe { IsWindow(ptr(child)) } == 0 {
                        if hook_usize != 0 {
                            unsafe { UnhookWindowsHookEx(hook_usize as *mut core::ffi::c_void) };
                        }
                        if let Ok(mut g) = HOOK_STATE.lock() {
                            *g = None;
                        }
                        return;
                    }
                    dock_apply(child, parent, &spec);
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
fn spawn_dock_tracker(_child: HWND, _spec: DockSpec) {}
