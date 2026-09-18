//! Ponte do app ↔ engine wgpu da timeline (`timeline-render`).
//!
//! Estratégia de renderização (profissional):
//!   - **Nativo**: `install` tenta abrir a timeline numa janela winit própria
//!     (thread dedicada) onde o wgpu apresenta DIRETO na surface — zero
//!     readback, performance total de GPU. O app espelha o `Project`
//!     (clips/bpm, read-only) via `WindowCommand` e compartilha o `Transport`.
//!   - **Fallback offscreen**: se a janela nativa não abrir, `install_offscreen`
//!     desenha headless e publica `slint::Image` no canvas do `timeline.slint`.
//!
//! A edição (write-back dos clips para o `Project`) é a próxima etapa; por
//! enquanto a engine recebe o projeto apenas para exibição.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, SharedPixelBuffer, Timer, TimerMode};

use daw_core::transport::Transport;
use daw_core::Project;
use timeline_render::app::ButtonState;
use timeline_render::clips::ClipDemo;
use timeline_render::App as TimelineEngine;
use timeline_render::WindowCommand;

use crate::MainWindow;

/// Espelha os clips do core em `ClipDemo` para a engine. Reduz o custo de
/// decodificar áudio: o `build_wave_bars_and_segments` da engine decodifica
/// a wave real quando o clip carrega o `file_path` do disco (mesmo caminho
/// do binário standalone `timeline-render`); sem ele, cai para a wave
/// procedural placeholder.
pub fn build_clip_demos(proj: &Project) -> Vec<ClipDemo> {
    let track_index = |track_id: u64| -> usize {
        proj.tracks
            .iter()
            .position(|t| t.id == track_id)
            .unwrap_or(0)
    };

    proj.clips
        .iter()
        .map(|c| ClipDemo {
            track_index: track_index(c.track_id),
            beat: c.start_beat,
            len: c.length_beats.max(0.05),
            is_midi: c.is_midi(),
            color: c.color as usize,
            name: Some(c.name.clone()),
            seam_beat: None,
            pre_glue_len: None,
            pre_glue_color: None,
            wave: Vec::new(),
            file_path: c.file_path.clone(),
            source_start_beats: c.source_start_beats,
            source_length_beats: c.length_beats,
            channels: if c.is_midi() { 1 } else { 2 },
        })
        .collect()
}

/// Instala a timeline. Tenta a janela nativa wgpu (render direto); se a
/// abertura falhar, cai no bridge offscreen (canvas do `timeline.slint`).
///
/// Nota: o wiring atual do app usa `host::install_engine_callbacks` (janela
/// winit própria do processo). Este caminho fica para fallback/legado.
#[allow(dead_code)]
pub fn install(ui: &MainWindow, transport: Arc<Mutex<Transport>>, project: Rc<RefCell<Project>>) {
    if spawn_native_window(transport.clone(), project.clone()) {
        return;
    }
    eprintln!(
        "[timeline_render] janela nativa indisponível — usando fallback offscreen"
    );
    install_offscreen(ui, transport, project);
}

/// Sobe a thread da janela nativa da timeline (winit + wgpu, surface direta)
/// e passa a enviar o espelho de clips/bpm via `WindowCommand`.
fn spawn_native_window(transport: Arc<Mutex<Transport>>, project: Rc<RefCell<Project>>) -> bool {
    let (tx, rx) = mpsc::channel::<WindowCommand>();
    let transport_thread = transport.clone();
    let spawned = std::thread::Builder::new()
        .name("timeline-window".into())
        .spawn(move || {
            let _ = timeline_render::run_native_window(transport_thread, rx);
        })
        .map(|_handle| {
            eprintln!("[timeline_render] janela nativa iniciada (thread dedicada)");
            () 
        })
        .is_ok();
    if !spawned {
        return false;
    }

    // Estado inicial + re-espelho a cada mudança de documento.
    let push = |p: &Project, tx: &mpsc::Sender<WindowCommand>| {
        let _ = tx.send(WindowCommand::SetClips(build_clip_demos(p)));
        let _ = tx.send(WindowCommand::SetBpm(p.tempo_map.bpm_at(0.0)));
    };
    push(&project.borrow(), &tx);
    let project_rc = project.clone();
    project.borrow().subscribe(move |_event, _kind| {
        push(&project_rc.borrow(), &tx);
    });
    true
}

