//! RuanaStudio — App egui (wgpu renderer)
//!
//! Shell egui com timeline-render EMBUTIDA no mesmo device wgpu.
//! A timeline renderiza em textura offscreen (readback) e egui mostra no painel central.

use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::{CreationContext, NativeOptions};

use daw_core::transport::Transport;
use daw_core::project::Project;
use daw_core::clip::Clip;
use wgpu::util::DeviceExt;
use pollster::block_on;

use crate::toolbar::{ToolbarAction, ToolbarState, toolbar_view, ToolbarCtl, ToolbarCallback};
use crate::timeline_embed::TimelineEmbed;

// ============================================================================
// SHELL STATE
// ============================================================================

/// Estado global do shell
pub struct ShellState {
    pub transport: Arc<Mutex<Transport>>,
    pub project: Arc<Mutex<Project>>,
    pub clips: Arc<Mutex<Vec<Clip>>>,
    pub toolbar: ToolbarState,
    pub follow_on: bool,
    pub snap_on: bool,
    pub active_tool: usize,
    // Timeline embarcada
    pub timeline_embed: Option<TimelineEmbed>,
    // Canal para actions da toolbar
    pub action_tx: std::sync::mpsc::Sender<ToolbarAction>,
    pub action_rx: std::sync::mpsc::Receiver<ToolbarAction>,
}

impl ShellState {
    pub fn new() -> Self {
        let transport = Arc::new(Mutex::new(Transport::default()));
        let project = Arc::new(Mutex::new(Project::default()));
        let (action_tx, action_rx) = std::sync::mpsc::channel::<ToolbarAction>();

        // Timeline embed será criada no RuanaApp::new com device/queue do egui
        Self {
            transport: transport.clone(),
            project: project.clone(),
            clips: Arc::new(Mutex::new(Vec::new())),
            toolbar: ToolbarState::new(),
            follow_on: true,
            snap_on: true,
            active_tool: 0,
            timeline_embed: None,
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
    device: Option<Arc<wgpu::Device>>,
    queue: Option<Arc<wgpu::Queue>>,
    timeline_initialized: bool,
}

impl RuanaApp {
    pub fn new(cc: &CreationContext, mut state: ShellState) -> Self {
        // Captura device/queue do egui_wgpu
        eprintln!("DEBUG: Checking wgpu_render_state");
        let (device, queue) = if let Some(wgpu_render_state) = cc.wgpu_render_state.as_ref() {
            eprintln!("DEBUG: Got wgpu_render_state");
            (wgpu_render_state.device.clone(), wgpu_render_state.queue.clone())
        } else {
            panic!("Wgpu render state not available - must use Renderer::Wgpu");
        };

        eprintln!("DEBUG: Got device and queue, storing for later timeline creation");
        // Configura estilo
        configure_style(&cc.egui_ctx);

        Self { 
            state,
            device: Some(Arc::new(device)),
            queue: Some(Arc::new(queue)),
            timeline_initialized: false,
        }
    }
}

impl eframe::App for RuanaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Lazy initialize timeline on first frame
        if !self.timeline_initialized {
            if let (Some(device), Some(queue)) = (self.device.take(), self.queue.take()) {
                eprintln!("DEBUG: Creating TimelineEmbed on first frame");
                let transport = self.state.transport.clone();
                let timeline_embed = TimelineEmbed::new(device, queue, transport);
                eprintln!("DEBUG: TimelineEmbed created successfully");
                self.state.timeline_embed = Some(timeline_embed);
                self.timeline_initialized = true;
            }
        }

        // Processa actions da toolbar
        while let Ok(action) = self.state.action_rx.try_recv() {
            handle_toolbar_action(&mut self.state, action);
        }

        // Sync toolbar com transport
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

        // Callback da toolbar
        let action_tx = self.state.action_tx.clone();
        let on_action: ToolbarCallback = Arc::new(move |action: ToolbarAction| {
            let _ = action_tx.send(action);
        });

        // ===== Layout principal =====
        // Toolbar no topo (usando horizontal strip)
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.set_max_height(34.0);
            toolbar_view(ui, &mut self.state.toolbar, &on_action);
        });

        ui.add_space(4.0);

        // Painel central com timeline EMBUTIDA
        egui::CentralPanel::default().show(ui, move |ui| {
            ui.set_min_size(ui.available_size());
            
            if let Some(timeline) = self.state.timeline_embed.as_mut() {
                timeline.show(ui, &ctx);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Timeline não inicializada");
                });
            }
        });
    }
}

// ============================================================================
// STYLE CONFIGURATION
// ============================================================================

const BG_DARK: egui::Color32 = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x25, 0x25, 0x25);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xE8, 0xE5);
const BTN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x77, 0x77, 0x77);
const BTN_ACTIVE: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x55);

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    style.visuals.dark_mode = true;
    style.visuals.panel_fill = BG_DARK;
    style.visuals.window_fill = BG_DARK;
    style.visuals.extreme_bg_color = BG_DARK;

    style.visuals.button_frame = false;
    style.visuals.widgets.inactive.bg_fill = PANEL_BG;
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    style.visuals.widgets.inactive.corner_radius = 3.0.into();
    style.visuals.widgets.hovered.bg_fill = BTN_HOVER;
    style.visuals.widgets.active.bg_fill = BTN_ACTIVE;

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
            // Handled in toolbar_view
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
            if let Some(timeline) = state.timeline_embed.as_ref() {
                timeline.send_command(timeline_render::WindowCommand::SetTheme(idx));
            }
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
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Ruana Studio",
        options,
        Box::new(move |cc| Ok(Box::new(RuanaApp::new(cc, state)))),
    )
}