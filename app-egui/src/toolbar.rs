
//! RuanaStudio — Toolbar egui
//!
//! Toolbar visual + controle de estado.

use std::sync::Arc;

use daw_core::transport::Transport;
use egui::{Color32, RichText, Vec2};

// ============================================================================
// COLORS
// ============================================================================

const BG: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xE8);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0x9A, 0x9A);
const BUTTON: Color32 = Color32::from_rgb(0x91, 0x91, 0x91);
const GREEN: Color32 = Color32::from_rgb(0x00, 0xC8, 0x53);
const RED: Color32 = Color32::from_rgb(0xDC, 0x14, 0x3C);
const CLOSE: Color32 = Color32::from_rgb(0xFF, 0x5F, 0x57);
const MINIMIZE: Color32 = Color32::from_rgb(0xFE, 0xBC, 0x2E);
const MAXIMIZE: Color32 = Color32::from_rgb(0x28, 0xC8, 0x40);

// ============================================================================
// ACTIONS
// ============================================================================

#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarAction {
    Menu(usize),
    Undo,
    Redo,
    ToggleFollow,
    ToggleSnap,
    ProjectKey(usize),
    ThemeSelected(usize),
    TogglePlay,
    Stop,
    ToggleRec,
    ToggleLoop,
    ToggleMetronome,
    SetBpm(f32),
    ToolSelected(usize),
    Minimize,
    Maximize,
    Close,
}

// ============================================================================
// POSITION FORMAT
// ============================================================================

pub fn fmt_position(
    ticks: i64,
    ppq: i32,
    beats_per_bar: i32,
) -> String {
    let ppq = ppq.max(1) as i64;
    let beats_per_bar = beats_per_bar.max(1) as i64;

    let ticks_per_bar = ppq * beats_per_bar;
    let ticks = ticks.max(0);

    let bar = ticks / ticks_per_bar + 1;
    let remainder = ticks % ticks_per_bar;

    let beat = remainder / ppq + 1;
    let tick = remainder % ppq;

    format!("{bar:02}.{beat:02}.{tick:03}")
}

// ============================================================================
// TOOLBAR STATE
// ============================================================================

/// Estado reativo da toolbar.
#[derive(Clone)]
pub struct ToolbarState {
    pub follow_on: bool,
    pub snap_on: bool,
    pub rec_armed: bool,
    pub active_tool: usize,
    pub playing: bool,
    pub loop_enabled: bool,
    pub metronome_enabled: bool,
    pub project_key: usize,
    pub theme: usize,
    pub bpm_text: String,
    pub position_text: String,
}

impl Default for ToolbarState {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolbarState {
    pub fn new() -> Self {
        Self {
            follow_on: true,
            snap_on: true,
            rec_armed: false,
            active_tool: 0,
            playing: false,
            loop_enabled: false,
            metronome_enabled: false,
            project_key: 0,
            theme: 0,
            bpm_text: "120".to_string(),
            position_text: "01.01.000".to_string(),
        }
    }
}

/// Callback chamado quando uma ação da toolbar ocorre.
pub type ToolbarCallback = Arc<dyn Fn(ToolbarAction) + Send + Sync>;

// ============================================================================
// TOOLBAR VIEW
// ============================================================================

/// Constrói a toolbar no egui.
pub fn toolbar_view(
    ui: &mut egui::Ui,
    state: &mut ToolbarState,
    on_action: &ToolbarCallback,
) {
    // ------------------------------------------------------------------------
    // BACKGROUND
    //
    // O fundo precisa ser desenhado ANTES dos widgets.
    // ------------------------------------------------------------------------

    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, BG);

    // ------------------------------------------------------------------------
    // SPACING
    // ------------------------------------------------------------------------

    {
        let spacing = ui.spacing_mut();
        spacing.item_spacing = Vec2::new(4.0, 2.0);
        spacing.button_padding = Vec2::new(6.0, 2.0);
    }

    // ------------------------------------------------------------------------
    // MAIN TOOLBAR
    // ------------------------------------------------------------------------

