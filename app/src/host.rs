//! Host winit + wgpu (arquitetura do AGENTS.md).
//!
//! O app é dono do `EventLoop` do winit e do dispositivo wgpu da timeline
//! (`timeline_render::App`). No `resumed()`:
//!   1. cria a janela winit (frameless, maximizada);
//!   2. cria a engine wgpu com surface nesta janela;
//!   3. instala a plataforma offscreen do Slint (`SlintOverlay`, renderer
//!      Skia/wgpu-30 sobre a MESMA device/queue da engine);
//!   4. cria a `MainWindow` e o bridge `EngineBridge`;
//!   5. chama `crate::setup(...)`, que instala todo o wiring da DAW.
//!
//! A cada `RedrawRequested`:
//!   - `update_timers_and_animations()` do Slint (animations, meters, timers);
//!   - `EngineBridge::tick` — playhead, espelho de clips, `render_timeline_bg`
//!     (a cena vai DIRETA do shader para a textura `timeline_bg`, sem
//!     readback GPU→CPU→GPU), overlay da UI desenhado pelo Skia direto na
//!     textura do overlay (GPU→GPU);
//!   - `engine.render()` apresenta na surface (quad da timeline + UI por cima).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::ComponentHandle;
use timeline_render::app::{App as TimelineEngine, ButtonState};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::slint_host::SlintOverlay;
use crate::MainWindow;
use daw_core::project::Project;
use daw_core::transport::Transport;

/// Estado pequeno compartilhado entre o handler e as closures da DAW.
pub struct SharedState {
    pub scale: f32,
    pub win_pos: (f64, f64),
    pub last_cursor: (f32, f32),
    /// Modificadores atuais (Ctrl/Alt/Shift), dos `ModifiersChanged` do winit.
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Ponte da engine wgpu para o app hosted: transições de estado a cada frame.
pub struct EngineBridge {
    pub engine: TimelineEngine,
    pub overlay: SlintOverlay,
    pub transport: Arc<Mutex<Transport>>,
    pub project: Rc<RefCell<Project>>,
    pub scale: f32,
    /// Motor de áudio DAW mantido vivo pelo ciclo de vida do app. Se cair
    /// (drop), a stream fecha e o Transport nunca mais avança — o playhead
    /// da timeline trava mesmo com `playing=true`.
    pub audio_engine: Option<daw_audio::AudioEngine>,
    /// Canto superior-esquerdo da janela (px físicos). Manter atualizado no
    /// `Moved`; usado para converter a posição GLOBAL do cursor (GetCursorPos)
    /// em coordenadas da janela durante drags do SO.
    pub win_pos: (f64, f64),
    /// O projeto mudou → o espelho de clips precisa de rebuild.
    pub clips_flag: Rc<Cell<bool>>,
    /// A cena precisa de um novo frame (rendered direto para `timeline_bg`).
    pub dirty: bool,

    // Modelos da UI para sincronização reversa (clips importados pelo drag OS).
    pub waveforms: Option<Rc<RefCell<crate::waveform::WaveformCache>>>,
    pub clips_model: Option<Rc<slint::VecModel<crate::ClipData>>>,
    pub track_colors_model: Option<Rc<slint::VecModel<i32>>>,
}

impl EngineBridge {
    pub fn new(
        engine: TimelineEngine,
        overlay: SlintOverlay,
        transport: Arc<Mutex<Transport>>,
        project: Rc<RefCell<Project>>,
        scale: f32,
        win_pos: (f64, f64),
    ) -> Self {
        let clips_flag = Rc::new(Cell::new(true));
        {
            let flag = clips_flag.clone();
            project.borrow().subscribe(move |_event, _kind| {
                flag.set(true);
            });
        }
        Self {
            engine,
            overlay,
            transport,
            project,
            scale,
            audio_engine: None,
            win_pos,
            clips_flag,
            dirty: true,
            waveforms: None,
            clips_model: None,
            track_colors_model: None,
        }
    }

