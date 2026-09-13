//! RuanaStudio — Makepad
//! Fase atual:
//! - Janela maximizada
//! - Somente Toolbar
//! - Nenhum painel adicional
//! - Nenhuma timeline
//! - Nenhum mixer
//! - Nenhum Explorer
//! - Nenhuma janela wgpu externa

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

use makepad_widgets::makepad_platform::{CxOsOp, WindowId};
use makepad_widgets::*;

use crate::toolbar::{ToolbarAction, ToolbarCtl};

use daw_core::transport::Transport;
use daw_core::Project;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    App = {{App}} {
        ui: <Root> {

            main_window = <Window> {
                window: {
                    inner_size: vec2(1366, 768),
                    title: "Ruana Studio"
                }

                pass: {
                    clear_color: #101014
                }

                body = <View> {
                    width: Fill,
                    height: Fill,

                    flow: Down,
                    spacing: 0,

                    toolbar = <Toolbar> {}
                }
            }
        }
    }
}

app_main!(App);

/// Estado principal do shell.
///
/// Nesta fase o estado da aplicação continua existindo fora da UI,
/// mas não há timeline, mixer ou engine de áudio sendo inicializados.
struct ShellState {
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,

    follow_on: bool,
    snap_on: bool,
    rec_armed: bool,
    active_tool: usize,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            transport: Arc::new(Mutex::new(Transport::default())),
            project: Rc::new(RefCell::new(Project::startup())),

            follow_on: true,
            snap_on: true,
            rec_armed: false,
            active_tool: 0,
        }
    }
}

impl ShellState {
    /// Cria o estado inicial da aplicação.
    fn build() -> Self {
        let transport = Arc::new(Mutex::new(Transport::default()));

        {
            let mut transport = transport.lock().unwrap();

            transport.bpm = 120;
            transport.ppq = 960;
        }

        let project = Rc::new(RefCell::new(Project::startup()));

        Self {
            transport,
            project,

            follow_on: true,
            snap_on: true,
            rec_armed: false,
            active_tool: 0,
        }
    }

    /// Atualiza a Toolbar com o estado atual do Transport e do Shell.
    fn refresh_status(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let (
            playing,
            position_ticks,
            bpm,
            beats_per_bar,
            ppq,
            loop_enabled,
            metronome_enabled,
        ) = {
            let transport = self.transport.lock().unwrap();

            (
                transport.playing,
                transport.position_ticks,
                transport.bpm,
                transport.beats_per_bar,
                transport.ppq,
                transport.loop_enabled,
                transport.metronome_enabled,
            )
        };

        ToolbarCtl::set_playing(
            cx,
            ui,
            playing,
        );

        ToolbarCtl::set_position(
            cx,
            ui,
            position_ticks as i64,
            ppq as i32,
            beats_per_bar as i32,
        );

        ToolbarCtl::set_bpm(
            cx,
            ui,
            bpm as f32,
        );

        ToolbarCtl::set_loop(
            cx,
            ui,
            loop_enabled,
        );

        ToolbarCtl::set_metronome(
            cx,
            ui,
            metronome_enabled,
        );

        ToolbarCtl::set_tool(
            cx,
            ui,
            self.active_tool,
        );

        ToolbarCtl::set_follow(
            cx,
            ui,
            self.follow_on,
        );

        ToolbarCtl::set_snap(
            cx,
            ui,
            self.snap_on,
        );

        ToolbarCtl::set_rec(
            cx,
            ui,
            self.rec_armed,
        );

        ui.redraw(cx);
    }

    /// Play / Pause.
    fn toggle_play(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        {
            let mut transport = self.transport.lock().unwrap();

            transport.playing = !transport.playing;
        }

        self.refresh_status(ui, cx);
    }

    /// Stop e volta o playhead para zero.
    fn stop(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        {
            let mut transport = self.transport.lock().unwrap();

            transport.playing = false;
            transport.position_ticks = 0;
        }

        self.refresh_status(ui, cx);
    }

    /// Altera o BPM.
    fn set_bpm(
        &mut self,
        bpm: f32,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        {
            let mut transport = self.transport.lock().unwrap();

            transport.bpm = bpm.round() as u32;
        }

        self.refresh_status(ui, cx);
    }

    /// Liga/desliga Follow.
    fn toggle_follow(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        self.follow_on = !self.follow_on;

        self.refresh_status(ui, cx);
    }

