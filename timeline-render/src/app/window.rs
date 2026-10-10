//! Laço de eventos winit: `AppHandler` e os pontos de entrada `run` (binário
//! standalone) e `run_native_window` (janela nativa embeddável pelo app).

use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use daw_core::transport::Transport;
use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::Foundation::{POINT, HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect, GetClientRect};

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
use winit::window::{WindowId};
use winit::window;

use crate::app::{DropPreview, KeyAction, App};
use crate::clips::ClipDemo;
use crate::tools::{self, current_tool, set_tool, Tool};

/// Comandos externos (thread do app → thread da janela da timeline).
/// Enviados pela janela nativa do app para espelhar clips/bpm no engine; o
/// engine desenha e apresenta direto na surface (sem readback).
#[derive(Clone, Debug)]
pub enum WindowCommand {
    /// Substitui o espelho de clips (read-only) + regenera wave bars/segments.
    SetClips(Vec<ClipDemo>),
    /// BPM do projeto (usado na conversão tempo→pixels).
    SetBpm(f32),
    /// Mapa de tempo do projeto (espelho do timewarp).
    SetTempoMap(Vec<daw_core::timeline::TempoChange>),
    /// Índice do tema (paleta).
    SetTheme(usize),
}

// Obtém a posição atual do cursor em coordenadas lógicas da janela, usando
// `GetCursorPos` (Win32) porque durante o arrasto de um arquivo o sistema
// operacional não envia eventos `CursorMoved` para a janela.
pub fn current_cursor_pos(window: &winit::window::Window) -> Option<(f32, f32)> {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) != 0 {
            // Tenta obter a posição da área cliente via Win32
            let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if let raw_window_handle::RawWindowHandle::Win32(h) = window.window_handle().ok()?.as_raw() {
                let hwnd = h.hwnd.get() as HWND;
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    // Obtém o tamanho da área cliente
                    let mut client_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    if GetClientRect(hwnd, &mut client_rect) != 0 {
                        // O client_rect tem (0,0, width, height) - relativo à área cliente
                        // O rect tem (left, top, right, bottom) - coordenadas de tela da janela
                        // A área cliente começa em (rect.left + border_left, rect.top + caption_height)
                        // Mas não temos border/caption facilmente. Use inner_position como fallback.
                    }
                }
            }
            // Fallback: inner_position (área cliente) - mais confiável para DPI
            let window_pos = window.inner_position().ok()?;
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

    #[cfg(windows)]
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
                WindowCommand::SetTempoMap(changes) => {
                    // Substitui o espelho de tempo (preservando o BPM base do
                    // change 0 quando vier vazio).
                    if changes.is_empty() {
                        app.tempo_map = daw_core::timeline::TempoMap::from_bpm(app.bpm);
                    } else {
                        app.tempo_map.changes = changes;
                        app.tempo_map.changes.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
                    }
                    app.request_redraw();
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
                attrs = attrs
                    .with_decorations(false)
                    .with_visible(false);
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
                App::new(Some(win), (inner.width, inner.height), self.transport.clone())
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
            #[cfg(windows)]
            if let (Some(dock), Some(app)) = (self.dock.clone(), &self.app) {
                if let Some(window) = &app.window {
                    if let Ok(handle) = window.window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                            spawn_dock_tracker(h.hwnd.get() as HWND, dock);
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
        if window_id != self.window_id { return; }
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
                // Redesenha continuamente enquanto a splitter da área inferior está sendo arrastada
                if (app.show_piano_roll || app.show_timewarp) && app.splitter_drag {
                    app.request_redraw();
                }
                // Redesenha continuamente enquanto houver scrub ou loop drag no
                // piano roll / scrub no timewarp (INDEPENDENTES). Evita que o
                // arrasto pare de atualizar se o cursor parar de se mover.
                if app.show_piano_roll && (app.pr_loop_drag.is_some() || app.pr_scrubbing) {
                    app.request_redraw();
                }
                if app.show_timewarp && app.tw_scrubbing {
                    app.request_redraw();
                }
                // Se um arquivo estiver sendo arrastado, atualiza a posição do preview
                // usando a posição global do cursor (já que o OS não envia CursorMoved
                // durante drag de arquivo).
if app.is_dragging_file {
                    if let Some(pos) = app
                        .window
                        .as_ref()
                        .and_then(|w| current_cursor_pos(w))
                    {
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
                // Atalhos do motor (ferramentas, clipboard, piano roll, etc.).
                // Espaço e Esc voltam como ações para o loop decidir.
                match app.handle_key(&event) {
                    KeyAction::TogglePlay => {
                        app.handle_toggle_play();
                    }
                    KeyAction::ExitWindow => event_loop.exit(),
                    KeyAction::None => {}
                }
            }
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
                ..
            } => {
                // Tenta obter a posição atual do cursor via Win32 (fallback para
                // o primeiro clique antes de qualquer CursorMoved).
                let click_pos = if let Some(ref win) = app.window {
                    current_cursor_pos(win).unwrap_or(app.last_cursor)
                } else {
                    app.last_cursor
                };
                app.last_cursor = click_pos;

                if app.toolbar_hover.is_none() {
                    tools::update_cursor();
                }
                // Inicia arrasto da splitter da área inferior (piano roll ou timewarp)
                if state == ElementState::Pressed
                    && (app.show_piano_roll || app.show_timewarp)
                    && app.splitter_hit_test(click_pos.1, app.size.1 as f32)
                {
                    app.splitter_drag = true;
                }
                if state == ElementState::Released && app.splitter_drag {
                    app.splitter_drag = false;
                }
                if button == winit::event::MouseButton::Middle {
                    let pos = Some(click_pos);
                    app.handle_mouse_button(state.into(), pos);
                } else if button == winit::event::MouseButton::Left {
                    // Edit toolbar: clique muda a ferramenta e NÃO propaga.
                    if state == ElementState::Pressed {
                        if let Some(idx) = app.toolbar_hit(
                            app.size.0 as f32,
                            app.size.1 as f32,
                            click_pos,
                        ) {
                            if let Some(tool) = crate::app::edit_toolbar::tool_from_index(idx) {
                                set_tool(tool);
                            }
                            app.request_redraw();
                            return;
                        }
                    }
                    // Shift+Left Click: cola o clipboard na posição do cursor (qualquer track)
                    // Usa shift_pressed atualizado pelo handler de teclado
                    if state == ElementState::Pressed && app.shift_pressed {
                        let pos = click_pos;
                        if pos.1 >= app.ruler_px {
                            // Converte cursor para beat e track com snap (respeita snap_enabled)
                            let (beat, track) = app.cursor_to_snapped_beat_track(pos);
                            app.paste_at_beat(beat, track);
                            app.request_redraw();
                        }
                    } else {
                        app.handle_primary_button(state.into(), click_pos);
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
                    || (app.show_timewarp
                        && (app.tw_scrubbing || app.timewarp_inter.warp_is_dragging())) {
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
                        app.drop_preview = Some(DropPreview { beat, track, len_beats: 0.0 });
                        app.is_dragging_file = true;
                        app.request_redraw();
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
                app.request_redraw();
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
                        app.request_redraw();
                    }
                }
            }
WindowEvent::Focused(focused) => {
                if !focused {
                    app.playing = false;
                }
            }
            WindowEvent::CursorEntered { .. } => {
                // Inicializa last_cursor quando o mouse entra na janela
                // (evita primeiro clique com posição (0,0) ou stale)
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
                    if (app.transport_seconds - pos).abs() > 0.00005
                        || app.playing != t.playing
                    {
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
            // Atualiza pan com botão do meio (roda) - roda a cada frame para
            // funcionar mesmo quando o SO não envia CursorMoved durante o drag.
            if app.panning {
                app.update_panning_from_mouse();
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
/// com margens. Roda numa thread própria, em loop de ~40ms, até a janela filha
/// ser destruída. A filha é registrada como janela OWNED da mãe
/// (`GWLP_HWNDPARENT`), então o Windows já a esconde/restaura junto com a
/// minimização da mãe e ela nunca fica atrás da main em z-order.
#[cfg(windows)]
fn spawn_dock_tracker(child: HWND, spec: DockSpec) {
    // `HWND` é um ponteiro cru (não `Send`); vira `usize` ANTES de mover pra
    // thread, e é convertido de volta nos calls.
    let child = child as usize;
    std::thread::Builder::new()
        .name("timeline-dock".into())
        .spawn(move || {
            use std::time::Duration;
            use windows_sys::Win32::Foundation::{RECT, HWND as RawHWND};
            use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
            use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                FindWindowW, GetClientRect, IsIconic, IsWindow, IsWindowVisible,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT, HWND_TOP,
                SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
            };

            let to_wide = |s: &str| -> Vec<u16> {
                s.encode_utf16().chain(std::iter::once(0)).collect()
            };
            let parent_title = to_wide(&spec.parent_title);
            let ptr = |v: usize| v as RawHWND;
            let mut owner_set = false;

            loop {
                if unsafe { IsWindow(ptr(child)) } == 0 {
                    break;
                }
                let parent = unsafe { FindWindowW(std::ptr::null::<u16>(), parent_title.as_ptr()) }
                    as usize;
                if parent == 0 {
                    // Mãe ainda não encontrada: esconde a filha pra não ficar
                    // uma janela solta flutuando.
                    unsafe { ShowWindow(ptr(child), SW_HIDE) };
                    std::thread::sleep(Duration::from_millis(40));
                    continue;
                }

                // Registro único como janela owned da mãe: o Windows esconde/
                // mostra a filha junto com min/max da main e garante z-order
                // sempre acima dela (a timeline nunca fica atrás do app).
                if !owner_set {
                    unsafe {
                        SetWindowLongPtrW(
                            ptr(child),
                            GWLP_HWNDPARENT,
                            parent as isize,
                        );
                    }
                    owner_set = true;
                }

                // Mãe oculta/minimizada → oculta a filha também (quando a mãe
                // voltar, o Win32 restaura owned windows e o SetWindowPos
                // abaixo re-mostra e re-posiciona).
                if unsafe { IsWindowVisible(ptr(parent)) } == 0
                    || unsafe { IsIconic(ptr(parent)) } != 0
                {
                    if unsafe { IsWindowVisible(ptr(child)) } != 0 {
                        unsafe { ShowWindow(ptr(child), SW_HIDE) };
                    }
                    std::thread::sleep(Duration::from_millis(40));
                    continue;
                }

                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                let mut origin = POINT { x: 0, y: 0 };
                unsafe {
                    GetClientRect(ptr(parent), &mut client);
                    ClientToScreen(ptr(parent), &mut origin);
                }
                let dpi = unsafe { GetDpiForWindow(ptr(parent)) };
                // Margens em px físicos (logical * dpi / 96).
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
                    unsafe {
                        SetWindowPos(
                            ptr(child),
                            HWND_TOP,
                            x,
                            y,
                            w,
                            h,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        })
        .ok();
}