    /// Rotina por frame: re-espelha clips, segue o playhead, renderiza a cena
    /// da timeline DIRETO na textura `timeline_bg` (GPU→GPU) e compõe o
    /// overlay Slint. `render_surface()` apresenta o resultado no próximo
    /// passo.
    pub fn tick(&mut self, ui: &MainWindow) {
        let ls = self.scale;

        // Snap do grid vem da edit toolbar do Slint (mesmo estado que o
        // paste/clippool lê). Alimenta o engine a cada frame para os arrastos
        // de clips na timeline seguirem o botão.
        self.engine.snap_enabled = ui.get_snap_enabled();

        // A textura `timeline_bg` tem que casar EXATAMENTE com o quad na
        // surface (senão sobe de escala → borrado). O "buraco" do Slint é o
        // `canvas-image`: começa em `+left-padding`, ocupa toda a altura do
        // componente (a régua é desenhada pela propria engine no topo).
        let lp = ui.get_timeline_left_padding() * ls;
        let rx = (ui.get_timeline_canvas_abs_x() * ls + lp).max(0.0);
        let ry = ui.get_timeline_canvas_abs_y() * ls;
        let rw = (ui.get_timeline_canvas_rect_width() * ls - lp).max(0.0);
        let rh = ui.get_timeline_canvas_rect_height() * ls;
        self.engine.set_timeline_rect(rx, ry, rw, rh);

        // Tamanho físico (px) da cena: igual ao quad (1:1).
        let aw = rw.round().clamp(0.0, 8192.0) as u32;
        let ah = rh.round().clamp(0.0, 8192.0) as u32;

        // Espelha clips (projeto mudou).
        if self.clips_flag.replace(false) {
            crate::timeline_slint::sync_clips_from_project(&mut self.engine, &self.project.borrow());
            self.dirty = true;
        }

        // Playhead segue o Transport compartilhado.
        let (playing, pos) = {
            let t = self.transport.lock().unwrap();
            (t.playing, t.position_seconds() as f32)
        };
        {
            let e = &mut self.engine;
            if e.playing != playing {
                e.playing = playing;
                self.dirty = true;
            }
            if playing {
                e.update_playhead();
                self.dirty = true;
            } else if (e.transport_seconds - pos).abs() > 0.00005 {
                e.transport_seconds = pos;
                self.dirty = true;
            }
        }

        // Feedback de importação/drag na timeline wgpu: o "buraco" do canvas é
        // cobrido pelo quad da engine, então um ghost/preview desenhado pelo
        // Slint naquela área ficaria invisível (no binário standalone a timeline
        // ocupa a janela toda e o ghost aparece). Espelhamos o estado do drag do
        // Slint (`drag-active` + snap no engine) no `drop_preview` do motor.
        //
        // DURANTE o drag o preview precisa seguir o cursor REAL. No drag de
        // arquivo do SO (Explorer) o Windows NÃO emite MouseMove/CursorMoved —
        // o binário standalone resolve polando a posição global do cursor a
        // cada frame (`current_cursor_pos`); aqui fazemos o mesmo: realimenta o
        // ghost do Slint (`drag-move` → drag-x/drag-y + snap/track) e o motor
        // (`last_cursor` → `drop_preview`). Cobre também o drag do ClipPool em
        // qualquer track/região (que a TouchArea com captura não repassa para o
        // engine-input da timeline).
        let file_drag = self.engine.is_dragging_file;
        let clip_drag = ui.get_drag_active();
        if clip_drag || file_drag {
            #[cfg(windows)]
            {
                use windows_sys::Win32::Foundation::POINT;
                use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
                let mut pt = POINT { x: 0, y: 0 };
                if unsafe { GetCursorPos(&mut pt) } != 0 {
                    let wpx = pt.x as f64 - self.win_pos.0;
                    let wpy = pt.y as f64 - self.win_pos.1;
                    let ls = self.scale as f64;
                    // Ghost do Slint espera coordenadas lógicas da janela.
                    if clip_drag {
                        ui.invoke_drag_move((wpx / ls) as f32, (wpy / ls) as f32);
                    }
                    // Motor espera px físicos relativos ao quadrante da
                    // timeline (mesma origem do engine-input TouchArea).
                    let (qx, qy) = (self.engine.timeline_rect.0, self.engine.timeline_rect.1);
                    self.engine.last_cursor = (wpx as f32 - qx, wpy as f32 - qy);
                }
            }
            let (beat, track) =
                self.engine.cursor_to_snapped_beat_track(self.engine.last_cursor);
            let changed = match &self.engine.drop_preview {
                Some(p) => (p.beat - beat).abs() > 0.0005 || p.track != track,
                None => true,
            };
            if changed {
                self.engine.drop_preview =
                    Some(timeline_render::app::DropPreview { beat, track, len_beats: 0.0 });
                self.dirty = true;
            }
        } else if self.engine.drop_preview.is_some() {
            self.engine.drop_preview = None;
            self.dirty = true;
        }

        // Drag & Drop nativo do SO: processa importações pendentes da engine
        // e sincroniza de volta para o Project e o Slint (mesmo caminho do
        // binário standalone `timeline-render`).
        {
            let clips_before = self.engine.clips.len();
            self.engine.process_pending_drop_import();
            if self.engine.clips.len() != clips_before {
                let new_clips = self.engine.clips[clips_before..].to_vec();
                let bpm = self.engine.bpm;
                if let (Some(wf), Some(cm), Some(tc)) = (
                    self.waveforms.as_ref(),
                    self.clips_model.as_ref(),
                    self.track_colors_model.as_ref(),
                ) {
                    crate::timeline_slint::sync_engine_clips_to_project(
                        &new_clips,
                        bpm,
                        &self.project,
                        wf,
                        cm,
                        tc,
                        ui,
                    );
                }
                self.dirty = true;
            }
        }

        // Renderiza a cena direto na textura do quad (sem readback).
        if self.dirty {
            self.dirty = false;
            if !self.engine.render_timeline_bg(aw, ah) {
                self.dirty = true;
            }
        }

// Compõe a UI Slint por cima: o Skia desenha direto na textura do
        // overlay da engine (GPU→GPU, sem readback) e só quando há dirty.
        let (ow, oh) = self.overlay.size();
        if let Some(tex) = self.engine.ensure_overlay_texture(ow, oh) {
            self.overlay.draw(tex);
        }
    }
}

/// Instala os callbacks da engine vindos do componente `Timeline` (mesmos
/// handlers do bridge offscreen antigo, agora operando na engine da surface).
pub fn install_engine_callbacks(
    ui: &MainWindow,
    bridge: Rc<RefCell<EngineBridge>>,
) {
    // ---- PONTEIRO: DOWN ----------------------------------------------
    {
        let b = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_engine_pointer_down(move |x, y, button, ctrl, shift, alt| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let engine = &mut b.engine;
            engine.ctrl_pressed = ctrl;
            engine.shift_pressed = shift;
            engine.alt_pressed = alt;
            match button {
                0 => {
                    // Clique na régua = seek no Transport compartilhado.
                    if py <= engine.ruler_px {
                        let beat =
                            ((px + engine.scroll_x) / engine.px_per_beat.max(0.001)).max(0.0);
                        ui.invoke_seek_to(beat);
                    }
                    engine.handle_primary_button(ButtonState::Pressed, (px, py));
                }
                2 => {
                    engine.handle_mouse_button(ButtonState::Pressed, Some((px, py)));
                }
                _ => {}
            }
            b.dirty = true;
        });
    }