/// Estado interno da ponte offscreen (vive numa `Rc<RefCell>` capturada pelos closures).
struct TimelineBridge {
    engine: Option<TimelineEngine>,
    transport: Arc<Mutex<Transport>>,
    scale: f32,
    clips_dirty: bool,
    dirty: bool,
}

/// Constrói os clips da engine a partir do projeto do core (read-only).
pub fn sync_clips_from_project(engine: &mut TimelineEngine, proj: &Project) {
    engine.clips = build_clip_demos(proj);

    // Regenera o buffer de barras (min/max interleaved) por clip.
    let (bars, segments) =
        timeline_render::waveform::build_wave_bars_and_segments(&mut engine.clips, engine.px_per_beat);
    engine.wave_bars = bars;
    engine.wave_segments = segments;
}

/// Sincroniza clips recém-importados pela engine (via drag OS) de volta para o
/// `Project` do core e para os modelos do Slint.
///
/// Usado no modo hosted: a engine tem acesso direto ao sistema de arquivos e
/// decodifica os áudios sozinha. Ao detectar novos clips em `engine.clips`,
/// o host chama esta função para persistir no documento autoritativo.
pub fn sync_engine_clips_to_project(
    new_clips: &[timeline_render::clips::ClipDemo],
    bpm: f32,
    project: &Rc<RefCell<daw_core::Project>>,
    waveforms: &Rc<RefCell<crate::waveform::WaveformCache>>,
    clips_model: &Rc<slint::VecModel<crate::ClipData>>,
    track_colors_model: &Rc<slint::VecModel<i32>>,
    ui: &crate::MainWindow,
) {
    let bpm_f = if bpm > 0.0 { bpm } else { 120.0 };
    // Cache vazio — esses clips vieram da engine que já decodificou,
    // mas import_audio_file_with_cache vai decodificar de novo do disco
    // (simples e correto; decode é rápido comparado com drag latency).
    let empty_cache = RefCell::new(std::collections::HashMap::new());

    for clip in new_clips {
        let path_str = match &clip.file_path {
            Some(p) => p.clone(),
            None => continue,
        };
        let path = std::path::Path::new(&path_str);
        let _ = crate::importer::import_audio_file_with_cache(
            path,
            clip.track_index as i32,
            clip.beat,
            bpm_f,
            project.as_ref(),
            waveforms.as_ref(),
            clips_model,
            track_colors_model,
            &empty_cache,
        );
    }

    // Atualiza total_bars na UI.
    let end = {
        let proj = project.borrow();
        proj.clips.iter().map(|c| c.end_beat()).fold(0.0f32, f32::max)
    };
    let bpb = ui.get_beats_per_bar();
    let bars = ((end / bpb as f32).ceil() as i32 + 4).max(32);
    ui.set_total_bars(bars);
}

