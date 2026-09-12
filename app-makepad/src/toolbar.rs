//! Toolbar em Makepad — port de `ui/toolbar.slint` para um componente
//! separado (`app-makepad/src/toolbar.rs`).
//!
//! Não é um widget Rust custom: o toolbar é reproduzido como um protótipo
//! `<Toolbar>` (uma `View` pura) registrado via `live_design!`, e a lógica de
//! interação vive em `ToolbarCtl`, que manipula os widgets por `id!()` e
//! devolve `ToolbarAction`s (mesmo padrão do shell em `app.rs`).
//!
//! Ícones: PNGs 2× (48 px) rasterizados offline (Skia) + shader tint para
//! aplicar a cor do tema em runtime — garante nitidez de antialiasing nativo.

use makepad_widgets::*;

live_design! {
    link widgets;
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    // -----------------------------------------------------------------
    // Tokens de cor — espelham `ui/theme.slint` (índice 0: Studio Dark).
    // -----------------------------------------------------------------
    pub TW_BASE       = #313131
    pub TW_SURFACE    = #242424
    pub TW_SURF_HI    = #555555
    pub TW_TEXT       = #E8E8E8
    pub TW_TEXT_DIM   = #9A9A9A
    pub TW_ACCENT     = #00C853
    pub TW_CRIMSON    = #DC143C
    pub TW_ON_ACCENT  = #333333
    pub TW_ON_CRIM    = #FFFFFF
    pub TW_DANGER     = #FF5F57
    pub TW_DANGER_H   = #FF7F78
    pub TW_DANGER_D   = #E03F38
    pub TW_WARN       = #FEBC2E
    pub TW_WARN_H     = #FFD25E
    pub TW_OK         = #28C840
    pub TW_OK_H       = #48DC5E

    // -----------------------------------------------------------------
    // Botão de ícone: fundo transparente que captura o clique.
    // -----------------------------------------------------------------
    pub TbHit = <Button> {
        width: Fill, height: Fill,
        text: "",
        draw_bg: {
            color: #0000,
            color_hover: #FFFFFF14,
            color_down: #FFFFFF24,
            color_focus: #0000,
            color_disabled: #0000,
            border_size: 0.0,
            border_radius: 0.0,
        }
        draw_text: { color: #0000 }
    }

    // -----------------------------------------------------------------
    // Tile onde o ícone fica assentado (18×18 na maioria).
    // -----------------------------------------------------------------
    pub TbTile = <View> {
        width: Fill, height: Fill,
        show_bg: true,
        draw_bg: { color: (TW_SURF_HI), border_radius: 3.0, border_size: 0.0 }
    }

    // -----------------------------------------------------------------
    // Imagem toolbar com tint — PNG 2× + shader de cor multiplicativa.
    // Herda {{Image}} para reusar carregamento de textura / image_scale.
    // -----------------------------------------------------------------
    pub TbIcon = {{Image}} {
        draw_bg: {
            instance tint: vec4
            fn pixel(self) -> vec4 {
                let c = self.get_color_scale_pan(self.image_scale, self.image_pan)
                let tinted = vec4(c.x * self.tint.x, c.y * self.tint.y, c.z * self.tint.z, c.w * self.tint.w)
                let opacity = 1.0
                return Pal::premul(vec4(tinted.xyz, tinted.w * opacity))
            }
        }
    }

    // Botão de texto do menubar (File/Edit/New/…).
    TbTextBtn = <Button> {
        height: 20,
        draw_bg: {
            color: (TW_SURF_HI),
            color_hover: #666666,
            color_down: #474747,
            color_focus: (TW_SURF_HI),
            color_disabled: (TW_SURF_HI),
            border_size: 0.0,
            border_radius: 3.0,
        }
        draw_text: { color: (TW_TEXT), text_style: { font_size: 11.0 } }
    }

    // -----------------------------------------------------------------
    // Toolbar completo (34 px de altura — a timeline encaixada começa
    // exatamente abaixo desta barra).
    // -----------------------------------------------------------------
    pub Toolbar = <View> {
        width: Fill,
        height: 34,
        show_bg: true,
        draw_bg: { color: (TW_BASE) }
        flow: Right,
        spacing: 10,
        padding: { left: 8, right: 4 }

        // Logo + título -------------------------------------------------
        tb_logo = <Image> {
            source: dep("crate://self/resources/ruana_studio_2x.png"),
            width: 26, height: 26,
        }
        tb_title = <Label> {
            text: "RuanaStudio",
            draw_text: { color: (TW_TEXT), text_style: { font_size: 12.0 } }
        }

        // Menus ----------------------------------------------------------
        tb_menus = <View> {
            flow: Right,
            spacing: 2,
            tb_menu_file = <TbTextBtn> { text: "File" }
            tb_menu_edit = <TbTextBtn> { text: "Edit" }
            tb_menu_new = <TbTextBtn> { text: "New" }
            tb_menu_save = <TbTextBtn> { text: "Save" }
            tb_menu_saveas = <TbTextBtn> { text: "Save As" }
            tb_menu_settings = <TbTextBtn> { text: "Settings" }
            tb_menu_help = <TbTextBtn> { text: "Help" }
        }

        // Undo/Redo -------------------------------------------------------
        tb_editrow = <View> {
            flow: Right,
            spacing: 2,
            tb_undo = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_undo_tile = <TbTile> {}
                tb_undo_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_undo_icon = <TbIcon> { source: dep("crate://self/resources/icons/undo_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_undo_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/undo_2x.png"), draw_bg: { tint: (TW_ON_ACCENT) }, icon_walk: { width: 15, height: 15 } }
                tb_undo_hit = <TbHit> {}
            }
            tb_redo = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_redo_tile = <TbTile> {}
                tb_redo_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_redo_icon = <TbIcon> { source: dep("crate://self/resources/icons/redo_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_redo_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/redo_2x.png"), draw_bg: { tint: (TW_ON_ACCENT) }, icon_walk: { width: 15, height: 15 } }
                tb_redo_hit = <TbHit> {}
            }
        }

        // Follow / Snap (toggles) ---------------------------------------
        tb_toggles = <View> {
            flow: Right,
            spacing: 2,
            tb_follow = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_follow_tile = <TbTile> {}
                tb_follow_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_follow_icon = <TbIcon> { source: dep("crate://self/resources/icons/follow_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_follow_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/follow_filled_2x.png"), draw_bg: { tint: (TW_ON_ACCENT) }, icon_walk: { width: 15, height: 15 } }
                tb_follow_hit = <TbHit> {}
            }
            tb_snap = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_snap_tile = <TbTile> {}
                tb_snap_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_snap_icon = <TbIcon> { source: dep("crate://self/resources/icons/snap_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_snap_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/snap_filled_2x.png"), draw_bg: { tint: (TW_ON_ACCENT) }, icon_walk: { width: 15, height: 15 } }
                tb_snap_hit = <TbHit> {}
            }
        }

        // Project key ----------------------------------------------------
        tb_key = <DropDown> {
            width: 68, height: 18,
            selected_item: 0,
            labels: ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"],
        }

        // Theme picker ----------------------------------------------------
        tb_theme = <DropDown> {
            width: 96, height: 18,
            selected_item: 0,
            labels: ["Studio Dark", "Studio Light", "Studio Blue", "Studio Green", "Metal", "Flat", "Paper"],
        }

        // Transport -------------------------------------------------------
        tb_transport = <View> {
            flow: Right,
            spacing: 5,
            tb_pos = <Label> {
                width: 96, height: 18,
                text: "01.01.000",
                draw_text: {
                    color: (TW_ACCENT),
                    text_style: { font_size: 12.0 },
                    horizontal_align: Center,
                    vertical_align: Center,
                }
            }

            tb_play = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_play_tile = <TbTile> {}
                tb_play_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_play_icon = <TbIcon> { source: dep("crate://self/resources/icons/play_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_pause_icon = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/pause_2x.png"), draw_bg: { tint: (TW_ACCENT) }, icon_walk: { width: 15, height: 15 } }
                tb_play_hit = <TbHit> {}
            }

            tb_stop = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_stop_tile = <TbTile> {}
                tb_stop_icon = <TbIcon> { source: dep("crate://self/resources/icons/stop_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_stop_hit = <TbHit> {}
            }
            tb_rec = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_rec_tile = <TbTile> {}
                tb_rec_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }
                tb_rec_icon = <TbIcon> { source: dep("crate://self/resources/icons/rec_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_rec_hit = <TbHit> {}
            }
            tb_loop = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_loop_tile = <TbTile> {}
                tb_loop_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_loop_icon = <TbIcon> { source: dep("crate://self/resources/icons/loop_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_loop_hit = <TbHit> {}
            }
            tb_metro = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_metro_tile = <TbTile> {}
                tb_metro_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_ACCENT) } }
                tb_metro_icon = <TbIcon> { source: dep("crate://self/resources/icons/metronome_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 15, height: 15 } }
                tb_metro_hit = <TbHit> {}
            }
            tb_bpmbox = <View> {
                flow: Right,
                spacing: 2,
                tb_bpm = <TextInput> { width: 30, height: 18, text: "120", draw_text: { color: (TW_TEXT), text_style: { font_size: 12.0 }, horizontal_align: Center, vertical_align: Center } }
                tb_bpm_label = <Label> { text: "BPM", draw_text: { color: (TW_TEXT_DIM), text_style: { font_size: 11.0 } } }
            }
            tb_tsig = <Label> { width: 30, height: 18, text: "4/4", draw_text: { color: (TW_TEXT_DIM), text_style: { font_size: 12.0 }, horizontal_align: Center, vertical_align: Center } }
        }

        // Ferramentas ----------------------------------------------------
        tb_tools = <View> {
            flow: Right,
            spacing: 2,
            tb_t0 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t0_tile = <TbTile> {}, tb_t0_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t0_icon = <TbIcon> { source: dep("crate://self/resources/icons/select_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t0_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/select_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t0_hit = <TbHit> {} }
            tb_t1 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t1_tile = <TbTile> {}, tb_t1_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t1_icon = <TbIcon> { source: dep("crate://self/resources/icons/slice_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t1_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/slice_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t1_hit = <TbHit> {} }
            tb_t2 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t2_tile = <TbTile> {}, tb_t2_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t2_icon = <TbIcon> { source: dep("crate://self/resources/icons/glue_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t2_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/glue_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t2_hit = <TbHit> {} }
            tb_t3 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t3_tile = <TbTile> {}, tb_t3_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t3_icon = <TbIcon> { source: dep("crate://self/resources/icons/stretch_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t3_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/stretch_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t3_hit = <TbHit> {} }
            tb_t4 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t4_tile = <TbTile> {}, tb_t4_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t4_icon = <TbIcon> { source: dep("crate://self/resources/icons/draw_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t4_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/draw_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t4_hit = <TbHit> {} }
            tb_t5 = <View> { width: 19, height: 19, flow: Overlay, align: { x: 0.5, y: 0.5 }, tb_t5_tile = <TbTile> {}, tb_t5_tile_on = <TbTile> { visible: false, draw_bg: { color: (TW_CRIMSON) } }, tb_t5_icon = <TbIcon> { source: dep("crate://self/resources/icons/erase_2x.png"), draw_bg: { tint: (TW_TEXT) }, icon_walk: { width: 16, height: 16 } }, tb_t5_icon_f = <TbIcon> { visible: false, source: dep("crate://self/resources/icons/erase_filled_2x.png"), draw_bg: { tint: (TW_ON_CRIM) }, icon_walk: { width: 16, height: 16 } }, tb_t5_hit = <TbHit> {} }
        }

        // Controles da janela (dots) — canto direito ----------------------
        tb_dots = <View> {
            flow: Right,
            spacing: 6,
            tb_dot_min = <Button> { width: 18, height: 18, text: "", draw_bg: { color: (TW_WARN), color_hover: (TW_WARN_H), color_down: #E0A51E, color_focus: (TW_WARN), color_disabled: (TW_WARN), border_size: 0.0, border_radius: 9.0 } }
            tb_dot_max = <Button> { width: 18, height: 18, text: "", draw_bg: { color: (TW_OK), color_hover: (TW_OK_H), color_down: #1FA830, color_focus: (TW_OK), color_disabled: (TW_OK), border_size: 0.0, border_radius: 9.0 } }
            tb_dot_close = <Button> { width: 18, height: 18, text: "", draw_bg: { color: (TW_DANGER), color_hover: (TW_DANGER_H), color_down: (TW_DANGER_D), color_focus: (TW_DANGER), color_disabled: (TW_DANGER), border_size: 0.0, border_radius: 9.0 } }
        }
    }
}

/// Ações emitidas pelo toolbar para o shell tratar.
#[derive(Clone, Debug)]
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

/// Formata a posição em barras/compassos/ticks (mesmo estilo do toolbar.slint).
pub fn fmt_position(ticks: i64, ppq: i32, beats_per_bar: i32) -> String {
    let tpb = (ppq as i64) * (beats_per_bar as i64).max(1);
    let bar = ticks / tpb + 1;
    let rem = ticks % tpb;
    let beat = rem / (ppq as i64) + 1;
    let tick = rem % (ppq as i64);
    format!("{bar:02}.{beat:02}.{tick:03}")
}

/// Lógica do toolbar: lê cliques/ações por `id!()` e sincroniza a aparência
/// (ícones normal/filled, tiles idle/ativo) com o estado autoritativo do shell.
pub struct ToolbarCtl;

impl ToolbarCtl {
    pub fn handle_actions(ui: &WidgetRef, actions: &Actions, out: &mut Vec<ToolbarAction>) {
        if ui.button(id!(tb_menu_file)).clicked(actions) {
            out.push(ToolbarAction::Menu(0));
        }
        if ui.button(id!(tb_menu_edit)).clicked(actions) {
            out.push(ToolbarAction::Menu(1));
        }
        if ui.button(id!(tb_menu_new)).clicked(actions) {
            out.push(ToolbarAction::Menu(2));
        }
        if ui.button(id!(tb_menu_save)).clicked(actions) {
            out.push(ToolbarAction::Menu(3));
        }
        if ui.button(id!(tb_menu_saveas)).clicked(actions) {
            out.push(ToolbarAction::Menu(4));
        }
        if ui.button(id!(tb_menu_settings)).clicked(actions) {
            out.push(ToolbarAction::Menu(5));
        }
        if ui.button(id!(tb_menu_help)).clicked(actions) {
            out.push(ToolbarAction::Menu(6));
        }
        if ui.button(id!(tb_undo_hit)).clicked(actions) {
            out.push(ToolbarAction::Undo);
        }
        if ui.button(id!(tb_redo_hit)).clicked(actions) {
            out.push(ToolbarAction::Redo);
        }
        if ui.button(id!(tb_follow_hit)).clicked(actions) {
            out.push(ToolbarAction::ToggleFollow);
        }
        if ui.button(id!(tb_snap_hit)).clicked(actions) {
            out.push(ToolbarAction::ToggleSnap);
        }
        if let Some(i) = ui.drop_down(id!(tb_key)).selected(&actions) {
            out.push(ToolbarAction::ProjectKey(i));
        }
        if let Some(i) = ui.drop_down(id!(tb_theme)).selected(&actions) {
            out.push(ToolbarAction::ThemeSelected(i));
        }
        if ui.button(id!(tb_play_hit)).clicked(actions) {
            out.push(ToolbarAction::TogglePlay);
        }
        if ui.button(id!(tb_stop_hit)).clicked(actions) {
            out.push(ToolbarAction::Stop);
        }
        if ui.button(id!(tb_rec_hit)).clicked(actions) {
            out.push(ToolbarAction::ToggleRec);
        }
        if ui.button(id!(tb_loop_hit)).clicked(actions) {
            out.push(ToolbarAction::ToggleLoop);
        }
        if ui.button(id!(tb_metro_hit)).clicked(actions) {
            out.push(ToolbarAction::ToggleMetronome);
        }
        if let Some((text, _)) = ui.text_input(id!(tb_bpm)).returned(&actions) {
            if let Ok(bpm) = text.trim().parse::<f32>() {
                if (40.0..=300.0).contains(&bpm) {
                    out.push(ToolbarAction::SetBpm(bpm));
                }
            }
        }
        let tools = [
            id!(tb_t0_hit),
            id!(tb_t1_hit),
            id!(tb_t2_hit),
            id!(tb_t3_hit),
            id!(tb_t4_hit),
            id!(tb_t5_hit),
        ];
        for (i, path) in tools.iter().enumerate() {
            if ui.button(*path).clicked(actions) {
                out.push(ToolbarAction::ToolSelected(i));
            }
        }
        if ui.button(id!(tb_dot_min)).clicked(actions) {
            out.push(ToolbarAction::Minimize);
        }
        if ui.button(id!(tb_dot_max)).clicked(actions) {
            out.push(ToolbarAction::Maximize);
        }
        if ui.button(id!(tb_dot_close)).clicked(actions) {
            out.push(ToolbarAction::Close);
        }
    }

    pub fn set_toggle(
        cx: &mut Cx,
        ui: &WidgetRef,
        tile: &[LiveId],
        tile_on: &[LiveId],
        icon: &[LiveId],
        icon_f: &[LiveId],
        on: bool,
    ) {
        ui.view(tile).set_visible(cx, !on);
        ui.view(tile_on).set_visible(cx, on);
        ui.view(icon).set_visible(cx, !on);
        ui.view(icon_f).set_visible(cx, on);
    }

    pub fn set_playing(cx: &mut Cx, ui: &WidgetRef, playing: bool) {
        Self::set_toggle(
            cx,
            ui,
            id!(tb_play_tile),
            id!(tb_play_tile_on),
            id!(tb_play_icon),
            id!(tb_pause_icon),
            playing,
        );
    }
    pub fn set_follow(cx: &mut Cx, ui: &WidgetRef, on: bool) {
        Self::set_toggle(
            cx,
            ui,
            id!(tb_follow_tile),
            id!(tb_follow_tile_on),
            id!(tb_follow_icon),
            id!(tb_follow_icon_f),
            on,
        );
    }
    pub fn set_snap(cx: &mut Cx, ui: &WidgetRef, on: bool) {
        Self::set_toggle(
            cx,
            ui,
            id!(tb_snap_tile),
            id!(tb_snap_tile_on),
            id!(tb_snap_icon),
            id!(tb_snap_icon_f),
            on,
        );
    }
    pub fn set_rec(cx: &mut Cx, ui: &WidgetRef, on: bool) {
        ui.view(id!(tb_rec_tile)).set_visible(cx, !on);
        ui.view(id!(tb_rec_tile_on)).set_visible(cx, on);
    }
    pub fn set_loop(cx: &mut Cx, ui: &WidgetRef, on: bool) {
        ui.view(id!(tb_loop_tile)).set_visible(cx, !on);
        ui.view(id!(tb_loop_tile_on)).set_visible(cx, on);
    }
    pub fn set_metronome(cx: &mut Cx, ui: &WidgetRef, on: bool) {
        ui.view(id!(tb_metro_tile)).set_visible(cx, !on);
        ui.view(id!(tb_metro_tile_on)).set_visible(cx, on);
    }
    pub fn set_position(cx: &mut Cx, ui: &WidgetRef, ticks: i64, ppq: i32, beats_per_bar: i32) {
        ui.label(id!(tb_pos))
            .set_text(cx, &fmt_position(ticks, ppq, beats_per_bar));
    }
    pub fn set_bpm(cx: &mut Cx, ui: &WidgetRef, bpm: f32) {
        ui.text_input(id!(tb_bpm))
            .set_text(cx, &format!("{bpm:.0}"));
    }
    pub fn set_tool(cx: &mut Cx, ui: &WidgetRef, active: usize) {
        let tiles = [
            (
                id!(tb_t0_tile),
                id!(tb_t0_tile_on),
                id!(tb_t0_icon),
                id!(tb_t0_icon_f),
            ),
            (
                id!(tb_t1_tile),
                id!(tb_t1_tile_on),
                id!(tb_t1_icon),
                id!(tb_t1_icon_f),
            ),
            (
                id!(tb_t2_tile),
                id!(tb_t2_tile_on),
                id!(tb_t2_icon),
                id!(tb_t2_icon_f),
            ),
            (
                id!(tb_t3_tile),
                id!(tb_t3_tile_on),
                id!(tb_t3_icon),
                id!(tb_t3_icon_f),
            ),
            (
                id!(tb_t4_tile),
                id!(tb_t4_tile_on),
                id!(tb_t4_icon),
                id!(tb_t4_icon_f),
            ),
            (
                id!(tb_t5_tile),
                id!(tb_t5_tile_on),
                id!(tb_t5_icon),
                id!(tb_t5_icon_f),
            ),
        ];
        let active = active.min(5);
        for (i, (tile, tile_on, icon, icon_f)) in tiles.iter().enumerate() {
            let on = i == active;
            ui.view(*tile).set_visible(cx, !on);
            ui.view(*tile_on).set_visible(cx, on);
            ui.view(*icon).set_visible(cx, !on);
            ui.view(*icon_f).set_visible(cx, on);
        }
    }
}