    // ---- PONTEIRO: MOVE ----------------------------------------------
    {
        let b = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_engine_pointer_move(move |x, y| {
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let engine = &mut b.engine;
            if engine.scrubbing {
                let beat =
                    ((px + engine.scroll_x) / engine.px_per_beat.max(0.001)).max(0.0);
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_seek_to(beat);
                }
            }
            engine.handle_cursor_moved((px, py));
            b.dirty = true;
        });
    }

    // ---- PONTEIRO: UP ------------------------------------------------
    {
        let b = bridge.clone();
        ui.on_engine_pointer_up(move |x, y, button| {
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let engine = &mut b.engine;
            match button {
                0 => {
                    engine.handle_primary_button(ButtonState::Released, (px, py));
                }
                2 => {
                    engine.handle_mouse_button(ButtonState::Released, Some((px, py)));
                }
                _ => {}
            }
            b.dirty = true;
        });
    }

    // ---- RODA / TOUCHPAD ---------------------------------------------
    {
        let b = bridge.clone();
        ui.on_engine_scroll(move |dx, dy, ctrl, _shift, alt| {
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let px = dx * scale;
            let py = dy * scale;
            let engine = &mut b.engine;
            engine.handle_scroll(px, py, ctrl, alt);
            b.dirty = true;
        });
    }

    // ---- TECLADO -----------------------------------------------------
    // Espaço = play/pause (mesma rota do botão da UI).
    {
        let b = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_engine_key_event(move |text, _ctrl, _shift, _alt| {
            if text == " " {
                let playing = b
                    .borrow()
                    .transport
                    .lock()
                    .map(|t| t.playing)
                    .unwrap_or(false);
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_play_state_changed(!playing);
                }
            }
        });
    }
}

