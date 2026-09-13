//! Toolbar em Makepad — port de `ui/toolbar.slint` para um componente
//! separado (`app-makepad/src/toolbar.rs`).
//!
//! Não é um widget Rust custom: o toolbar é reproduzido como um protótipo
//! `<Toolbar>` (uma `View` pura) registrado via `live_design!`, e a lógica de
//! interação vive em `ToolbarCtl`, que manipula os widgets por `id!()` e
//! devolve `ToolbarAction`s (mesmo padrão do shell em `app.rs`).

use makepad_widgets::*;

live_design! {
    link widgets;
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    // -----------------------------------------------------------------
    // Paleta (Studio Dark — índice 0 do `ui/theme.slint`)
    // -----------------------------------------------------------------
    // base        #313131 (fundo do toolbar)
    // surface     #242424
    // surface-high#555555 (tiles/botões)
    // text        #E8E8E8
    // accent      #00C853 (toggles/play ligados)
    // crimson     #DC143C (ferramenta ativa / rec armado)
    // dots        #FEBC2E minimize, #28C840 maximize, #FF5F57 close

    // Button de ícone: fundo autônomo (água) que captura o clique.
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

    // Tile onde o ícone fica assentado (18×18 na maioria).
    pub TbTile = <RoundedView> {
        width: Fill, height: Fill,
        draw_bg: { color: #555555, border_radius: 3.0, border_size: 0.0 }
    }

    // Image com tint multiplicativo (para ícones coloridos, ex.: play).
    pub TintImage = <Image> {
        draw_bg: {
            instance tint: vec4
            fn get_color(self) -> vec4 {
                let c = self.get_color_scale_pan(self.image_scale, self.image_pan)
                return vec4(c.x * self.tint.x, c.y * self.tint.y, c.z * self.tint.z, c.w * self.tint.w)
            }
        }
    }

    // Botão de texto do menubar (File/Edit/New/…).
    TbTextBtn = <Button> {
        height: 20,
        draw_bg: {
            color: #555555,
            color_hover: #666666,
            color_down: #474747,
            color_focus: #555555,
            color_disabled: #555555,
            border_size: 0.0,
            border_radius: 3.0,
        }
        draw_text: { color: #E8E8E8, text_style: { font_size: 11.0 } }
    }

    // -----------------------------------------------------------------
    // Toolbar completo (34 px de altura — a timeline encaixada começa
    // exatamente abaixo desta barra).
    // -----------------------------------------------------------------
    pub Toolbar = <View> {
        width: Fill,
        height: 34,
        show_bg: true,
        draw_bg: { color: #313131 }
        flow: Right,
        spacing: 10,
        align: { x: 0.0, y: 0.5 },
        padding: { left: 8, right: 8 }

        // Logo + título -------------------------------------------------
        tb_logo = <Image> {
            source: dep("assets/ruana_studio_2x.png"),
            width: 26, height: 26,
            
        }
        tb_title = <Label> {
            text: "RuanaStudio",
            draw_text: { color: #E8E8E8, text_style: { font_size: 12.0 } }
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
                tb_undo_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_undo_icon = <Image> {
                    source: dep("assets/icons/undo_2x.png"),
                    width: 15, height: 15,
                }
                tb_undo_icon_f = <Image> {
                    visible: false,
                    source: dep("assets/icons/undo_2x.png"),
                    width: 15, height: 15,
                }
                tb_undo_hit = <TbHit> {}
            }
            tb_redo = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_redo_tile = <TbTile> {}
                tb_redo_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_redo_icon = <Image> {
                    source: dep("assets/icons/redo_2x.png"),
                    width: 15, height: 15,
                }
                tb_redo_icon_f = <Image> {
                    visible: false,
                    source: dep("assets/icons/redo_2x.png"),
                    width: 15, height: 15,
                }
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
                tb_follow_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_follow_icon = <Image> {
                    source: dep("assets/icons/follow_2x.png"),
                    width: 15, height: 15,
                }
                tb_follow_icon_f = <Image> {
                    visible: false,
                    source: dep("assets/icons/follow_filled_2x.png"),
                    width: 15, height: 15,
                }
                tb_follow_hit = <TbHit> {}
            }
            tb_snap = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_snap_tile = <TbTile> {}
                tb_snap_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_snap_icon = <Image> {
                    source: dep("assets/icons/snap_2x.png"),
                    width: 15, height: 15,
                }
                tb_snap_icon_f = <Image> {
                    visible: false,
                    source: dep("assets/icons/snap_filled_2x.png"),
                    width: 15, height: 15,
                }
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
                    color: #00C853,
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
                tb_play_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_play_icon = <Image> {
                    source: dep("assets/icons/play_2x.png"),
                    width: 15, height: 15,
                }
                tb_pause_icon = <Image> {
                    visible: false,
                    source: dep("assets/icons/pause_2x.png"),
                    width: 15, height: 15,
                }
                tb_play_hit = <TbHit> {}
            }

            tb_stop = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_stop_tile = <TbTile> {}
                tb_stop_icon = <Image> {
                    source: dep("assets/icons/stop_2x.png"),
                    width: 15, height: 15,
                }
                tb_stop_hit = <TbHit> {}
            }

            tb_rec = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_rec_tile = <TbTile> {}
                tb_rec_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_rec_icon = <Image> {
                    source: dep("assets/icons/rec_2x.png"),
                    width: 15, height: 15,
                }
                tb_rec_hit = <TbHit> {}
            }

            tb_loop = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_loop_tile = <TbTile> {}
                tb_loop_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_loop_icon = <Image> {
                    source: dep("assets/icons/loop_2x.png"),
                    width: 15, height: 15,
                }
                tb_loop_hit = <TbHit> {}
            }

            tb_metro = <View> {
                width: 18, height: 18,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_metro_tile = <TbTile> {}
                tb_metro_tile_on = <TbTile> { visible: false, draw_bg: { color: #00C853 } }
                tb_metro_icon = <Image> {
                    source: dep("assets/icons/metronome_2x.png"),
                    width: 15, height: 15,
                }
                tb_metro_hit = <TbHit> {}
            }

            tb_bpmbox = <View> {
                flow: Right,
                spacing: 2,
                tb_bpm = <TextInput> {
                    width: 30, height: 18,
                    text: "120",
                    draw_text: {
                        color: #E8E8E8,
                        text_style: { font_size: 12.0 },
                        horizontal_align: Center,
                        vertical_align: Center,
                    }
                }
                tb_bpm_label = <Label> {
                    text: "BPM",
                    draw_text: { color: #9A9A9A, text_style: { font_size: 11.0 } }
                }
            }

            tb_tsig = <Label> {
                width: 30, height: 18,
                text: "4/4",
                draw_text: {
                    color: #9A9A9A,
                    text_style: { font_size: 12.0 },
                    horizontal_align: Center,
                    vertical_align: Center,
                }
            }
        }

        // Ferramentas (EditToolbar: select/slice/glue/stretch/draw/erase) --
        tb_tools = <View> {
            flow: Right,
            spacing: 2,

            tb_t0 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t0_tile = <TbTile> {}
                tb_t0_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t0_icon = <Image> { source: dep("assets/icons/select_2x.png"), width: 16, height: 16 }
                tb_t0_icon_f = <Image> { visible: false, source: dep("assets/icons/select_filled_2x.png"), width: 16, height: 16 }
                tb_t0_hit = <TbHit> {}
            }
            tb_t1 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t1_tile = <TbTile> {}
                tb_t1_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t1_icon = <Image> { source: dep("assets/icons/slice_2x.png"), width: 16, height: 16 }
                tb_t1_icon_f = <Image> { visible: false, source: dep("assets/icons/slice_filled_2x.png"), width: 16, height: 16 }
                tb_t1_hit = <TbHit> {}
            }
            tb_t2 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t2_tile = <TbTile> {}
                tb_t2_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t2_icon = <Image> { source: dep("assets/icons/glue_2x.png"), width: 16, height: 16 }
                tb_t2_icon_f = <Image> { visible: false, source: dep("assets/icons/glue_filled_2x.png"), width: 16, height: 16 }
                tb_t2_hit = <TbHit> {}
            }
            tb_t3 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t3_tile = <TbTile> {}
                tb_t3_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t3_icon = <Image> { source: dep("assets/icons/stretch_2x.png"), width: 16, height: 16 }
                tb_t3_icon_f = <Image> { visible: false, source: dep("assets/icons/stretch_filled_2x.png"), width: 16, height: 16 }
                tb_t3_hit = <TbHit> {}
            }
            tb_t4 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t4_tile = <TbTile> {}
                tb_t4_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t4_icon = <Image> { source: dep("assets/icons/draw_2x.png"), width: 16, height: 16 }
                tb_t4_icon_f = <Image> { visible: false, source: dep("assets/icons/draw_filled_2x.png"), width: 16, height: 16 }
                tb_t4_hit = <TbHit> {}
            }
            tb_t5 = <View> {
                width: 19, height: 19,
                flow: Overlay,
                align: { x: 0.5, y: 0.5 },
                tb_t5_tile = <TbTile> {}
                tb_t5_tile_on = <TbTile> { visible: false, draw_bg: { color: #DC143C } }
                tb_t5_icon = <Image> { source: dep("assets/icons/erase_2x.png"), width: 16, height: 16 }
                tb_t5_icon_f = <Image> { visible: false, source: dep("assets/icons/erase_filled_2x.png"), width: 16, height: 16 }
                tb_t5_hit = <TbHit> {}
            }
        }

        // Controles da janela (dots) --------------------------------------
        tb_dots = <View> {
            flow: Right,
            spacing: 4,
            tb_dot_min = <Button> {
                width: 14, height: 14,
                text: "",
                draw_bg: {
                    color: #FEBC2E,
                    color_hover: #FFD25E,
                    color_down: #E0A51E,
                    color_focus: #FEBC2E,
                    color_disabled: #FEBC2E,
                    border_size: 0.0,
                    border_radius: 7.0,
                }
            }
            tb_dot_max = <Button> {
                width: 14, height: 14,
                text: "",
                draw_bg: {
                    color: #28C840,
                    color_hover: #48DC5E,
                    color_down: #1FA830,
                    color_focus: #28C840,
                    color_disabled: #28C840,
                    border_size: 0.0,
                    border_radius: 7.0,
                }
            }
            tb_dot_close = <Button> {
                width: 14, height: 14,
                text: "",
                draw_bg: {
                    color: #FF5F57,
                    color_hover: #FF7F78,
                    color_down: #E03F38,
                    color_focus: #FF5F57,
                    color_disabled: #FF5F57,
                    border_size: 0.0,
                    border_radius: 7.0,
                }
            }
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
    /// Processa as ações da UI: preenche `out` com o que o shell deve tratar.
    /// Toggles (follow/snap/rec/loop/metro) chegam sem payload — o shell mantém
    /// o estado autoritativo e devolve via `set_*`.
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

    // ---- Sincronização de estado (chamado pelo shell) ------------------

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
        ui.image(icon).set_visible(cx, !on);
        ui.image(icon_f).set_visible(cx, on);
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

    /// Marca a ferramenta `active` (tile ativo + ícone filled branco).
    pub fn set_tool(cx: &mut Cx, ui: &WidgetRef, active: usize) {
        let tiles = [
            (id!(tb_t0_tile), id!(tb_t0_tile_on), id!(tb_t0_icon), id!(tb_t0_icon_f)),
            (id!(tb_t1_tile), id!(tb_t1_tile_on), id!(tb_t1_icon), id!(tb_t1_icon_f)),
            (id!(tb_t2_tile), id!(tb_t2_tile_on), id!(tb_t2_icon), id!(tb_t2_icon_f)),
            (id!(tb_t3_tile), id!(tb_t3_tile_on), id!(tb_t3_icon), id!(tb_t3_icon_f)),
            (id!(tb_t4_tile), id!(tb_t4_tile_on), id!(tb_t4_icon), id!(tb_t4_icon_f)),
            (id!(tb_t5_tile), id!(tb_t5_tile_on), id!(tb_t5_icon), id!(tb_t5_icon_f)),
        ];
        let active = active.min(5);
        for (i, (tile, tile_on, icon, icon_f)) in tiles.iter().enumerate() {
            let on = i == active;
            ui.view(*tile).set_visible(cx, !on);
            ui.view(*tile_on).set_visible(cx, on);
            ui.image(*icon).set_visible(cx, !on);
            ui.image(*icon_f).set_visible(cx, on);
        }
    }

    pub fn set_bpm(cx: &mut Cx, ui: &WidgetRef, bpm: f32) {
        ui.text_input(id!(tb_bpm)).set_text(cx, &format!("{bpm:.0}"));
    }

    pub fn set_position(cx: &mut Cx, ui: &WidgetRef, ticks: i64, ppq: i32, beats_per_bar: i32) {
        ui.label(id!(tb_pos)).set_text(cx, &fmt_position(ticks, ppq, beats_per_bar));
    }
}