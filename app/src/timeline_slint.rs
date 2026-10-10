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

use daw_core::clip::{beats_eq, Clip, ClipKind, TimeMode};
use daw_core::track::{Track, TrackKind};
use daw_core::transport::Transport;
use daw_core::Project;
use timeline_render::app::ButtonState;
use timeline_render::clips::{ClipDemo, ClipNote};
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
            id: c.id as u32,
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

            // As notas vão junto porque a timeline DRA o clip MIDI: sem
            // elas o renderer não tem o que desenhar e o clip fica igual a
            // um retângulo vazio — que era como o arrasto do beat sequencer
            // aparecia, com o pattern inteiro sem nenhuma célula ligada.
            notes: if c.is_midi() {
                c.notes
                    .iter()
                    .map(|n| ClipNote {
                        pitch: n.pitch,
                        start: n.start,
                        length: n.length,
                        velocity: n.velocity,
                    })
                    .collect()
            } else {
                Vec::new()
            },

            file_path: c.file_path.clone(),
            source_start_beats: c.source_start_beats,
            // Clip MUSICAL: a onda tem de cobrir a FONTE INTEIRA a partir do
            // trim (é o conteúdo que será esticado para caber no span), e
            // `0` é como o `ClipDemo` pede "até o fim do arquivo" — ver a
            // regra de região em `timeline-render/src/waveform.rs`. Num clip
            // de 2 beats esticado 2x, passar `length_beats` desenharia só
            // metade do conteúdo e silêncio no resto.
            //
            // Clip LINEAR continua com o trecho de duração conhecida: nele o
            // span do clip é que decide quanto do arquivo é ouvido.
            source_length_beats: if c.is_musical() { 0.0 } else { c.length_beats },
            // Palpite inicial. O nº REAL de canais do arquivo substitui
            // este valor em `build_wave_bars_and_segments` assim que o
            // áudio é decodificado — senão um arquivo mono era desenhado
            // como duas faixas L/R, cada uma com metade das colunas.
            channels: if c.is_midi() { 1 } else { 2 },
            // Preenchido pelo decode da onda na engine (`build_wave_bars_and_segments`).
            region_seconds: 0.0,
            warp_markers: c.warp_markers.clone(),
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
        let _ = tx.send(WindowCommand::SetTempoMap(p.tempo_map.changes.clone()));
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
pub fn sync_clips_from_project(engine: &mut TimelineEngine, proj: &Project, bpm: f32) {
    engine.clips = build_clip_demos(proj);

    // Regenera o buffer de barras (min/max interleaved) por clip.
    // `bpm` converte os beats de origem (`source_start_beats`, trim do
    // importer) para frames do arquivo — o mesmo mapeamento que o motor
    // de áudio usa para tocar, mantendo a wave alinhada com o som.
    //
    // A RESOLUÇÃO É FIXA (`WAVE_PPB`), NUNCA o zoom vivo.
    //
    // Aqui passava `engine.px_per_beat` (8..640). Duas consequências ruins:
    //
    //   1. o número de colunas — e portanto o tamanho do bucket de cada
    //      coluna — mudava a cada rebuild, então o MESMO áudio era
    //      reamostrado com buckets de tamanhos diferentes. Como cada
    //      coluna guarda o pico do seu bucket, o envelope desenhado
    //      mudava de AMPLITUDE conforme o zoom/criação. Era isto que
    //      fazia duas cópias do mesmo arquivo parecerem diferentes e o
    //      nível da onda "pular" ao dar zoom;
    //   2. o binário standalone (`timeline-render`) sempre gerou o buffer
    //      nesta referência fixa — é o contrato de `WAVE_PPB` — então o
    //      app renderizava diferente do próprio renderer.
    //
    // Com a referência fixa, o desenho da onda passa a depender só do
    // arquivo e da região de origem: idêntico em qualquer zoom e entre
    // cópias, e o shader escolhe envelope × curva apenas pelo zoom.
    let (bars, segments) = timeline_render::waveform::build_wave_bars_and_segments(
        &mut engine.clips,
        timeline_render::app::WAVE_PPB,
        bpm,
    );
    engine.wave_bars = bars;
    engine.wave_segments = segments;
}