/// Callbacks brutos do SO para o drag & drop de arquivos, montados pelo
/// wiring da DAW depois que o core/núcleos existem.
pub struct DropSink {
    pub hovered: Rc<dyn Fn(PathBuf)>,
    pub cancelled: Rc<dyn Fn()>,
    pub dropped: Rc<dyn Fn(PathBuf)>,
}

pub struct Handler {
    ui: Option<MainWindow>,
    bridge: Option<Rc<RefCell<EngineBridge>>>,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    state: Rc<RefCell<SharedState>>,
    drop_sink: Option<DropSink>,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            ui: None,
            bridge: None,
            window: None,
            window_id: None,
            state: Rc::new(RefCell::new(SharedState {
                scale: 1.0,
                win_pos: (0.0, 0.0),
                last_cursor: (0.0, 0.0),
                ctrl: false,
                alt: false,
                shift: false,
            })),
            drop_sink: None,
        }
    }

    pub fn set_drop_sink(&mut self, sink: DropSink) {
        self.drop_sink = Some(sink);
    }
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.ui.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Ruana Studio")
            .with_inner_size(LogicalSize::new(1440.0, 900.0))
            .with_decorations(false)
            .with_resizable(true);
        let window = event_loop
            .create_window(attrs)
            .expect("host: falha ao criar janela winit");
        let window_id = window.id();
        let inner = window.inner_size();
        let win = Arc::new(window);
        let scale = win.scale_factor() as f32;
        let win_pos = win
            .outer_position()
            .map(|p| (p.x as f64, p.y as f64))
            .unwrap_or((0.0, 0.0));

        {
            let mut st = self.state.borrow_mut();
            st.scale = scale;
            st.win_pos = win_pos;
        }

        // 1. Estado autoritativo + engine wgpu com surface nesta janela.
        let project = Rc::new(RefCell::new(Project::startup()));
        let transport = Arc::new(Mutex::new(Transport::default()));
        let mut engine = TimelineEngine::new(
            Some(win.clone()),
            (inner.width, inner.height),
            Some(transport.clone()),
        )
        .expect("host: timeline-render init failed");
        engine.set_hosted(true);

        // Cantos da timeline: mesmo raio (6px lógicos) dos painéis do Slint
        // (explorer/clip pool têm border-radius: 6px), escalado pelo DPR.
        engine.set_timeline_corner_radius(6.0 * scale);

        // 2. Plataforma offscreen do Slint (Skia/wgpu-30) compartilhando a
        //    device/queue da engine + componente principal.
        let overlay = SlintOverlay::new(
            engine.instance.clone(),
            engine.adapter.clone(),
            (*engine.device).clone(),
            (*engine.queue).clone(),
        )
        .expect("host: plataforma Slint (já setada?)");
        overlay.resize(inner.width, inner.height, scale);
        let ui = crate::MainWindow::new().expect("host: MainWindow init failed");

        // 3. Bridge compartilhado (engine + overlay + estado do frame).
        let bridge = Rc::new(RefCell::new(EngineBridge::new(
            engine,
            overlay,
            transport.clone(),
            project.clone(),
            scale,
            win_pos,
        )));

        // 4. Janela: ícone e maximizada (abertura profissional pedida).
        crate::icon::apply_to_window(&win);
        win.set_maximized(true);

        // 5. Wiring completo da DAW + bridge da engine + drop do SO.
        let drop_sink = crate::setup(
            &ui,
            win.clone(),
            bridge.clone(),
            project,
            transport,
            self.state.clone(),
        );
        self.set_drop_sink(drop_sink);

        self.ui = Some(ui);
        self.bridge = Some(bridge);
        self.window = Some(win);
        self.window_id = Some(window_id);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }
        let Some(bridge) = self.bridge.as_ref() else {
            return;
        };
        let Some(ui) = self.ui.as_ref() else {
            return;
        };
        let scale = self.state.borrow().scale;

        match event {
            WindowEvent::CloseRequested => {
                crate::cursors::reset_cursor_to_default().ok();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let mut b = bridge.borrow_mut();
                b.dirty = true;
                b.engine.resize((size.width, size.height));
                // A textura do overlay precisa acompanhar a surface (senão a
                // UI fica esticada/embaçada ao maximizar/redimensionar).
                b.overlay.resize(size.width, size.height, scale);
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: sf, ..
            } => {
                self.state.borrow_mut().scale = sf as f32;
                let size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or(winit::dpi::PhysicalSize::new(0, 0));
                let mut b = bridge.borrow_mut();
                b.scale = sf as f32;
                b.dirty = true;
                b.overlay.resize(size.width, size.height, sf as f32);
                b.engine.resize((size.width, size.height));
            }
            WindowEvent::Moved(position) => {
                let pos = (position.x as f64, position.y as f64);
                self.state.borrow_mut().win_pos = pos;
                bridge.borrow_mut().win_pos = pos;
            }
            WindowEvent::Focused(true) => {
                // Devolve o foco de teclado ao FocusScope global (senão
                // atalhos tipo Delete/Ctrl+Z morrem após alt-tab).
                ui.invoke_restore_keyboard_focus();
            }
            WindowEvent::RedrawRequested => {
                SlintOverlay::update_timers_and_animations();
                bridge.borrow_mut().tick(ui);
                bridge.borrow_mut().engine.render();
            }
            WindowEvent::ModifiersChanged(state) => {
                let sts = state.state();
                let mut st = self.state.borrow_mut();
                st.ctrl = sts.control_key();
                st.alt = sts.alt_key();
                st.shift = sts.shift_key();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.state.borrow_mut().last_cursor = (position.x as f32, position.y as f32);
                let logical = logical_pos(position.x, position.y, scale);
                overlay_dispatch(bridge, slint::platform::WindowEvent::PointerMoved {
                    position: logical,
                });
            }
            WindowEvent::CursorLeft { .. } => {
                overlay_dispatch(bridge, slint::platform::WindowEvent::PointerExited);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (px, py) = self.state.borrow().last_cursor;
                let slint_button = match button {
                    winit::event::MouseButton::Left => slint::platform::PointerEventButton::Left,
                    winit::event::MouseButton::Right => slint::platform::PointerEventButton::Right,
                    _ => slint::platform::PointerEventButton::Middle,
                };
                let event = match state {
                    ElementState::Pressed => slint::platform::WindowEvent::PointerPressed {
                        position: logical_pos(px as f64, py as f64, scale),
                        button: slint_button,
                    },
                    ElementState::Released => slint::platform::WindowEvent::PointerReleased {
                        position: logical_pos(px as f64, py as f64, scale),
                        button: slint_button,
                    },
                };
                overlay_dispatch(bridge, event);
            }
            WindowEvent::MouseWheel {
                delta,
                phase: _,
                ..
            } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                // Ctrl+scroll = zoom horizontal, Alt+scroll = altura das faixas:
                // rota DIRETA ao motor com os modificadores reais do winit (via
                // `ModifiersChanged`). O Slint não vê esse wheel (evita a rota
                // `engine-scroll`, que teria Ctrl/Alt sempre falso — não
                // enviamos ModifiersChanged pra ele).
                let st = self.state.borrow();
                let (ctrl, alt) = (st.ctrl, st.alt);
                drop(st);
                if ctrl || alt {
                    let mut b = bridge.borrow_mut();
                    b.engine.handle_scroll(dx, dy, ctrl, alt);
                    b.dirty = true;
                } else {
                    let (px, py) = self.state.borrow().last_cursor;
                    overlay_dispatch(
                        bridge,
                        slint::platform::WindowEvent::PointerScrolled {
                            position: logical_pos(px as f64, py as f64, scale),
                            delta_x: dx / scale,
                            delta_y: dy / scale,
                        },
                    );
                }
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                let adapter = bridge.borrow().overlay.window.clone();
                // O motor processa os mesmos atalhos do renderer standalone
                // (1-6, P, F/Ctrl+F, Ctrl+C/A/Z/Y/V/D, Delete, etc.). Espaço
                // (§`TogglePlay`) é resolvido AQUI e vai pelo MESMO caminho do
                // botão de play da UI (`play-state-changed` → playback.rs):
                // mexe no `Transport` compartilhado, dispara o clock visual e o
                // flush de áudio — o `tick` do host espelha tudo no engine.
                // Antes o espaço togglava o `engine.playing` direto, mas o tick
                // re-sincronizava o engine a partir do Transport (que ficava
                // parado) e o play nunca engatava.
                // Não vai pro Slint para não depender do foco do FocusScope
                // nem acontecer double-toggle.
                let forward_to_slint = {
                    let mut b = bridge.borrow_mut();
                    match b.engine.handle_key(&key) {
                        timeline_render::app::KeyAction::TogglePlay => {
                            let playing = b
                                .transport
                                .lock()
                                .map(|t| t.playing)
                                .unwrap_or(false);
                            ui.invoke_play_state_changed(!playing);
                            // O callback já mexeu no Transport; espelha no
                            // ícone do Slint para o botão continuar coerente.
                            let playing = b
                                .transport
                                .lock()
                                .map(|t| t.playing)
                                .unwrap_or(false);
                            ui.set_playing(playing);
                            false
                        }
                        timeline_render::app::KeyAction::ExitWindow
                        | timeline_render::app::KeyAction::None => true,
                    }
                };
                // Qualquer tecla válida marca o frame: o próximo `tick`
                // re-renderiza a cena refletindo o atalho aplicado.
                bridge.borrow_mut().dirty = true;
                if forward_to_slint {
                    forward_key(&adapter, &key);
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
                        let mut b = bridge.borrow_mut();
                        if !b.engine.drop_paths.contains(&p.to_string()) {
                            b.engine.drop_paths.push(p.to_string());
                        }
                        let (beat, track) =
                            b.engine.cursor_to_snapped_beat_track(b.engine.last_cursor);
                        b.engine.drop_path = Some(p.to_string());
                        b.engine.drop_preview =
                            Some(timeline_render::app::DropPreview { beat, track, len_beats: 0.0 });
                        b.engine.is_dragging_file = true;
                        b.dirty = true;
                    }
                }
            }
            WindowEvent::HoveredFileCancelled => {
                let mut b = bridge.borrow_mut();
                b.engine.drop_preview = None;
                b.engine.drop_path = None;
                b.engine.drop_paths.clear();
                b.engine.pending_drop_import = false;
                b.engine.is_dragging_file = false;
                b.dirty = true;
            }
            WindowEvent::DroppedFile(path) => {
                if let Some(p) = path.to_str() {
                    let ext = std::path::Path::new(p)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .unwrap_or_default();
                    if ["wav", "mp3", "flac", "ogg", "oga"].contains(&ext.as_str()) {
                        let mut b = bridge.borrow_mut();
                        if !b.engine.drop_paths.contains(&p.to_string()) {
                            b.engine.drop_paths.push(p.to_string());
                        }
                        b.engine.pending_drop_import = true;
                        b.engine.last_drop_instant = std::time::Instant::now();
                        b.engine.drop_preview = None;
                        b.engine.drop_path = None;
                        b.engine.is_dragging_file = false;
                        b.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Loop contínuo do motor: playhead, animations do Slint e timers.
        // NÃO forçamos redraw do overlay aqui: o próprio Slint pede repaint
        // quando a cena fica suja (animações, metros, hover, timers) via o
        // adapter — com a UI estática nada é re-renderizado/readback (o custo
        // de full-screen readback + premultiply só existe na hora da sujeira).
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Entrada do app: dono do `EventLoop` (winit) — NUNCA o Slint.
pub fn run() -> Result<(), slint::PlatformError> {
    let event_loop = EventLoop::new().map_err(|e| {
        slint::PlatformError::Other(format!("winit: falha ao criar event loop: {e}"))
    })?;
    let mut handler = Handler::new();
    event_loop.run_app(&mut handler).map_err(|e| {
        slint::PlatformError::Other(format!("winit: event loop terminou com erro: {e}"))
    })?;
    Ok(())
}

// ---- Input: winit → Slint (mesmo mapeamento do overlay_demo) -------------

/// Despacha um evento de janela para o overlay SEM segurar o borrow do
/// bridge: os callbacks do Slint (engine-pointer-*, engine-scroll) reentram
/// no bridge com `borrow_mut`, então o RefCell precisa estar livre.
fn overlay_dispatch(bridge: &Rc<RefCell<EngineBridge>>, event: slint::platform::WindowEvent) {
    let adapter = bridge.borrow().overlay.window.clone();
    SlintOverlay::dispatch_to(&adapter, event);
}

/// Dispatch de teclado para o Slint: mapeia `logical_key` do winit para o
/// texto de `WindowEvent::KeyPressed/Released` do Slint.
fn forward_key(adapter: &Rc<crate::slint_host::OverlayWindowAdapter>, event: &KeyEvent) {
    use slint::SharedString;
    let text: Option<SharedString> = match &event.logical_key {
        Key::Character(s) => Some(SharedString::from(s.as_str())),
        Key::Named(named) => named_key_text(*named),
        Key::Unidentified(_) | Key::Dead(_) => None,
    };
    let Some(text) = text else { return };
    let ev = match event.state {
        ElementState::Pressed => slint::platform::WindowEvent::KeyPressed { text },
        ElementState::Released => slint::platform::WindowEvent::KeyReleased { text },
    };
    SlintOverlay::dispatch_to(adapter, ev);
}

/// Converte posição em px físicos para px lógicos do Slint.
fn logical_pos(x: f64, y: f64, scale_factor: f32) -> slint::LogicalPosition {
    slint::LogicalPosition::new(x as f32 / scale_factor, y as f32 / scale_factor)
}

/// Mapeia teclas nomeadas do winit para os nomes que o Slint entende.
fn named_key_text(key: NamedKey) -> Option<slint::SharedString> {
    let name: Option<&'static str> = match key {
        NamedKey::Enter => Some("Return"),
        NamedKey::Tab => Some("Tab"),
        NamedKey::Space => Some(" "),
        NamedKey::Backspace => Some("Backspace"),
        NamedKey::Delete => Some("Delete"),
        NamedKey::Escape => Some("Escape"),
        NamedKey::ArrowUp => Some("Up"),
        NamedKey::ArrowDown => Some("Down"),
        NamedKey::ArrowLeft => Some("Left"),
        NamedKey::ArrowRight => Some("Right"),
        NamedKey::Home => Some("Home"),
        NamedKey::End => Some("End"),
        NamedKey::PageUp => Some("PageUp"),
        NamedKey::PageDown => Some("PageDown"),
        NamedKey::Shift => Some("Shift"),
        NamedKey::Control => Some("Control"),
        NamedKey::Alt => Some("Alt"),
        NamedKey::Meta => Some("Meta"),
        NamedKey::CapsLock => Some("CapsLock"),
        _ => None,
    };
    name.map(Into::into)
}