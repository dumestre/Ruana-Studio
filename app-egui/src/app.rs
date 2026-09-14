//! RuanaStudio — App egui
//!
//! Ponto de entrada da UI egui + integração com timeline-render (janela winit
//! nativa separada).
//!
//! Arquitetura:
//! - `ShellState`: estado compartilhado (transport, project, clips, follow, snap, tool, etc.)
//! - `run_app()`: cria aplicação egui/eframe, barra de ferramentas

use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::{CreationContext, NativeOptions};

use daw_core::transport::Transport;
use daw_core::project::Project;
use daw_core::clip::Clip;

use crate::toolbar::{ToolbarAction, ToolbarState, toolbar_view, ToolbarCtl, ToolbarCallback};

// ============================================================================
// SHELL STATE
// ============================================================================

/// Estado global do shell (UI + timeline thread + transport compartilhado).
pub struct ShellState {
    pub transport: Arc<Mutex<Transport>>,
    pub project: Arc<Mutex<Project>>,
    pub clips: Arc<Mutex<Vec<Clip>>>,
    pub toolbar: ToolbarState,
    pub follow_on: bool,
    pub snap_on: bool,
    pub active_tool: usize,
    // Channel para actions da toolbar
    pub action_tx: std::sync::mpsc::Sender<ToolbarAction>,
    pub action_rx: std::sync::mpsc::Receiver<ToolbarAction>,
}

impl ShellState {
    pub fn new() -> Self {
        let transport = Arc::new(Mutex::new(Transport::default()));
        let project = Arc::new(Mutex::new(Project::default()));
        let (action_tx, action_rx) = std::sync::mpsc::channel::<ToolbarAction>();

        Self {
            transport: transport.clone(),
            project: project.clone(),
            clips: Arc::new(Mutex::new(Vec::new())),
            toolbar: ToolbarState::new(),
            follow_on: true,
            snap_on: true,
            active_tool: 0,
            action_tx,
            action_rx,
        }
    }
}

// ============================================================================
// EGUI APP
// ============================================================================

pub struct RuanaApp {
    state: ShellState,
}

impl RuanaApp {
    pub fn new(cc: &CreationContext, state: ShellState) -> Self {
        // Configure egui style
        configure_style(&cc.egui_ctx);

        Self { state }
    }
}

impl eframe::App for RuanaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Process toolbar actions from channel
        while let Ok(action) = self.state.action_rx.try_recv() {
            handle_toolbar_action(&mut self.state, action);
        }

        // Sync toolbar with transport
        {
            let transport = self.state.transport.lock().unwrap();
            let rec_armed = self.state.toolbar.rec_armed;
            let active_tool = self.state.active_tool;
            let follow_on = self.state.follow_on;
            let snap_on = self.state.snap_on;
            
            ToolbarCtl::sync(
                &mut self.state.toolbar,
                &transport,
                follow_on,
                snap_on,
                rec_armed,
                active_tool,
            );
        }

        // Setup callback
        let action_tx = self.state.action_tx.clone();
        let on_action: ToolbarCallback = Arc::new(move |action: ToolbarAction| {
            let _ = action_tx.send(action);
        });

        // ===== Toolbar (fixed height at top) =====
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.set_max_height(34.0);
            toolbar_view(ui, &mut self.state.toolbar, &on_action);
        });

        ui.add_space(4.0);

        // ===== Central area =====
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.label(egui::RichText::new("Ruana Studio - egui Shell")
                .size(16.0)
                .color(egui::Color32::from_rgb(0x9A, 0x9A, 0x9A)));
        });
    }
}

// ============================================================================
// STYLE CONFIGURATION
// ============================================================================

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    // Dark theme
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
    style.visuals.window_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);

    // Button styling
    style.visuals.button_frame = false;
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x91, 0x91, 0x91);
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    style.visuals.widgets.inactive.corner_radius = 3.0.into();
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x77, 0x77, 0x77);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x55, 0x55, 0x55);

    // Spacing
    style.spacing.item_spacing = egui::Vec2::new(4.0, 4.0);
    style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);

    ctx.set_style_of(egui::Theme::Dark, style);
}

// ============================================================================
// TOOLBAR ACTION HANDLER
// ============================================================================

fn handle_toolbar_action(state: &mut ShellState, action: ToolbarAction) {
    match action {
        ToolbarAction::TogglePlay => {
            let mut transport = state.transport.lock().unwrap();
            transport.playing = !transport.playing;
        }
        ToolbarAction::Stop => {
            let mut transport = state.transport.lock().unwrap();
            transport.playing = false;
            transport.position_ticks = 0;
        }
        ToolbarAction::ToggleRec => {
            // Handled in toolbar_view directly
        }
        ToolbarAction::ToggleLoop => {
            let mut transport = state.transport.lock().unwrap();
            transport.loop_enabled = !transport.loop_enabled;
        }
        ToolbarAction::ToggleMetronome => {
            let mut transport = state.transport.lock().unwrap();
            transport.metronome_enabled = !transport.metronome_enabled;
        }
        ToolbarAction::ToggleFollow => {
            state.follow_on = !state.follow_on;
        }
        ToolbarAction::ToggleSnap => {
            state.snap_on = !state.snap_on;
        }
        ToolbarAction::ToolSelected(idx) => {
            state.active_tool = idx;
        }
        ToolbarAction::ProjectKey(idx) => {
            state.toolbar.project_key = idx;
        }
        ToolbarAction::ThemeSelected(idx) => {
            state.toolbar.theme = idx;
        }
        ToolbarAction::Undo => {}
        ToolbarAction::Redo => {}
        ToolbarAction::Minimize => {}
        ToolbarAction::Maximize => {}
        ToolbarAction::Close => {
            std::process::exit(0);
        }
        ToolbarAction::Menu(_) => {}
        ToolbarAction::SetBpm(bpm) => {
            let mut transport = state.transport.lock().unwrap();
            transport.bpm = bpm as u32;
        }
    }
}

// ============================================================================
// ENTRY POINT
// ============================================================================

pub fn run_app() -> eframe::Result<()> {
    let state = ShellState::new();
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ruana Studio")
            .with_inner_size([1366.0, 768.0])
            .with_resizable(true),
        // Use glow (OpenGL) renderer instead of wgpu
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "Ruana Studio",
        options,
        Box::new(|cc| Ok(Box::new(RuanaApp::new(cc, state)))),
    )
}