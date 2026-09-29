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
use daw_core::timeline::TempoMap;
use daw_core::transport::{PlayMode, Transport};

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

/// Publica no Slint em qual superfície o ponteiro está, para o play ser
/// roteado pelo contexto (espaço / botão de play do header do beat).
///
/// Feito aqui porque o Rust é o dono do `CursorMoved` do winit e o Slint
/// não expõe um ponteiro global. Um `TouchArea.has-hover` dentro da
/// popup não serviria: o hit-test do Slint entrega o ponteiro só ao item
/// de cima, e todo o conteúdo (header, grid, trilhas) fica ACIMA de um
/// probe de hover — ele nunca veria `has-hover == true`.
///
/// `x`/`y` em px LÓGICOS de janela (o mesmo espaço de
/// `beat-popup-x`… e de `timeline-canvas-abs-x`).
fn update_pointer_context(ui: &MainWindow, x: f32, y: f32) {
    let w = ui.get_beat_popup_w();
    let h = ui.get_beat_popup_h();
    let over_beat = ui.get_beat_open()
        && w > 0.0
        && h > 0.0
        && x >= ui.get_beat_popup_x()
        && x < ui.get_beat_popup_x() + w
        && y >= ui.get_beat_popup_y()
        && y < ui.get_beat_popup_y() + h;
    if ui.get_pointer_over_beat() != over_beat {
        ui.set_pointer_over_beat(over_beat);
    }
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
    pub audio_sync_timer: Option<slint::Timer>,
    /// Canto superior-esquerdo da janela (px físicos). Manter atualizado no
    /// `Moved`; usado para converter a posição GLOBAL do cursor (GetCursorPos)
    /// em coordenadas da janela durante drags do SO.
    pub win_pos: (f64, f64),
    /// Posição do cursor em px LÓGICOS relativos à janela (último
    /// `CursorMoved`). Usada pela rota do play por contexto — o Slint
    /// não tem um "ponteiro global" para consultar.
    pub cursor_logical: (f32, f32),
    /// O projeto mudou → o espelho de clips precisa de rebuild.
    pub clips_flag: Rc<Cell<bool>>,
    /// Ids que o espelho já carregou no engine (tick). Guarda do write-back
    /// de clips: um clip recém-importado só pode ser apagado do Project
    /// depois que o rebuild admitir o id no engine.
    pub mirrored_clip_ids: Rc<RefCell<Vec<u64>>>,
    /// Duplo clique num clip na timeline → id para abrir o editor. A engine
    /// roda com o `bridge` emprestado durante o press, então ela só marca a
    /// flag; o `tick` (que tem o `ui`) despacha de fato.
    pub open_clip_flag: Rc<Cell<Option<u32>>>,
    /// Roteamento de "abrir clip", instalado pelo `setup` do app: só o app
    /// tem `AudioBridge`/`Project` para escolher e abrir o editor certo.
    pub open_clip_handler: Option<Rc<dyn Fn(u64)>>,
    /// A cena precisa de um novo frame (rendered direto para `timeline_bg`).
    pub dirty: bool,

    // Modelos da UI para sincronização reversa (clips importados pelo drag OS).
    pub waveforms: Option<Rc<RefCell<crate::waveform::WaveformCache>>>,
    pub clips_model: Option<Rc<slint::VecModel<crate::ClipData>>>,
    pub track_colors_model: Option<Rc<slint::VecModel<i32>>>,
}

