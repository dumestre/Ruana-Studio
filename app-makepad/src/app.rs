//! Aplicativo RuanaStudio em Makepad — fase 1 (shell + transport + timeline).
//!
//! - Janela Makepad: barra de transport (play/pause, stop, BPM, relógio).
//! - Timeline real: `timeline-render` numa janela winit/wgpu borderless
//!   ENCAIXADA na janela Makepad (rastreia a janela mãe e cobre a área da
//!   timeline), com o `Transport` compartilhado (Arc<Mutex>) e clips
//!   espelhados via `WindowCommand::SetClips/SetBpm` (read-only nesta fase).
//! - Estado autoritativo fora da UI: `Project` + `Transport` + `AudioEngine`.
//!
//! A caption bar padrão do Makepad/winit fica OCULTA: a toolbar (`toolbar.rs`,
//! port de `ui/toolbar.slint`) ocupa o topo e o espaço vazio dela arrasta a
//! janela frameless (mesmo comando `window-drag-*` do Slint, com threshold).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use makepad_widgets::makepad_platform::{CxOsOp, WindowId};
use makepad_widgets::*;

use crate::toolbar::{ToolbarAction, ToolbarCtl};
use daw_core::transport::Transport;
use daw_core::Project;
use timeline_render::clips::ClipDemo;
use timeline_render::{DockSpec, WindowCommand};

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { inner_size: vec2(1100, 640), title: "RuanaStudio" }
                pass: { clear_color: #191919 }

                body = <View> {
                    width: Fill, height: Fill,
                    flow: Down,
                    spacing: 0,

                    toolbar = <Toolbar> {
                        width: Fill, height: 34,
                    }

                    // Linha principal: explorer (futuro) à esquerda + timeline no
                    // centro. A janela nativa da timeline cobre apenas a região
                    // `timeline_area`, com margem de 10px nas laterais — o resto
                    // da janela fica reservado para os outros painéis (estilo DAW).
                    main_row = <View> {
                        width: Fill, height: Fill,
                        flow: Right,
                        spacing: 0,

                        explorer_area = <View> {
                            width: 190, height: Fill,
                            show_bg: true,
                            draw_bg: { color: #1E1E21 },
                            padding: { left: 8, right: 8, top: 8 },
                            flow: Down,
                            explorer_label = <Label> {
                                text: "EXPLORER (próxima fase)",
                                draw_text: { color: #666, text_style: { font_size: 10 } },
                            }
                        }

                        timeline_area = <View> {
                            width: Fill, height: Fill,
                            show_bg: true,
                            draw_bg: { color: #101014 },
                        }
                    }

                    // Reserva de espaço para o mixer (próxima fase). A timeline
                    // encaixada termina exatamente acima desta barra.
                    mixer_area = <View> {
                        width: Fill, height: 200,
                        show_bg: true,
                        draw_bg: { color: #18181B },
                        align: { x: 0.5, y: 0.5 },
                        mixer_label = <Label> {
                            text: "MIXER (próxima fase)",
                            draw_text: { color: #666, text_style: { font_size: 11 } },
                        }
                    }

                    status_bar = <View> {
                        width: Fill, height: 22,
                        align: { x: 0.5, y: 0.5 },
                        status_label = <Label> {
                            text: "RuanaStudio · timeline wgpu encaixada",
                            draw_text: {
                                color: #555,
                                text_style: { font_size: 11 },
                            },
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

/// Estado de um arrasto de janela iniciado na toolbar (equivalente ao
/// `window-drag-*` do `toolbar.slint`, com dead-zone de 4 px antes de mover).
struct WindowDrag {
    armed: bool,
    abs_start: DVec2,
    win_start: DVec2,
}

/// Estado do shell (fora do `#[live]`, vive no App e sobrevive à UI).
/// `Default` é inerte; o wiring real acontece em `ShellState::build` (Startup).
struct ShellState {
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
    _audio_engine: Option<daw_audio::AudioEngine>,
    timeline_tx: Option<mpsc::Sender<WindowCommand>>,
    follow_on: bool,
    snap_on: bool,
rec_armed: bool,
    active_tool: usize,
    window_drag: Option<WindowDrag>,
    dbg_logged: bool,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            transport: Arc::new(Mutex::new(Transport::default())),
            project: Rc::new(RefCell::new(Project::startup())),
            _audio_engine: None,
            timeline_tx: None,
            follow_on: true,
            snap_on: true,
rec_armed: false,
            active_tool: 0,
            window_drag: None,
            dbg_logged: false,
        }
    }
}

impl ShellState {
    /// Sobe o estado autoritativo + timeline nativa. Chamado no Startup.
    fn build() -> Self {
        let transport = Arc::new(Mutex::new(Transport::default()));
        {
            let mut t = transport.lock().unwrap();
            t.bpm = 120;
            t.ppq = 960;
        }
        let project = Rc::new(RefCell::new(Project::startup()));

        // Motor de áudio: avança o playhead do Transport compartilhado mesmo
        // sem clips (a timeline lê `position_seconds` a cada frame). Se o
        // dispositivo não abrir, o shell segue em silêncio.
        let _audio_engine = match daw_audio::AudioEngine::start(transport.clone()) {
            Ok(engine) => {
                let cfg = engine.output_config();
                eprintln!("[audio] {} Hz, {} canais", cfg.sample_rate, cfg.channels);
                Some(engine)
            }
            Err(err) => {
                eprintln!("[audio] dispositivo indisponível: {err} — em silêncio");
                None
            }
        };

        // Timeline encaixada na janela Makepad (layout estilo DAW): a janela
        // winit/wgpu borderless rastreia a janela "RuanaStudio" e cobre APENAS
        // o painel central da timeline — abaixo da toolbar (34), à direita do
        // explorer (190) com margem de 10px, e acima do mixer (200) + status
        // (22). Como toolbars/painéis vizinhos têm tamanho fixo, o encaixe
        // continua correto em qualquer tamanho de janela (as margens do
        // DockSpec são relativas ao client area).
        let (tx, rx) = mpsc::channel::<WindowCommand>();
        let transport_thread = transport.clone();
        let spec = DockSpec {
            parent_title: "RuanaStudio".to_string(),
            top: 34,
            left: 190 + 10,
            right: 10,
            bottom: 200 + 22,
        };
        std::thread::Builder::new()
            .name("timeline-window".into())
            .spawn(move || {
                let _ = timeline_render::run_docked_window(transport_thread, rx, spec);
            })
            .map(|_| eprintln!("[timeline] janela encaixada iniciada"))
            .unwrap_or_else(|e| eprintln!("[timeline] falha ao abrir janela: {e}"));

        // Espelho inicial: clips demo read-only + BPM (a engine regenera as
        // barras de waveform procedurais ao receber).
        let mut clips = demo_clips();
        timeline_render::waveform::build_wave_bars_and_segments(&mut clips, 8.0);
        let _ = tx.send(WindowCommand::SetClips(clips));
        let _ = tx.send(WindowCommand::SetBpm(120.0));

        Self {
            transport,
            project,
            _audio_engine,
            timeline_tx: Some(tx),
            follow_on: true,
            snap_on: true,
rec_armed: false,
            active_tool: 0,
            window_drag: None,
            dbg_logged: false,
        }
    }

    /// Espelha o estado autoritativo no toolbar (ícones, posição, BPM).
    fn refresh_status(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let (playing, pos_ticks, bpm, beats_per_bar, ppq, loop_on, metro_on) = {
            let t = self.transport.lock().unwrap();
            (
                t.playing,
                t.position_ticks,
                t.bpm,
                t.beats_per_bar,
                t.ppq,
                t.loop_enabled,
                t.metronome_enabled,
            )
        };
        ToolbarCtl::set_playing(cx, ui, playing);
        ToolbarCtl::set_position(cx, ui, pos_ticks as i64, ppq as i32, beats_per_bar as i32);
        ToolbarCtl::set_bpm(cx, ui, bpm as f32);
        ToolbarCtl::set_loop(cx, ui, loop_on);
        ToolbarCtl::set_metronome(cx, ui, metro_on);
        ToolbarCtl::set_tool(cx, ui, self.active_tool);
        ToolbarCtl::set_follow(cx, ui, self.follow_on);
        ToolbarCtl::set_snap(cx, ui, self.snap_on);
        ToolbarCtl::set_rec(cx, ui, self.rec_armed);
        ui.redraw(cx);
    }

    fn toggle_play(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let mut t = self.transport.lock().unwrap();
        t.playing = !t.playing;
        drop(t);
        self.refresh_status(ui, cx);
    }

    fn stop(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let mut t = self.transport.lock().unwrap();
        t.playing = false;
        t.position_ticks = 0;
        drop(t);
        self.refresh_status(ui, cx);
    }

    fn set_bpm(&mut self, bpm: f32, ui: &WidgetRef, cx: &mut Cx) {
        {
            let mut t = self.transport.lock().unwrap();
            t.bpm = bpm.round() as u32;
        }
        if let Some(tx) = &self.timeline_tx {
            let _ = tx.send(WindowCommand::SetBpm(bpm));
        }
        self.refresh_status(ui, cx);
    }

    fn toggle_follow(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        self.follow_on = !self.follow_on;
        self.refresh_status(ui, cx);
    }

    fn toggle_snap(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        self.snap_on = !self.snap_on;
        self.refresh_status(ui, cx);
    }

    fn toggle_rec(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        self.rec_armed = !self.rec_armed;
        self.refresh_status(ui, cx);
    }

    fn toggle_loop(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        {
            let mut t = self.transport.lock().unwrap();
            t.loop_enabled = !t.loop_enabled;
        }
        self.refresh_status(ui, cx);
    }

    fn toggle_metronome(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        {
            let mut t = self.transport.lock().unwrap();
            t.metronome_enabled = !t.metronome_enabled;
        }
        self.refresh_status(ui, cx);
    }

    fn set_tool(&mut self, tool: usize, ui: &WidgetRef, cx: &mut Cx) {
        if tool >= 6 {
            return;
        }
        self.active_tool = tool;
        self.refresh_status(ui, cx);
    }
}

/// Clips demo read-only para a fase 1 (sem arquivo de áudio real; a engine
/// sintetiza a waveform).
fn demo_clips() -> Vec<ClipDemo> {
    const ROWS: &[(usize, f32, f32, bool, usize, &str)] = &[
        (0, 0.0, 4.0, true, 2, "Bateria A"),
        (0, 4.0, 4.0, true, 2, "Bateria B"),
        (1, 0.0, 8.0, false, 4, "Baixo"),
        (2, 0.0, 8.0, false, 3, "Piano"),
        (2, 8.0, 4.0, false, 3, "Piano BR"),
        (3, 0.0, 4.0, false, 5, "Pad"),
        (3, 4.0, 4.0, false, 1, "Guitarra"),
        (3, 8.0, 4.0, false, 5, "Pad 2"),
    ];
    ROWS.iter()
        .map(|&(track, beat, len, is_midi, color, name)| ClipDemo {
            track_index: track,
            beat,
            len: len.max(0.05),
            is_midi,
            color,
            name: Some(name.to_string()),
            seam_beat: None,
            pre_glue_len: None,
            pre_glue_color: None,
            wave: Vec::new(),
            file_path: None,
            source_start_beats: beat,
            source_length_beats: len,
            channels: if is_midi { 1 } else { 2 },
        })
        .collect()
}

/// Componente raiz do app.
#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    shell: ShellState,
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_widgets::live_design(cx);
        crate::toolbar::live_design(cx);
        cx.link(live_id!(theme), live_id!(theme_desktop_dark));
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.shell = ShellState::build();

        // Oculta a caption bar padrão do Makepad/winit (o `after_apply` do
        // widget `Window` força `visible: true` no Windows para a barra e os
        // botões min/max/close — aplicamos o contrário aqui, no runtime). A
        // toolbar ocupa o topo e o espaço vazio dela arrasta a janela.
        let win = self.ui.window(id!(main_window));
        win.view(id!(caption_bar)).set_visible(cx, false);
        win.view(id!(windows_buttons)).set_visible(cx, false);

        // Atualiza o relógio do transport a cada 100 ms.
        let _ = cx.start_interval(0.1);
        self.shell.refresh_status(&self.ui, cx);
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        let mut acts = Vec::new();
        ToolbarCtl::handle_actions(&self.ui, actions, &mut acts);
        for act in acts {
            match act {
                ToolbarAction::TogglePlay => self.shell.toggle_play(&self.ui, cx),
                ToolbarAction::Stop => self.shell.stop(&self.ui, cx),
                ToolbarAction::SetBpm(bpm) => self.shell.set_bpm(bpm, &self.ui, cx),
                ToolbarAction::ToggleFollow => self.shell.toggle_follow(&self.ui, cx),
                ToolbarAction::ToggleSnap => self.shell.toggle_snap(&self.ui, cx),
                ToolbarAction::ToggleRec => self.shell.toggle_rec(&self.ui, cx),
                ToolbarAction::ToggleLoop => self.shell.toggle_loop(&self.ui, cx),
                ToolbarAction::ToggleMetronome => self.shell.toggle_metronome(&self.ui, cx),
                ToolbarAction::ToolSelected(i) => self.shell.set_tool(i, &self.ui, cx),
                ToolbarAction::Menu(_)
                | ToolbarAction::Undo
                | ToolbarAction::Redo
                | ToolbarAction::ProjectKey(_)
                | ToolbarAction::ThemeSelected(_) => {}
                ToolbarAction::Minimize => {
                    cx.push_unique_platform_op(CxOsOp::MinimizeWindow(WindowId(0, 0)));
                }
                ToolbarAction::Maximize => {
                    cx.push_unique_platform_op(CxOsOp::MaximizeWindow(WindowId(0, 0)));
                }
                ToolbarAction::Close => {
                    cx.push_unique_platform_op(CxOsOp::CloseWindow(WindowId(0, 0)));
                }
            }
        }
    }

fn handle_timer(&mut self, cx: &mut Cx, _event: &TimerEvent) {
        if !self.shell.dbg_logged {
            self.shell.dbg_logged = true;
            eprintln!("[dbg] main window rect = {:?}", self.ui.window(id!(main_window)).area().rect(cx));
            eprintln!("[dbg] caption_bar valid = {}", self.ui.view(id!(caption_bar)).area().is_valid(cx));
            let tb = self.ui.view(id!(toolbar)).area();
            eprintln!("[dbg] toolbar valid = {}, rect = {:?}", tb.is_valid(cx), tb.rect(cx));
            for (name, widget) in [
                ("body", id!(body)),
                ("main_row", id!(main_row)),
                ("explorer_area", id!(explorer_area)),
                ("tb_logo", id!(tb_logo)),
                ("tb_title", id!(tb_title)),
                ("tb_menus", id!(tb_menus)),
                ("tb_key", id!(tb_key)),
                ("tb_theme", id!(tb_theme)),
                ("tb_transport", id!(tb_transport)),
                ("tb_tools", id!(tb_tools)),
                ("tb_dots", id!(tb_dots)),
                ("mixer_area", id!(mixer_area)),
            ] {
                let a = self.ui.view(widget).area();
                eprintln!("[dbg] {name} valid={} rect={:?}", a.is_valid(cx), a.rect(cx));
            }
            eprintln!("[dbg] body-style rect do toolbar_area = {:?}", self.ui.view(id!(timeline_area)).area().rect(cx));
        }
        self.shell.refresh_status(&self.ui, cx);
    }

    fn handle_key_down(&mut self, cx: &mut Cx, event: &KeyEvent) {
        if event.key_code == KeyCode::Space {
            self.shell.toggle_play(&self.ui, cx);
        }
    }
}

impl App {
    /// Arrasto da janela frameless a partir do ESPAÇO VAZIO da toolbar.
    ///
    /// Roda depois de `ui.handle_event`: o roteamento do Makepad entrega o
    /// `MouseDown` ao primeiro widget atingido (botões/tiles/dropdowns), então
    /// só chega aqui quem não bateu em nenhum controle; aí `e.hits` captura a
    /// área da toolbar como dona do gesto. Com threshold de 4 px (mesmo do
    /// `toolbar.slint`) a janela segue o mouse via `RepositionWindow`.
    fn handle_toolbar_drag(&mut self, cx: &mut Cx, event: &Event) {
        let toolbar_area = self.ui.view(id!(toolbar)).area();
        match event {
            Event::MouseDown(e) if e.button == MouseButton::PRIMARY => {
                if matches!(event.hits(cx, toolbar_area), Hit::FingerDown(_)) {
                    let Event::MouseDown(e) = event else { return };
                    self.shell.window_drag = Some(WindowDrag {
                        armed: false,
                        abs_start: e.abs,
                        win_start: self.ui.window(id!(main_window)).get_position(cx),
                    });
                }
            }
            Event::MouseMove(_) => {
                if let Some(drag) = &mut self.shell.window_drag {
                    if matches!(event.hits(cx, toolbar_area), Hit::FingerMove(_)) {
                        let Event::MouseMove(e) = event else { return };
                        let delta = e.abs - drag.abs_start;
                        if !drag.armed && (delta.x.abs() >= 4.0 || delta.y.abs() >= 4.0) {
                            drag.armed = true;
                        }
                        if drag.armed {
                            cx.push_unique_platform_op(CxOsOp::RepositionWindow(
                                WindowId(0, 0),
                                drag.win_start + delta,
                            ));
                        }
                    }
                }
            }
            Event::MouseUp(_) => {
                self.shell.window_drag = None;
            }
            _ => {}
        }
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        self.handle_toolbar_drag(cx, event);
    }
}