/// Cria a engine offscreen e liga todos os canais com o componente Slint
/// (canvas do `timeline.slint`). Usado quando a janela nativa não abre.
#[allow(dead_code)]
fn install_offscreen(
    ui: &MainWindow,
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
) {
    let scale = ui.window().scale_factor();
    let mut engine = match TimelineEngine::new(None, (320, 200), Some(transport.clone())) {
        Ok(e) => {
            eprintln!("[timeline_render] engine headless inicializada");
            Some(e)
        }
        Err(err) => {
            eprintln!("[timeline_render] engine indisponível: {err}");
            None
        }
    };
    if let Some(e) = engine.as_mut() {
        sync_clips_from_project(e, &project.borrow());
    }

    let bridge = Rc::new(RefCell::new(TimelineBridge {
        engine,
        transport,
        scale,
        clips_dirty: true,
        dirty: true,
    }));

    // Qualquer mudança de documento (clip criado/movido/apagado, undo/redo)
    // marca o espelho como sujo — o timer coalesce o rebuild + re-render.
    {
        let weak = Rc::downgrade(&bridge);
        project.borrow().subscribe(move |_event, _kind| {
            if let Some(b) = weak.upgrade() {
                b.borrow_mut().clips_dirty = true;
            }
        });
    }

    // Tamanho/estado do canvas mudou → re-renderiza uma vez.
    {
        let b = bridge.clone();
        ui.on_timeline_canvas_render_requested(move || {
            b.borrow_mut().dirty = true;
        });
    }

    // ---- PONTEIRO: DOWN ----------------------------------------------
    {
        let b = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_engine_pointer_down(move |x, y, button, ctrl, shift, alt| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let Some(engine) = b.engine.as_mut() else { return };
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
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let Some(engine) = b.engine.as_mut() else { return };
            engine.handle_cursor_moved((px, py));

            // Durante interações contínuas (drag/marquee/pan/scrub) o render
            // é imediato; senão apenas marca sujo (coalescido no timer).
            let busy = engine.panning
                || engine.scrubbing
                || engine.loop_drag.is_some()
                || engine.clip_interaction.is_dragging()
                || engine.clip_interaction.is_marqueeing()
                || engine.clip_interaction.is_drawing()
                || engine.clip_interaction.is_seam_hovered()
                || engine.fade_interaction.is_dragging();
            if busy {
                b.submit(&ui);
                b.publish(&ui);
            } else {
                b.dirty = true;
            }
        });
    }

    // ---- PONTEIRO: UP ------------------------------------------------
    {
        let b = bridge.clone();
        ui.on_engine_pointer_up(move |x, y, button| {
            let mut b = b.borrow_mut();
            let scale = b.scale;
            let (px, py) = (x * scale, y * scale);
            let Some(engine) = b.engine.as_mut() else { return };
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
            let Some(engine) = b.engine.as_mut() else { return };
            engine.handle_scroll(px, py, ctrl, alt);
            b.dirty = true;
        });
    }

    // ---- TECLADO -----------------------------------------------------
    // Mínimo nesta entrega: Espaço = play/pause (mesma rota do botão da UI).
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

    // ---- TIMER DE MANUTENÇÃO (40 Hz) ---------------------------------
    // Segue o playhead do transporte, espelha clips sujos e re-renderiza.
    {
        let b = bridge.clone();
        let ui_weak = ui.as_weak();
        let project = project.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(25), move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut b = b.borrow_mut();
            b.tick(&ui, &project);
        });
        // Mantém o timer vivo enquanto a ponte existir.
        std::mem::forget(timer);
    }
    // TODO: substituir o `mem::forget` acima por guardar o Timer no bridge
    // quando o modelo de ciclo de vida da ponte fechar (hoje `install` nunca
    // retorna; o app só sai via window-close → process::exit).
    // _timer_guard = timer, mas ainda não há um lugar para guardar.

    ui.window().request_redraw();
}

impl TimelineBridge {
    /// Rotina periódica: re-espelha o projeto, segue o playhead e renderiza
    /// se a cena estiver suja.
    fn tick(&mut self, ui: &MainWindow, project: &Rc<RefCell<Project>>) {
        if self.clips_dirty {
            self.clips_dirty = false;
            if let Some(engine) = self.engine.as_mut() {
                sync_clips_from_project(engine, &project.borrow());
            }
            self.dirty = true;
        }

        let (playing, pos) = {
            let t = self.transport.lock().unwrap();
            (t.playing, t.position_seconds() as f32)
        };

        if let Some(engine) = self.engine.as_mut() {
            if engine.playing != playing {
                engine.playing = playing;
                self.dirty = true;
            }
            if playing {
                engine.update_playhead();
                self.dirty = true;
            } else if (engine.transport_seconds - pos).abs() > 0.00005 {
                engine.transport_seconds = pos;
                self.dirty = true;
            }
        }

        if self.dirty {
            self.dirty = false;
            self.submit(ui);
        }
        self.publish(ui);
    }

    /// Submete um frame offscreen sem bloquear (o readback chega depois via
    /// [`Self::publish`]). Se o GPU ainda não coletou o frame anterior, o
    /// frame é descartado e `dirty` volta a ficar pendente.
    fn submit(&mut self, ui: &MainWindow) {
        let Some(engine) = self.engine.as_mut() else { return };
        let w_log = ui.get_timeline_canvas_area_width();
        let h_log = ui.get_timeline_canvas_area_height();
        if w_log <= 0.0 || h_log <= 0.0 {
            return;
        }
        let w = ((w_log * self.scale).round()).clamp(1.0, 8192.0) as u32;
        let h = ((h_log * self.scale).round()).clamp(1.0, 8192.0) as u32;

        if !engine.submit_frame(w, h) {
            // Não conseguiu submeter neste tick; tenta de novo na próxima.
            self.dirty = true;
        }
    }

    /// Coleta o readback do último frame já pronto e publica como
    /// `slint::Image`. Não-bloqueante (poll de 0ms por tick).
    fn publish(&mut self, ui: &MainWindow) {
        let Some(engine) = self.engine.as_mut() else { return };
        let Some((w, h, rgba)) = engine.take_rgba() else { return };
        let buf = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, w, h);
        ui.set_timeline_canvas_background(slint::Image::from_rgba8(buf));
    }
}