    ui.horizontal(|ui| {
        ui.style_mut().visuals.button_frame = false;

        // ====================================================================
        // LOGO
        // ====================================================================

        ui.label(
            RichText::new("RuanaStudio")
                .size(14.0)
                .color(TEXT)
                .strong(),
        );

        ui.separator();

        // ====================================================================
        // MENUS
        // ====================================================================

        let menus = [
            "File",
            "Edit",
            "New",
            "Save",
            "Save As",
            "Settings",
            "Help",
        ];

        for (idx, menu) in menus.iter().enumerate() {
            ui.push_id(("menu", idx), |ui| {
                let response = ui.button(
                    RichText::new(*menu)
                        .size(10.0)
                        .color(TEXT),
                );

                if response.clicked() {
                    on_action(ToolbarAction::Menu(idx));
                }
            });
        }

        // ====================================================================
        // UNDO / REDO
        // ====================================================================

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        if ui
            .button(RichText::new("↶").size(16.0))
            .on_hover_text("Undo")
            .clicked()
        {
            on_action(ToolbarAction::Undo);
        }

        if ui
            .button(RichText::new("↷").size(16.0))
            .on_hover_text("Redo")
            .clicked()
        {
            on_action(ToolbarAction::Redo);
        }

        // ====================================================================
        // FOLLOW / SNAP
        // ====================================================================

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        if ui
            .selectable_label(state.follow_on, "Follow")
            .on_hover_text("Follow playhead")
            .clicked()
        {
            state.follow_on = !state.follow_on;
            on_action(ToolbarAction::ToggleFollow);
        }

        if ui
            .selectable_label(state.snap_on, "Snap")
            .on_hover_text("Snap clips and edits to the grid")
            .clicked()
        {
            state.snap_on = !state.snap_on;
            on_action(ToolbarAction::ToggleSnap);
        }

        // ====================================================================
        // PROJECT KEY
        // ====================================================================

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        let project_keys = [
            "C", "C#", "D", "D#", "E", "F",
            "F#", "G", "G#", "A", "A#", "B",
        ];

        // Segurança contra índice inválido.
        if state.project_key >= project_keys.len() {
            state.project_key = 0;
        }

        let previous_key = state.project_key;

        egui::ComboBox::from_id_salt("project_key")
            .selected_text(project_keys[state.project_key])
            .width(50.0)
            .show_ui(ui, |ui| {
                for (idx, key) in project_keys.iter().enumerate() {
                    ui.selectable_value(
                        &mut state.project_key,
                        idx,
                        *key,
                    );
                }
            });

        if state.project_key != previous_key {
            on_action(ToolbarAction::ProjectKey(state.project_key));
        }

        // ====================================================================
        // THEME
        // ====================================================================

        ui.add_space(4.0);

        let themes = [
            "Studio Dark",
            "Studio Light",
            "Studio Blue",
            "Studio Green",
            "Metal",
            "Flat",
            "Paper",
        ];

        if state.theme >= themes.len() {
            state.theme = 0;
        }

        let previous_theme = state.theme;

        egui::ComboBox::from_id_salt("theme")
            .selected_text(themes[state.theme])
            .width(100.0)
            .show_ui(ui, |ui| {
                for (idx, theme) in themes.iter().enumerate() {
                    ui.selectable_value(
                        &mut state.theme,
                        idx,
                        *theme,
                    );
                }
            });

        if state.theme != previous_theme {
            on_action(ToolbarAction::ThemeSelected(state.theme));
        }

        // ====================================================================
        // TRANSPORT
        // ====================================================================

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            // ----------------------------------------------------------------
            // POSITION
            // ----------------------------------------------------------------

            ui.label(
                RichText::new(&state.position_text)
                    .size(14.0)
                    .color(GREEN)
                    .monospace(),
            );

            ui.add_space(6.0);

            // ----------------------------------------------------------------
            // PLAY / PAUSE
            // ----------------------------------------------------------------

            let play_text = if state.playing {
                "⏸"
            } else {
                "▶"
            };

            let play_color = if state.playing {
                GREEN
            } else {
                TEXT
            };

            let play_fill = if state.playing {
                Color32::from_rgb(0x00, 0x70, 0x32)
            } else {
                BUTTON
            };

            if ui
                .add(
                    egui::Button::new(
                        RichText::new(play_text)
                            .size(18.0)
                            .color(play_color),
                    )
                    .fill(play_fill)
                    .corner_radius(3.0),
                )
                .on_hover_text(if state.playing {
                    "Pause"
                } else {
                    "Play"
                })
                .clicked()
            {
                state.playing = !state.playing;
                on_action(ToolbarAction::TogglePlay);
            }

            // ----------------------------------------------------------------
            // STOP
            // ----------------------------------------------------------------

            if ui
                .add(
                    egui::Button::new(
                        RichText::new("■")
                            .size(15.0)
                            .color(TEXT),
                    )
                    .fill(BUTTON)
                    .corner_radius(3.0),
                )
                .on_hover_text("Stop")
                .clicked()
            {
                state.playing = false;
                on_action(ToolbarAction::Stop);
            }

            // ----------------------------------------------------------------
            // RECORD
            // ----------------------------------------------------------------

            let rec_color = if state.rec_armed {
                RED
            } else {
                BUTTON
            };

            if ui
                .add(
                    egui::Button::new(
                        RichText::new("●")
                            .size(15.0)
                            .color(if state.rec_armed {
                                Color32::WHITE
                            } else {
                                TEXT
                            }),
                    )
                    .fill(rec_color)
                    .corner_radius(3.0),
                )
                .on_hover_text("Record")
                .clicked()
            {
                state.rec_armed = !state.rec_armed;
                on_action(ToolbarAction::ToggleRec);
            }

            // ----------------------------------------------------------------
            // LOOP
            // ----------------------------------------------------------------

            let loop_color = if state.loop_enabled {
                GREEN
            } else {
                BUTTON
            };

            if ui
                .add(
                    egui::Button::new(
                        RichText::new("↻")
                            .size(17.0)
                            .color(TEXT),
                    )
                    .fill(loop_color)
                    .corner_radius(3.0),
                )
                .on_hover_text("Loop")
                .clicked()
            {
                state.loop_enabled = !state.loop_enabled;
                on_action(ToolbarAction::ToggleLoop);
            }

            // ----------------------------------------------------------------
            // METRONOME
            // ----------------------------------------------------------------

            let metro_color = if state.metronome_enabled {
                GREEN
            } else {
                BUTTON
            };

            if ui
                .add(
                    egui::Button::new(
                        RichText::new("♪")
                            .size(16.0)
                            .color(TEXT),
                    )
                    .fill(metro_color)
                    .corner_radius(3.0),
                )
                .on_hover_text("Metronome")
                .clicked()
            {
                state.metronome_enabled = !state.metronome_enabled;
                on_action(ToolbarAction::ToggleMetronome);
            }

            ui.add_space(6.0);

            // ----------------------------------------------------------------
            // BPM
            // ----------------------------------------------------------------

            ui.label(
                RichText::new("BPM")
                    .size(11.0)
                    .color(TEXT_DIM),
            );

            let bpm_response = ui.add(
                egui::TextEdit::singleline(&mut state.bpm_text)
                    .desired_width(48.0)
                    .font(egui::FontId::monospace(12.0)),
            );

            if bpm_response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                if let Ok(bpm) = state.bpm_text.trim().parse::<f32>() {
                    if bpm.is_finite() && bpm > 0.0 {
                        on_action(ToolbarAction::SetBpm(bpm));
                    }
                }
            }

            // ----------------------------------------------------------------
            // TIME SIGNATURE
            // ----------------------------------------------------------------

            ui.add_space(6.0);

            ui.label(
                RichText::new("4/4")
                    .size(12.0)
                    .color(TEXT_DIM),
            );
        });

        // ====================================================================
        // TOOLS
        // ====================================================================

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);

        let tools = [
            ("Select", 0usize),
            ("Slice", 1usize),
            ("Glue", 2usize),
            ("Stretch", 3usize),
            ("Draw", 4usize),
            ("Erase", 5usize),
        ];

        for (name, idx) in tools {
            let is_active = state.active_tool == idx;

            let fill = if is_active {
                RED
            } else {
                BUTTON
            };

            let text_color = if is_active {
                Color32::WHITE
            } else {
                TEXT
            };

            ui.push_id(("tool", idx), |ui| {
                let response = ui.add(
                    egui::Button::new(
                        RichText::new(name)
                            .size(10.0)
                            .color(text_color),
                    )
                    .fill(fill)
                    .corner_radius(3.0)
                    .min_size(Vec2::new(50.0, 22.0)),
                );

                if response.clicked() {
                    state.active_tool = idx;
                    on_action(ToolbarAction::ToolSelected(idx));
                }
            });
        }

        // ====================================================================
        // WINDOW CONTROLS
        // ====================================================================

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);

            // ------------------------------------------------------------
            // MINIMIZE
            // ------------------------------------------------------------

            ui.push_id("win_min", |ui| {
                let response = ui.add(
                    egui::Button::new("")
                        .fill(MINIMIZE)
                        .min_size(Vec2::new(12.0, 12.0))
                        .corner_radius(6.0),
                );

                if response.clicked() {
                    on_action(ToolbarAction::Minimize);
                }
            });

            // ------------------------------------------------------------
            // MAXIMIZE
            // ------------------------------------------------------------

            ui.push_id("win_max", |ui| {
                let response = ui.add(
                    egui::Button::new("")
                        .fill(MAXIMIZE)
                        .min_size(Vec2::new(12.0, 12.0))
                        .corner_radius(6.0),
                );

                if response.clicked() {
                    on_action(ToolbarAction::Maximize);
                }
            });

            // ------------------------------------------------------------
            // CLOSE
            // ------------------------------------------------------------

            ui.push_id("win_close", |ui| {
                let response = ui.add(
                    egui::Button::new("")
                        .fill(CLOSE)
                        .min_size(Vec2::new(12.0, 12.0))
                        .corner_radius(6.0),
                );

                if response.clicked() {
                    on_action(ToolbarAction::Close);
                }
            });
        });
    });
}

// ============================================================================
// CONTROLLER
// ============================================================================

pub struct ToolbarCtl;

impl ToolbarCtl {
    /// Sincroniza o estado visual da toolbar com o estado da aplicação.
    pub fn sync(
        state: &mut ToolbarState,
        transport: &Transport,
        follow_on: bool,
        snap_on: bool,
        rec_armed: bool,
        active_tool: usize,
    ) {
        let ppq = transport.ppq as i32;
        let beats_per_bar = transport.beats_per_bar as i32;

        let position_text = fmt_position(
            transport.position_ticks as i64,
            ppq,
            beats_per_bar,
        );

        state.playing = transport.playing;
        state.loop_enabled = transport.loop_enabled;
        state.metronome_enabled = transport.metronome_enabled;

        state.position_text = position_text;
        state.bpm_text = format!("{}", transport.bpm);

        state.follow_on = follow_on;
        state.snap_on = snap_on;
        state.rec_armed = rec_armed;
        state.active_tool = active_tool;
    }
}