    /// Liga/desliga Snap.
    fn toggle_snap(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        self.snap_on = !self.snap_on;

        self.refresh_status(ui, cx);
    }

    /// Liga/desliga Record Arm.
    fn toggle_rec(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        self.rec_armed = !self.rec_armed;

        self.refresh_status(ui, cx);
    }

    /// Liga/desliga Loop.
    fn toggle_loop(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        {
            let mut transport = self.transport.lock().unwrap();

            transport.loop_enabled = !transport.loop_enabled;
        }

        self.refresh_status(ui, cx);
    }

    /// Liga/desliga metrônomo.
    fn toggle_metronome(
        &mut self,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        {
            let mut transport = self.transport.lock().unwrap();

            transport.metronome_enabled =
                !transport.metronome_enabled;
        }

        self.refresh_status(ui, cx);
    }

    /// Seleciona uma ferramenta da Toolbar.
    fn set_tool(
        &mut self,
        tool: usize,
        ui: &WidgetRef,
        cx: &mut Cx,
    ) {
        if tool >= 6 {
            return;
        }

        self.active_tool = tool;

        self.refresh_status(ui, cx);
    }
}

/// Componente raiz do aplicativo.
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

        cx.link(
            live_id!(theme),
            live_id!(theme_desktop_dark),
        );
    }
}

impl MatchEvent for App {
    fn handle_startup(
        &mut self,
        cx: &mut Cx,
    ) {
        // Inicializa somente o shell.
        self.shell = ShellState::build();

        // Maximiza a janela principal.
        cx.push_unique_platform_op(
            CxOsOp::MaximizeWindow(
                WindowId(0, 0),
            ),
        );

        // Atualização periódica da Toolbar.
        let _ = cx.start_interval(0.1);

        // Sincroniza a Toolbar imediatamente.
        self.shell.refresh_status(
            &self.ui,
            cx,
        );
    }

    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
    ) {
        let mut acts = Vec::new();

        ToolbarCtl::handle_actions(
            &self.ui,
            actions,
            &mut acts,
        );

        for act in acts {
            match act {
                ToolbarAction::TogglePlay => {
                    self.shell.toggle_play(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::Stop => {
                    self.shell.stop(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::SetBpm(bpm) => {
                    self.shell.set_bpm(
                        bpm,
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToggleFollow => {
                    self.shell.toggle_follow(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToggleSnap => {
                    self.shell.toggle_snap(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToggleRec => {
                    self.shell.toggle_rec(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToggleLoop => {
                    self.shell.toggle_loop(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToggleMetronome => {
                    self.shell.toggle_metronome(
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::ToolSelected(tool) => {
                    self.shell.set_tool(
                        tool,
                        &self.ui,
                        cx,
                    );
                }

                ToolbarAction::Menu(_) => {}

                ToolbarAction::Undo => {}

                ToolbarAction::Redo => {}

                ToolbarAction::ProjectKey(_) => {}

                ToolbarAction::ThemeSelected(_) => {}

                ToolbarAction::Minimize => {
                    cx.push_unique_platform_op(
                        CxOsOp::MinimizeWindow(
                            WindowId(0, 0),
                        ),
                    );
                }

                ToolbarAction::Maximize => {
                    cx.push_unique_platform_op(
                        CxOsOp::MaximizeWindow(
                            WindowId(0, 0),
                        ),
                    );
                }

                ToolbarAction::Close => {
                    cx.push_unique_platform_op(
                        CxOsOp::CloseWindow(
                            WindowId(0, 0),
                        ),
                    );
                }
            }
        }
    }

    fn handle_timer(
        &mut self,
        cx: &mut Cx,
        _event: &TimerEvent,
    ) {
        self.shell.refresh_status(
            &self.ui,
            cx,
        );
    }

    fn handle_key_down(
        &mut self,
        cx: &mut Cx,
        event: &KeyEvent,
    ) {
        if event.key_code == KeyCode::Space {
            self.shell.toggle_play(
                &self.ui,
                cx,
            );
        }
    }
}

impl AppMain for App {
    fn handle_event(
        &mut self,
        cx: &mut Cx,
        event: &Event,
    ) {
        self.match_event(
            cx,
            event,
        );

        self.ui.handle_event(
            cx,
            event,
            &mut Scope::empty(),
        );
    }
}