/// Reconcilia a lista COMPLETA de clips da engine (`clips`) de volta para o
/// `Project` (fonte da verdade) após qualquer edição estrutural feita na
/// timeline (mover, deletar, colar, duplicar, desenhar, fatiar, cola).
///
/// Instalado como `engine.clip_write_back` no hosted: sempre que a engine
/// termina uma edição ela entrega a lista inteira de `ClipDemo`. As regras:
///
/// 1. clips do `Project` que NÃO aparecem na engine são apagados
///    (a engine já removeu do desenho);
/// 2. clips com `id != 0` presentes nos dois são reposicionados
///    (track/beat) quando a engine mudou a posição;
/// 3. clips com `id == 0` (criados na engine: paste/duplicate/draw/slice)
///    são adicionados ao `Project`, ganhando id real.
///
/// Sem isso, edições da timeline ficavam SÓ na engine: o áudio (que lê o
/// `Project`) continuava tocando clips apagados/na posição antiga, e um
/// import (que refaz o espelho a partir do `Project`) apagava os clips
/// montados na própria engine.
///
/// `mirrored_ids` = ids que o espelho (tick) já admitiu no engine. Só clips
/// COM esse id são candidatos a delete: um clip recém-importado ainda não
/// forte para o engine até o próximo rebuild, então não pode ser apagado por
/// um write-back disparado no mesmo frame do import.
pub fn reconcile_engine_clips_to_project(
    clips: &[ClipDemo],
    project: &mut Project,
    mirrored_ids: &[u64],
) {
    // 1) Clips do Project que sumiram da engine → apaga do documento.
    //    Restrito a ids que o espelho já conhecia (evita matar um clip
    //    recém-importado cujo rebuild ainda não rodou).
    let engine_ids: Vec<u64> = clips.iter().map(|c| c.id as u64).collect();
    let mut to_delete: Vec<u64> = Vec::new();
    for core in &project.clips {
        if mirrored_ids.contains(&core.id) && !engine_ids.contains(&core.id) {
            to_delete.push(core.id);
        }
    }
    for id in to_delete {
        project.delete_clip(id);
    }

    // 2) Reposicionar clips existentes (id casando entre engine e Project).
    for clip in clips.iter().filter(|c| c.id != 0) {
        let Some(track_id) = project.tracks.get(clip.track_index).map(|t| t.id) else {
            continue;
        };
        // Descobre o clip correspondente no Project por id.
        let Some(core) = project.clips.iter().find(|c| c.id == clip.id as u64) else {
            continue;
        };
        let (core_id, core_track, core_beat, core_len, core_warp, core_mode) = (
            core.id,
            core.track_id,
            core.start_beat,
            core.length_beats,
            core.warp_markers.clone(),
            core.time_mode,
        );
        if core_track != track_id || !beats_eq(core_beat, clip.beat) {
            project.move_clip(core_id, track_id, clip.beat);
        }
        if !beats_eq(core_len, clip.len) && clip.len > 0.0 {
            let _ = project.resize_clip(core_id, clip.beat, clip.len);
        }

        // 2b) Timewarp do clip (free warp): sincroniza os marcadores de
        //     AudioWarp editados na engine. Um clip com marcadores vira
        //     MUSICAL — é o modo em que o motor reestica por trecho; sem isso
        //     os marcadores seriam gravados mas ignorados no playback.
        if core_warp != clip.warp_markers {
            if !clip.warp_markers.is_empty() && core_mode != TimeMode::Musical {
                project.set_clip_time_mode(core_id, TimeMode::Musical);
            }
            project.set_clip_warp_markers(core_id, clip.warp_markers.clone());
        }
    }

    // 3) Clips novos da engine (id == 0) → adiciona ao Project.
    //    Idempotente: se já existe um clip nestas track/beat/len/kind, é o
    //    mesmo que o write-back anterior já persistiu (o rebuild que troca o
    //    id==0 pelo id real ainda não rodou).
    for clip in clips.iter().filter(|c| c.id == 0 && c.len > 0.05) {
        let Some(track_id) = project.tracks.get(clip.track_index).map(|t| t.id) else {
            continue;
        };
        let kind = if clip.is_midi { ClipKind::Midi } else { ClipKind::Audio };
        let already_there = project.clips.iter().any(|c| {
            c.track_id == track_id
                && beats_eq(c.start_beat, clip.beat)
                && beats_eq(c.length_beats, clip.len)
                && (kind == ClipKind::Midi) == c.is_midi()
        });
        if already_there {
            continue;
        }
        let name = clip.name.clone().unwrap_or_default();
        let new_id = if let Some(path) = &clip.file_path {
            project.add_clip_with_source(
                track_id,
                kind,
                name,
                clip.beat,
                clip.len,
                clip.source_start_beats,
                Some(path.clone()),
            )
        } else {
            project.add_clip(track_id, kind, name, clip.beat, clip.len)
        };
        // Preserva a cor do ClipDemo da engine — sem isso, clips duplicados
        // ou importados de patterns MIDI nascem com cor aleatória (baseada
        // no id novo, via (id % 19) + 1 em Clip::new).
        if let Some(id) = new_id {
            project.set_clip_color(id, clip.color as u8);
        }
        // Preserva as notas MIDI — sem isso, clips MIDI duplicados/colados
        // nascem vazios (o piano roll e o motor não têm notas para tocar).
        if kind == ClipKind::Midi && !clip.notes.is_empty() {
            if let Some(id) = new_id {
                let notes: Vec<daw_midi::Note> = clip
                    .notes
                    .iter()
                    .map(|cn| daw_midi::Note {
                        id: 0,
                        pitch: cn.pitch,
                        start: cn.start,
                        length: cn.length,
                        velocity: cn.velocity,
                    })
                    .collect();
                project.set_clip_notes(id, notes);
            }
        }
    }
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
        let bpm = transport.lock().map(|t| t.bpm as f32).unwrap_or(120.0);
        sync_clips_from_project(e, &project.borrow(), bpm);
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
                let bpm = self
                    .transport
                    .lock()
                    .map(|t| t.bpm as f32)
                    .unwrap_or(120.0);
                sync_clips_from_project(engine, &project.borrow(), bpm);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moved_engine_clip_updates_project_without_changing_source_offset() {
        let mut project = Project::default();
        project.tracks.push(Track::new(0, "Track 1", TrackKind::Audio));
        project.tracks.push(Track::new(1, "Track 2", TrackKind::Audio));
        let clip_id = project
            .add_clip_with_source(
                0,
                daw_core::clip::ClipKind::Audio,
                "Audio",
                1.0,
                4.0,
                2.0,
                Some("sample.wav".to_string()),
            )
            .unwrap();
        let mirrored_ids = project.clips.iter().map(|c| c.id).collect::<Vec<_>>();
        let mut clips = build_clip_demos(&project);
        clips[0].beat = 8.0;
        clips[0].track_index = 1;

        reconcile_engine_clips_to_project(&clips, &mut project, &mirrored_ids);

        let clip = project.clip(clip_id).unwrap();
        assert_eq!(clip.start_beat, 8.0);
        assert_eq!(clip.track_id, 1);
        assert_eq!(clip.source_start_beats, 2.0);
    }

    /// O arrasto do "BEAT SEQUENCER" do Clip Pool nascia um clip MIDI com
    /// `notes: []`, porque `DrumPattern::new` monta a grade toda desligada
    /// (`crates/daw-midi/src/pattern.rs:80`). A timeline desenhava esse
    /// clip como um retângulo chapado — indistinguível de um placeholder.
    ///
    /// Este teste trava o elo que faltava: as notas do `Project` precisam
    /// chegar no `ClipDemo`, senão o renderer não tem o que desenhar.
    #[test]
    fn build_clip_demos_carrega_as_notas_do_clip_midi() {
        let mut project = Project::default();
        let track_id = project.tracks.first().map(|t| t.id).unwrap_or(0);

        let mut com_notas = Clip::new(
            1,
            track_id,
            "COM NOTAS",
            ClipKind::Midi,
            0.0,
            4.0,
        );
        com_notas.notes = vec![
            daw_midi::Note {
                id: 1,
                pitch: 36,
                start: 0.0,
                length: 0.25,
                velocity: 110,
            },
            daw_midi::Note {
                id: 2,
                pitch: 42,
                start: 1.0,
                length: 0.25,
                velocity: 90,
            },
        ];

        let sem_notas = Clip::new(
            2,
            track_id,
            "VAZIO",
            ClipKind::Midi,
            4.0,
            4.0,
        );

        let audio = Clip::new(
            3,
            track_id,
            "AUDIO",
            ClipKind::Audio,
            8.0,
            4.0,
        );

        project.clips.push(com_notas);
        project.clips.push(sem_notas);
        project.clips.push(audio);

        let demos = build_clip_demos(&project);

        let com = demos
            .iter()
            .find(|d| d.name.as_deref() == Some("COM NOTAS"))
            .expect("clip MIDI com notas");
        let vazio = demos
            .iter()
            .find(|d| d.name.as_deref() == Some("VAZIO"))
            .expect("clip MIDI sem notas");
        let audio = demos
            .iter()
            .find(|d| d.name.as_deref() == Some("AUDIO"))
            .expect("clip de audio");

        assert_eq!(com.notes.len(), 2);
        assert_eq!(com.notes[0].pitch, 36);
        assert_eq!(com.notes[0].length, 0.25);
        assert_eq!(com.notes[1].pitch, 42);

        // Sem notas é honesto: o clip aparece vazio.
        assert!(vazio.notes.is_empty());

        // Áudio nunca carrega notas, mesmo que sobre.
        assert!(audio.notes.is_empty());
    }
}