impl EngineBridge {
    pub fn new(
        mut engine: TimelineEngine,
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
        // Timewarp → write-back: quando o usuário edita pontos de tempo na
        // engine (timewarp), o mapa atualizado é gravado no projeto do core
        // (fonte da verdade), preservando undo/redo do documento.
        {
            let project = project.clone();
            engine.tempo_write_back = Some(Box::new(move |map: &TempoMap| {
                project.borrow_mut().tempo_map = map.clone();
            }));
        }
        let mirrored_clip_ids = Rc::new(RefCell::new(Vec::<u64>::new()));
        // Clips → write-back: toda edição estrutural na engine (mover, deletar,
        // colar, duplicar, desenhar, fatiar) entrega a lista completa de clips.
        // Reconcilia com o `Project` (fonte da verdade do áudio): sem isso o
        // áudio continuaria tocando clips apagados / na posição antiga e
        // qualquer import (rebuild do espelho) sumiria com clips da engine.
        {
            let project = project.clone();
            let mirrored = mirrored_clip_ids.clone();
            engine.clip_write_back =
                Some(Box::new(move |clips: &[timeline_render::clips::ClipDemo]| {
                    crate::timeline_slint::reconcile_engine_clips_to_project(
                        clips,
                        &mut project.borrow_mut(),
                        &mirrored.borrow(),
                    );
                }));
        }
        // Duplo clique num clip: a engine só marca o id (ela roda com o bridge
        // emprestado). O tick resolve no Project e abre o editor.
        let open_clip_flag: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));
        {
            let flag = open_clip_flag.clone();
            engine.clip_open_request = Some(Box::new(move |id: u32| {
                flag.set(Some(id));
            }));
        }
        Self {
            engine,
            overlay,
            transport,
            project,
            scale,
            audio_engine: None,
            audio_sync_timer: None,
            win_pos,
            cursor_logical: (0.0, 0.0),
            clips_flag,
            mirrored_clip_ids,
            open_clip_flag,
            open_clip_handler: None,
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

        // Duplo clique num clip → abre o editor correspondente. O id chega
        // pela flag porque a engine não pode chamar de volta enquanto o
        // bridge está emprestado pelo callback de ponteiro do Slint; o
        // roteamento (piano roll / sampler) é do app, instalado no setup.
        if let Some(id) = self.open_clip_flag.take() {
            if let Some(handler) = self.open_clip_handler.clone() {
                handler(id as u64);
            }
        }

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

        // =====================================================
        // ESCALA DA CENA → SLINT (a cada frame, sem Timer)
        //
        // A engine é quem desenhou o frame anterior; o `drag-move`
        // do main.slint e o ghost do ClipPool precisam das MESMAS
        // medidas para converter cursor → beat/linha. Antes elas
        // vinham de um Timer de 50ms copiando constantes do Slint
        // (40px/beat, 42px de linha, 25px de régua) que nunca
        // foram as da engine (px_per_beat vivo, 64*zoom_y, 28px):
        // o preview que o usuário via e o ponto onde o clip entrava
        // discordavam, e o ghost só "acertava" quando o Timer
        // tinha rodado. Publicar aqui elimina a divergência E o
        // atraso de 50ms — o card passa a seguir o cursor no mesmo
        // frame em que o cursor se moveu.
        //
        // Tudo em px LÓGICOS (o Slint trabalha em lógico; a engine
        // em físico).
        // =====================================================
        {
            // Origem: o MESMO retângulo do quad (`rx`/`ry` já incluem o
            // `left-padding`), para o cursor → beat/linha do Slint cair
            // em cima do que a engine desenhou, e não numa origem
            // paralela que Divergiria com o padding.
            let abs_x = rx / ls;
            let abs_y = ry / ls;
            let rect_w = rw / ls;
            let rect_h = rh / ls;

            // Altura da linha: a engine desenha 64 * zoom_y (físico).
            let row_h = 64.0 * self.engine.zoom_y.max(0.001) / ls;
            let ruler_h = self.engine.ruler_px / ls;
            let ppb = self.engine.px_per_beat / ls;

            // O `px-per-beat`/`row-height`/`ruler-height` do
            // componente Timeline deixam de ser constantes: o
            // Rust passa a ser o dono, o que conserta também o
            // `map-window-coords-to-timeline` (main.slint), que
            // fazia a mesma conta com os números velhos.
            ui.set_timeline_px_per_beat(ppb);
            ui.set_timeline_row_height(row_h);
            ui.set_timeline_ruler_height(ruler_h);

            let m = <crate::TimelineMetrics as slint::Global<crate::MainWindow>>::get(&ui);
            m.set_timeline_abs_x(abs_x);
            m.set_timeline_abs_y(abs_y);
            m.set_content_top_abs_y(abs_y + ruler_h);
            m.set_width(rect_w);
            m.set_rect_height(rect_h);
            m.set_ruler_height(ruler_h);
            m.set_px_per_beat(ppb);
            m.set_row_height(row_h);
            m.set_scroll_x(self.engine.scroll_x / ls);
            m.set_scroll_y(self.engine.scroll_y / ls);
            m.set_snap_beat_step(self.engine.grid_beat_step().max(1e-4));
        }

        // Refresca a rota do play por contexto. O `CursorMoved` já faz
        // isso a cada movimento, mas a popup do beat é ARRASTÁVEL: sem
        // esta passada, mover a janela do beat com o cursor por cima
        // deixaria `pointer-over-beat` preso no valor antigo.
        {
            let (cx, cy) = self.cursor_logical;
            update_pointer_context(ui, cx, cy);
        }

        // A engine é a fonte da verdade da área de exibição: P/T abertos
        // (show_piano_roll/timewarp) fazem a timeline ocupar a largura total
        // (esconde a track panel); zoom vertical e scroll são espelhados no
        // Slint para a track panel acompanhar as linhas do wgpu (mesma
        // proporção e mesmo offset).
        let expanded = self.engine.show_piano_roll || self.engine.show_timewarp;
        ui.set_timeline_area_expanded(expanded);
        ui.set_zoom_y_level(self.engine.zoom_y);
        ui.set_timeline_scroll_y((self.engine.scroll_y / ls.max(1e-4)).max(0.0));
        let slint_sx = ui.get_timeline_scroll_x();
        let eng_sx = self.engine.scroll_x;
        if (slint_sx * ls - eng_sx).abs() > 1.0 {
            eprintln!(
                "[dbg-scroll-div] tick: engine.scroll_x={eng_sx} slint.scroll-x*scale={} (slint={slint_sx})",
                slint_sx * ls,
            );
        }

        // Tamanho físico (px) da cena: igual ao quad (1:1).
        let aw = rw.round().clamp(0.0, 8192.0) as u32;
        let ah = rh.round().clamp(0.0, 8192.0) as u32;

        // Espelha clips (projeto mudou).
        if self.clips_flag.replace(false) {
            let proj = self.project.borrow();
            let bpm = self
                .transport
                .lock()
                .map(|t| t.bpm as f32)
                .unwrap_or(120.0);
            self.engine.bpm = bpm;
            // `bpm` alinha a região audível da wave (source_start_beats) com
            // o mesmo mapeamento beats→frames do motor de áudio.
            crate::timeline_slint::sync_clips_from_project(&mut self.engine, &proj, bpm);
            self.engine.tempo_map = proj.tempo_map.clone();
            *self.mirrored_clip_ids.borrow_mut() = proj.clips.iter().map(|c| c.id).collect();
            drop(proj);
            self.dirty = true;
        }

        // BPM mudou → re-decoda as waves (offsets de trim em beats) para
        // manter a waveform alinhada com o que o motor de áudio vai tocar.
        let tbpm = self
            .transport
            .lock()
            .map(|t| t.bpm as f32)
            .unwrap_or(120.0);
        if (self.engine.bpm - tbpm).abs() > 0.01 {
            self.engine.bpm = tbpm;
            self.clips_flag.set(true);
        }

        // Loop (botão da toolbar ↔ engine wgpu): o botão LOOP da toolbar
        // liga/desliga o loop no Transport compartilhado (consumido pelo
        // motor de áudio). Espelhamos esse estado no engine para a régua
        // wgpu mostrar/ocultar a região. A faixa editada na régua é escrita
        // de volta no Transport pela engine (`emit_loop_write_back`) quando o
        // arrasto acaba; durante o arrasto não repomos a região (senão o drag
        // "volta sozinho" a cada frame).
        {
            let t = self.transport.lock().unwrap();
            if self.engine.loop_enabled != t.loop_enabled {
                self.engine.loop_enabled = t.loop_enabled;
                self.dirty = true;
            }
            if self.engine.loop_drag.is_none() {
                let start_beat = t.loop_start_ticks as f32 / t.ppq as f32;
                let end_beat = (t.loop_end_ticks as f32 / t.ppq as f32).max(start_beat + 0.01);
                if (self.engine.loop_start_beat - start_beat).abs() > 0.0001 {
                    self.engine.loop_start_beat = start_beat;
                    self.dirty = true;
                }
                if (self.engine.loop_end_beat - end_beat).abs() > 0.0001 {
                    self.engine.loop_end_beat = end_beat;
                    self.dirty = true;
                }
            }
        }

        // Playhead segue o Transport compartilhado.
        //
        // No modo "só beat" o transporte TAMBÉM está `playing` (o motor
        // de áudio precisa da posição para agrupar o pattern), mas a
        // timeline tem de ficar CONGELADA: para a engine beat-only é
        // equivalente a não estar tocando — senão a agulha anda e o
        // follow-playhead rola a timeline enquanto só o pattern soa.
        let (playing, beat_only, pos) = {
            let t = self.transport.lock().unwrap();
            (
                t.playing && t.mode == PlayMode::Timeline,
                t.playing && t.mode == PlayMode::BeatOnly,
                t.position_seconds() as f32,
            )
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
            } else if !beat_only && (e.transport_seconds - pos).abs() > 0.00005 {
                // Parado de verdade: espelha a posição (seek/clique na
                // régua). Em modo beat o `pos` está descartado de
                // propósito, senão a agulha andaria junto do pattern.
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
        let mut clip_drag = ui.get_drag_active();

        // =====================================================
        // DRAG PRESO: o botão já foi solto mas o Slint não viu
        //
        // O `drag-finished` do ClipPool só dispara no
        // `PointerEventKind.up` DA TouchArea que fez o grab. Se o
        // `up` se perde — soltar fora da janela, Alt+Tab no meio
        // do arrasto, perder o foco — ninguém volta `drag-active`
        // para false: o card fica grudado na tela seguindo o
        // mouse, e o próximo clique é gasto como se fosse o
        // "soltar" daquele arrasto. É daí que vem a sensação de
        // que só.funciona se você for rápido.
        //
        // O estado real do botão do SO é a única fonte confiável
        // aqui: se ele já subiu, o arrasto acabou, ponto.
        // =====================================================
        if clip_drag && !left_button_is_down() {
            // Resolve o drop com a posição atual (o `drag-move` do
            // frame anterior já publishou beat/linha) e desliga o
            // ghost. `drag-end` limpa `drag-active` e `drag-inited`.
            ui.invoke_drag_end();
            clip_drag = false;
            self.dirty = true;
        }

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

        // Auto-scroll contínuo nas bordas durante drag ativo (mesmo com cursor
        // parado), igual ao binário standalone (window.rs RedrawRequested).
        if self.engine.clip_interaction.is_dragging() {
            let (sx0, sy0) = (self.engine.scroll_x, self.engine.scroll_y);
            self.engine.update_edge_scroll(self.engine.last_cursor);
            self.engine.apply_edge_scroll();
            if (self.engine.scroll_x - sx0).abs() > 0.0001
                || (self.engine.scroll_y - sy0).abs() > 0.0001
            {
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
            eprintln!(
                "[dbg-ptr-down] x={x} y={y} btn={button} scale={scale} -> px={px} py={py} rect=({},{},{},{}) ruler={} pps={} scroll=({},{})",
                b.engine.timeline_rect.0,
                b.engine.timeline_rect.1,
                b.engine.timeline_rect.2,
                b.engine.timeline_rect.3,
                b.engine.ruler_px,
                b.engine.px_per_beat,
                b.engine.scroll_x,
                b.engine.scroll_y,
            );
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
                    // Shift+Click (sem arrasto) = colar clipboard na posição
                    if shift {
                        engine.paste_at_beat(
                            ((px + engine.scroll_x) / engine.px_per_beat.max(0.001)).max(0.0),
                            ((py - engine.ruler_px + engine.scroll_y) / (64.0 * engine.zoom_y.max(0.001)))
                                .floor()
                                .max(0.0) as usize,
                        );
                    } else {
                        engine.handle_primary_button(ButtonState::Pressed, (px, py));
                    }
                }
                2 => {
                    // Botão do meio (middle) = pan (Slint envia middle=2)
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
            if engine.clip_interaction.is_dragging() {
                eprintln!(
                    "[dbg-drag] pos=({px},{py}) live_beat={:.3} track={}",
                    engine.clip_interaction.drag_live_beat,
                    engine.clip_interaction.drag_live_track,
                );
            }
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
                    // Botão do meio (middle) = pan (Slint envia middle=2)
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
        let ui_weak = ui.as_weak();
        ui.on_engine_scroll(move |dx, dy, ctrl, _shift, alt| {
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let px = dx * scale;
            let py = dy * scale;
            let engine = &mut b.engine;
            engine.handle_scroll(px, py, ctrl, alt);
            let sx = engine.scroll_x;
            let slint_sx = ui_weak.upgrade().map(|u| u.get_timeline_scroll_x()).unwrap_or(-1.0);
            eprintln!(
                "[dbg-scroll-host] engine.scroll_x={sx} slint.scroll-x={slint_sx} dx={dx} dy={dy}"
            );
            b.dirty = true;
        });
    }

    // ---- ZOOM VERTICAL ESCRITO PELO SLINT ---------------------------------
    // A engine é a fonte da verdade do zoom vertical: qualquer mudança vinda
    // da UI (track panel) é gravada direto nela; o `tick` espelha de volta.
    {
        let b = bridge.clone();
        ui.on_timeline_zoom_y_changed(move |zy| {
            let mut b = b.borrow_mut();
            b.engine.zoom_y = zy.clamp(0.25, 6.0);
            b.dirty = true;
        });
    }

    // ---- TECLADO -----------------------------------------------------
    // Espaço = play/pause. NÃO é tratado aqui: o atalho vive no
    // `undo-scope` do main.slint e no `KeyAction::TogglePlay` do winit,
    // e os dois chamam o MESMO `toggle-play-routed`, que decide pelo
    // contexto do ponteiro (`pointer-over-beat`). Tratar aqui também
    // causaria double-toggle.
    {
        ui.on_engine_key_event(move |_text, _ctrl, _shift, _alt| {});
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
        engine.init_tools();

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
                bridge.borrow_mut().cursor_logical = (logical.x, logical.y);
                // Rota do play por contexto: com o cursor sobre a popup
                // do beat o espaço comanda o sequenciencer, não a timeline.
                update_pointer_context(ui, logical.x, logical.y);
                overlay_dispatch(bridge, slint::platform::WindowEvent::PointerMoved {
                    position: logical,
                });
            }
            WindowEvent::CursorLeft { .. } => {
                // Saiu da janela: nada está "sobre" a popup, então o
                // play volta a ser o da timeline.
                ui.set_pointer_over_beat(false);
                overlay_dispatch(bridge, slint::platform::WindowEvent::PointerExited);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let st = self.state.borrow();
                let (px, py) = st.last_cursor;
                let scale = st.scale;
                let (ctrl, alt, shift) = (st.ctrl, st.alt, st.shift);
                drop(st);
                let slint_button = match button {
                    winit::event::MouseButton::Left => slint::platform::PointerEventButton::Left,
                    winit::event::MouseButton::Right => slint::platform::PointerEventButton::Right,
                    _ => slint::platform::PointerEventButton::Middle,
                };
                let event = match state {
                    ElementState::Pressed => {
                        // Shift+clique na timeline (sem Ctrl/Alt) = colar clipboard
                        // na posição do cursor. No modo hosted, o Slint não recebe
                        // os modificadores do winit (ModifiersChanged não é roteado
                        // pra ele), então `on_engine_pointer_down` sempre vê
                        // `shift=false`. A gente intercepta AQUI, com os modificadores
                        // reais, e chama o paste direto na engine — o mesmo caminho
                        // que o `on_engine_pointer_down` usa no modo native.
                        if shift && !ctrl && !alt && button == winit::event::MouseButton::Left {
                            let mut b = bridge.borrow_mut();
                            let engine = &mut b.engine;
                            // Só cola se o clique estiver dentro da área da timeline
                            // (abaixo da régua), não dentro de popups/knobs do Slint.
                            if py >= engine.ruler_px {
                                let beat = ((px + engine.scroll_x) / engine.px_per_beat.max(0.001)).max(0.0);
                                let track = ((py - engine.ruler_px + engine.scroll_y)
                                    / (64.0 * engine.zoom_y.max(0.001)))
                                    .floor()
                                    .max(0.0) as usize;
                                engine.shift_pressed = true;
                                engine.paste_at_beat(beat, track);
                                b.dirty = true;
                                // Não despacha para o Slint também: o paste já
                                // manipulou o clique, evitando duplo processamento.
                                return;
                            }
                        }
                        slint::platform::WindowEvent::PointerPressed {
                            position: logical_pos(px as f64, py as f64, scale),
                            button: slint_button,
                        }
                    }
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
                // (§`TogglePlay`) é resolvido AQUI, mas a DECISÃO de o que
                // tocar é do Slint (`toggle-play-routed`), que conhece o
                // contexto do ponteiro: sobre a popup do beat o espaço
                // comanda o step sequencer, em qualquer outro lugar comanda
                // a timeline. O callback do Rust confirma o estado real
                // (`set_playing`/`set_beat_playing` em playback.rs), então
                // os dois botões de play ficam coerentes.
                // Não vai para o Slint como tecla, nem acontece
                // double-toggle — só o routed-toggle resolve.
                let forward_to_slint = {
                    let mut b = bridge.borrow_mut();
                    match b.engine.handle_key(&key) {
                        timeline_render::app::KeyAction::TogglePlay => {
                            ui.invoke_toggle_play_routed();
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
                // Roteia pelo DropSink (os_drop): pré-decode + drop efetivo.
                // O feedback visual NÃO é o ghost do ClipPool e sim o
                // `drop_preview` da engine ("normal da timeline"): ligamos
                // `is_dragging_file` aqui para o `tick` alimentar o preview
                // que segue o cursor real (mesmo caminho do binário
                // standalone). Sem isso o hover mostraria o ghost do pool.
                if crate::importer::is_audio_path(&path) {
                    bridge.borrow_mut().engine.is_dragging_file = true;
                }
                if let Some(sink) = &self.drop_sink {
                    (sink.hovered)(path);
                }
            }
            WindowEvent::HoveredFileCancelled => {
                bridge.borrow_mut().engine.is_dragging_file = false;
                if let Some(sink) = &self.drop_sink {
                    (sink.cancelled)();
                }
            }
            WindowEvent::DroppedFile(path) => {
                // Decide o alvo pelo cursor real (beat → sampler →
                // timeline) — o mesmo fluxo do binário standalone.
                bridge.borrow_mut().engine.is_dragging_file = false;
                if let Some(sink) = &self.drop_sink {
                    (sink.dropped)(path);
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
///
/// O `main` sobe isto numa thread com stack grande (ver `main.rs`): a
/// construção da árvore do Slint em `resumed` é recursiva e estoura os
/// 1 MB da thread principal. O winit recusa criar o event loop fora da
/// thread principal, daí o `any_thread` no Windows.
pub fn run() -> Result<(), slint::PlatformError> {
    let mut builder = EventLoop::with_user_event();
    #[cfg(windows)]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let event_loop = builder.build().map_err(|e| {
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

/// O botão esquerdo do mouse ainda está pressionado?
///
/// Usado como rede de segurança do arrasto: o `PointerEvent::up`
/// do Slint pode se perder (soltar fora da janela, Alt+Tab, perda
/// de foco) e, sem isto, o ghost do ClipPool grudava na tela. Sem
/// Windows não há como perguntar — aí assumimos "pressionado" e o
/// comportamento é o de sempre (o `up` resolve).
fn left_button_is_down() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        const VK_LBUTTON: i32 = 0x01;
        unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0 }
    }
    #[cfg(not(windows))]
    {
        true
    }
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