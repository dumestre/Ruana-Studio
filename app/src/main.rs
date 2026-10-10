//! RuanaStudio — entrypoint.
//!
//! Arquitetura do AGENTS.md: o app é dono do loop winit e do wgpu.
//! O `main` apenas sobe o event loop do winit (`host::run`); o `setup`
//! monta o estado compartilhado (core + áudio + modelos) e delega o
//! wiring dos handlers para os módulos por domínio quando a janela sobe:
//!
//! - `host`       — janela winit + surface wgpu + overlay Slint offscreen
//! - `waveform`   — cache, rasterização e re-render por zoom
//! - `importer`   — importação de arquivos de áudio
//! - `explorer`   — explorador de arquivos (navegação/filtro)
//! - `mixer`      — mixer (estado, modelos compartilhados)
//! - `models`     — projeção core → Slint e eventos do documento
//! - `timeline`   — edição de clips na timeline
//! - `floats`     — modo sem janelas flutuantes (UI única em overlay)
//! - `os_drop`    — drag & drop de arquivos do SO (no nível winit)
//!
//! A timeline wgpu renderiza DIRETO na surface (GPU→GPU, via
//! `EngineBridge::tick` → `render_timeline_bg`); a UI Slint é desenhada
//! pelo backend Skia sobre wgpu-30 na MESMA device e composta por cima como
//! overlay (o componente Timeline vira um "buraco").

slint::include_modules!();

mod audio_bridge;
mod async_sampler;
mod beat;
mod clip_cache;
mod cursors;
mod decode_worker;
mod explorer;
mod floats;
mod host;
mod icon;
mod importer;
mod interaction;
mod meters;
#[macro_use]
mod mixer;
mod models;
mod os_drop;
mod playback;
mod slint_host;
mod stretch_worker;
mod theme;

mod timeline;
mod timeline_slint;

mod waveform;
mod waveform_gpu;
mod timewarp;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::cursors::{
    init_cursor_manager, set_cursor_for_tool, set_dark_cursors, set_timeline_bounds,
};
#[cfg(windows)]
use crate::cursors::set_cursor_window_handle;
use crate::audio_bridge::AudioBridge;
use crate::mixer::MixerModels;
use crate::meters::MeterPump;
use crate::models::{bars_for_end_beat, bind_document_events, initial_bind};
use crate::waveform::WaveformCache;
use daw_core::mixer::MixerParam;
use daw_core::project::Project;
use daw_core::transport::Transport;
use raw_window_handle::HasWindowHandle;
use slint::Model;
#[cfg(windows)]
use timeline_render::tools::{init_tool_manager, set_hwnd as set_tools_hwnd};

#[cfg(windows)]
fn choose_audio_file() -> Option<String> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::core::{PCWSTR, PWSTR};
    let filter: Vec<u16> = "Audio Files\0*.wav;*.mp3;*.flac;*.ogg;*.oga\0All Files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut path = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW::default();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = HWND::default();
    dialog.lpstrFilter = PCWSTR::from(filter.as_ptr());
    dialog.lpstrFile = PWSTR::from(path.as_mut_ptr());
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if unsafe { GetOpenFileNameW(&mut dialog) } != 0 {
        let len = path.iter().position(|c| *c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&path[..len]))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn choose_audio_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Audio Files", &["wav", "mp3", "flac", "ogg", "oga"])
        .add_filter("All Files", &["*"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Stack do event loop. A construção da árvore do Slint é recursiva e cada
/// elemento com `drop-shadow-*` acrescenta um nível; a UI atual (knob,
/// toolbar, transport, mixer) estoura os 1 MB da thread principal do Windows
/// em `MainWindow::new()`.
const MAIN_STACK_SIZE: usize = 32 * 1024 * 1024;

fn main() -> Result<(), slint::PlatformError> {
    // No macOS o winit exige que o event loop rode na thread principal.
    #[cfg(target_os = "macos")]
    {
        host::run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let event_loop = std::thread::Builder::new()
            .name("ruana-main".into())
            .stack_size(MAIN_STACK_SIZE)
            .spawn(|| {
                if let Err(e) = host::run() {
                    eprintln!("[main] event loop encerrou com erro: {e}");
                }
            })
            .expect("main: nao foi possivel iniciar a thread do event loop");

        match event_loop.join() {
            Ok(()) => Ok(()),
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic desconhecido".to_string());
                eprintln!("[main] thread do event loop morreu: {msg}");
                std::process::exit(1);
            }
        }
    }
}

/// Roteia o duplo clique na timeline para o editor do clip.
///
/// - **MIDI** → piano roll da engine, carregando as notas reais do clip.
/// - **Áudio** → sampler mostrando o ARQUIVO DO CLIP, com decode em thread
///   de fundo. Clip sem `file_path` cai no sample da linha do beat (o mesmo
///   que o botão "SP" mostra).
///
/// Id que o `Project` não conhece (clip ainda sem espelho) é ignorado.
fn open_clip_editor(
    ui: &MainWindow,
    project: &Rc<RefCell<Project>>,
    audio: &AudioBridge,
    sampler_decode: &async_sampler::SamplerDecode,
    ui_weak: &slint::Weak<MainWindow>,
    id: u64,
) {
    let (is_midi, track_index, file_path) = {
        let proj = project.borrow();
        let Some(clip) = proj.clip(id) else {
            return;
        };
        let track_index = proj.tracks.iter().position(|t| t.id == clip.track_id);
        (
            clip.is_midi(),
            track_index,
            clip.file_path.clone().filter(|p| !p.is_empty()),
        )
    };

    if is_midi {
        return;
    }

    let Some(track_index) = track_index else {
        return;
    };
    ui.set_beat_selected_track(track_index as i32);
    ui.set_bottom_panel_open(true);
    ui.set_bottom_active_tab(1);

    let Some(path) = file_path else {
        beat::open_sampler_for_track(ui, project, track_index);
        return;
    };
    let path_buf = std::path::PathBuf::from(&path);
    let name = path_buf
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Clip")
        .to_string();
    sampler_decode.schedule(
        ui_weak,
        project.clone(),
        audio.clone(),
        track_index as i32,
        path_buf,
        async_sampler::SamplerSource::Clip,
        name,
    );
}

/// Instala todo o wiring da DAW quando a janela winit sobe (`resumed` no
/// host). Devolve o [`host::DropSink`] com o fluxo do OS drag & drop
/// (o software window do Slint não recebe eventos nativos de drop).
pub fn setup(
    ui: &MainWindow,
    window: Arc<winit::window::Window>,
    bridge: Rc<RefCell<host::EngineBridge>>,
    project: Rc<RefCell<Project>>,
    transport: Arc<Mutex<Transport>>,
    state: Rc<RefCell<host::SharedState>>,
) -> host::DropSink {
    // Modo nativo (AGENTS.md): a timeline do Slint vira o "buraco" por onde
    // o wgpu é composto; a UI do Slint é o overlay que flutua por cima.
    ui.set_native_overlay(true);

    init_cursor_manager().expect("Failed to initialize cursor manager");

    // Cursor customizado: HWND da janela winit (já existe sincronamente — sem
    // depender de timer que esperava a janela nativa do Slint subir).
    #[cfg(windows)]
    {
        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as *mut std::ffi::c_void;
                set_cursor_window_handle(hwnd);
                // Também inicializa o ToolManager (cursores das ferramentas) com o mesmo HWND
                let _ = init_tool_manager();
                set_tools_hwnd(hwnd);
            }
        }
    }

    // Estado autoritativo da DAW: vive no core (`project`) e no Transport
    // compartilhado — criados pelo host e passados para cá.

    // Motor de áudio: abre UMA vez e fica aberto pela vida do app.
    //
    // Antes o stream nascia no play e morria no stop. Isso custava
    // ~100ms de handshake com o driver no início de cada play (o
    // primeiro compasso saía picado) e tornava impossível pré-escutar
    // sample com o transporte parado — que é justamente o que o
    // channel rack do FL Studio faz. Se o dispositivo não abrir, o
    // app continua rodando em silêncio em vez de morrer.
    let audio_engine = match daw_audio::AudioEngine::start(transport.clone()) {
        Ok(engine) => {
            let cfg = engine.output_config();
            eprintln!("[audio] {} Hz, {} canais", cfg.sample_rate, cfg.channels);
            Some(engine)
        }
        Err(err) => {
            eprintln!("[audio] dispositivo indisponível: {err} — seguindo em silêncio");
            None
        }
    };
    let audio = audio_bridge::AudioBridge::new(audio_engine.as_ref().map(|e| e.handle()));

    // O `audio_engine` precisa sobreviver ao retorno desta função: se cair,
    // a stream do CPAL fecha e o Transport nunca avança (playhead preso).
    bridge.borrow_mut().audio_engine = audio_engine;

    // Qualquer evento de documento (clip criado, movido, cortado,
    // apagado, undo/redo) muda o que a thread RT precisa ouvir.
    // Assinar aqui é melhor que marcar sujo em cada call site: um
    // caminho novo de edição não pode nascer mudo por esquecimento.
    {
        let audio = audio.clone();
        project.borrow().subscribe(move |_event, _kind| {
            audio.mark_dirty();
        });
    }

    // Republica o projeto na thread RT quando algo muda (coalescido).
    bridge.borrow_mut().audio_sync_timer = Some(audio_bridge::start_sync_timer(
        audio.clone(),
        project.clone(),
    ));

    // Cache de waveforms (LRU): clip_id → CachedWave (LODs min/max).
    const WAVEFORM_CACHE_CAP: usize = 256;
    let waveforms: Rc<RefCell<WaveformCache>> = Rc::new(RefCell::new(crate::waveform::WaveformCache::new(WAVEFORM_CACHE_CAP)));

    // Renderer de waveform via wgpu (headless/offscreen + readback).
    // Criado UMA vez aqui na main thread e reutilizado por todos os clips.
    // Se o adapter falhar, a waveform simplesmente não é desenhada e o app
    // segue rodando (Option).
    let wgpu_renderer: Option<Rc<crate::waveform_gpu::WgpuRenderer>> = {
        match crate::waveform_gpu::WgpuRenderer::new() {
            Ok(r) => {
                eprintln!("[waveform_gpu] wgpu renderer initialized");
                Some(Rc::new(r))
            }
            Err(e) => {
                eprintln!("[waveform_gpu] Failed to init wgpu renderer: {e}");
                None
            }
        }
    };

    // Cache de samples em RAM (path → decodificado + waveform):
    // alimenta colagem instantânea (Ctrl+C / Shift+clique) e o
    // pré-decode durante o drag do SO.
    let sample_cache: Rc<RefCell<clip_cache::SampleCache>> = Rc::new(RefCell::new(HashMap::new()));

    let (tracks_model, clips_model, track_colors_model) =
        initial_bind(ui, &project.borrow(), &waveforms.borrow());
    let pool_clips_model = {
        let pool: Vec<crate::ClipData> = (0..clips_model.row_count())
            .filter_map(|i| clips_model.row_data(i))
            .collect();
        let pattern_names: Vec<String> = project
            .borrow()
            .beat_patterns
            .patterns
            .iter()
            .map(|pattern| pattern.name.clone())
            .collect();
        let pool_model = Rc::new(slint::VecModel::from(
            crate::models::build_pool_clips(&pool, &pattern_names),
        ));
        ui.set_pool_clips(pool_model.clone().into());
        pool_model
    };
    bind_document_events(
        &project.borrow(),
        tracks_model,
        clips_model.clone(),
        pool_clips_model,
        waveforms.clone(),
        ui.as_weak(),
    );

    // Métrica musical: espelha o core na UI (fonte única — sem
    // hardcodes nas telas) e dimensiona a timeline pelo projeto.
    {
        let proj = project.borrow();
        let (bpm, ppq, bpb) = {
            let t = transport.lock().unwrap();
            (t.bpm as i32, t.ppq as i32, t.beats_per_bar as i32)
        };
        ui.set_ppq(ppq);
        ui.set_beats_per_bar(bpb);
        ui.set_bpm(bpm);
        let end = proj
            .clips
            .iter()
            .map(|c| c.end_beat())
            .fold(0.0f32, f32::max);
        ui.set_total_bars(bars_for_end_beat(end as f64, bpb));
    }

    // Mixer: modelos compartilhados — atualização por canal (set_row_data)
    // propaga para a janela principal (sem flutuantes nesta arquitetura).
    let mixer_models: Rc<RefCell<MixerModels>> =
        Rc::new(RefCell::new(MixerModels::from_project(&project.borrow())));
    apply_mixer_models!(ui, &mixer_models.borrow());

    // Meters: um único par de modelos para o mixer E o track panel, mais
    // o estado de suavização (ataque/decaimento/peak-hold). O bombeamento
    // é feito por `playback::install` a partir do pico que a engine
    // publica — não há mais VU simulado.
    let meter_pump: Rc<RefCell<MeterPump>> = Rc::new(RefCell::new(MeterPump::new(
        project.borrow().tracks.len() + 1,
    )));
    audio.set_meter_channels(project.borrow().tracks.len() + 1);
    {
        // Os `Rc<VecModel>` são clonado para dentro da janela e o pump
        // fica com os originais: os dois lados escrevem/lêem o MESMO
        // modelo, então resize e repintura andam juntos.
        let pump = meter_pump.borrow();
        ui.set_channel_meters(pump.models().values.clone().into());
        ui.set_channel_clipped(pump.models().clipped.clone().into());
    }

    // Beat sequencer: projeção inicial + persistência no core.
    {
        let project = project.clone();
        beat::wire_beat(ui, project, audio.clone(), transport.clone());
    }

    // Filtro de carga do sampler compartilhado pelos três caminhos (diálogo,
    // drop e duplo clique no clip de áudio): uma única fila de decodes.
    let sampler_decode = async_sampler::SamplerDecode::default();

    {
        let project = project.clone();
        let audio = audio.clone();
        let ui_weak = ui.as_weak();
        let sampler_decode = sampler_decode.clone();
        ui.on_sampler_load_requested(move |track| {
            let Some(path) = choose_audio_file() else {
                return;
            };
            // NÃO reescrever o mesmo sample: se a track alvo já tem
            // este MESMO arquivo carregado, ignora o pedido. Só uma
            // carga com nome/arquivo diferente substitui.
            {
                let proj = project.borrow();
                if let Some(pattern) = proj.current_beat_pattern() {
                    if let Some(params) = pattern.track_params.get(track.max(0) as usize) {
                        if params.sample_path.as_deref() == Some(path.as_str()) {
                            return;
                        }
                    }
                }
            }
            let path_buf = std::path::PathBuf::from(&path);
            let name = path_buf
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Sample")
                .to_string();
            // Decode em thread de fundo: a UI não congela em MP3/FLAC
            // longos — o apply (painel + pattern + preview) roda no
            // drainer quando o resultado chega.
            sampler_decode.schedule(
                &ui_weak,
                project.clone(),
                audio.clone(),
                track,
                path_buf,
                async_sampler::SamplerSource::Load,
                name,
            );
        });
    }

    // Drop de áudio no sampler (explorer → sampler ou SO → sampler).
    {
        let project = project.clone();
        let audio = audio.clone();
        let ui_weak = ui.as_weak();
        let sampler_decode = sampler_decode.clone();
        ui.on_debug_log(move |msg| {
            eprintln!("[slint] {msg}");
        });
        ui.on_sampler_sample_dropped(move |name, path| {
            eprintln!("[drop] sampler-sample-dropped name={name} path={path}");
            if path.is_empty() {
                return;
            }
            let path_buf = std::path::PathBuf::from(&path);
            let display_name = if name.is_empty() {
                path_buf
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Sample")
                    .to_string()
            } else {
                name.to_string()
            };
            // Decode em thread de fundo; o apply grava na linha
            // SELECIONADA na hora em que o resultado chega.
            sampler_decode.schedule(
                &ui_weak,
                project.clone(),
                audio.clone(),
                -1,
                path_buf,
                async_sampler::SamplerSource::Drop,
                display_name,
            );
        });
    }

    // Duplo clique num clip da timeline → abre o editor daquele clip. A engine
    // só sinaliza o id (ela roda com o bridge emprestado); o roteamento é
    // daqui, que tem `AudioBridge` para o decode do sampler.
    {
        let project = project.clone();
        let audio = audio.clone();
        let sampler_decode = sampler_decode.clone();
        let ui_weak = ui.as_weak();
        bridge.borrow_mut().open_clip_handler = Some(Rc::new(move |id| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            open_clip_editor(&ui, &project, &audio, &sampler_decode, &ui_weak, id);
        }));
    }

    // Modo single-window (AGENTS.md): sem janelas flutuantes — os handles
    // continuam existindo para os módulos que os pedem.
    let handles = floats::install_no_floats(
        ui,
        project.clone(),
        mixer_models.clone(),
        audio.clone(),
    );

    // Timeline: edição de clips (drop, slice, glue, move/resize,
    // delete/rename/cor, clipboard, duplicate, add-track, undo/redo).
    timeline::install(
        ui,
        project.clone(),
        waveforms.clone(),
        clips_model.clone(),
        track_colors_model.clone(),
        sample_cache.clone(),
        handles.apply_mixer_state.clone(),
        handles.fit_float_mixer_windows.clone(),
    );

    // Motor de zoom/scroll: captura o input cru e suaviza por frame.
    interaction::install(ui);

    // Engine wgpu da timeline: callbacks do componente `Timeline` (régua,
    // pointer, scroll) operando na engine da surface — composta a cada
    // `RedrawRequested` pelo host.
    host::install_engine_callbacks(ui, bridge.clone());

    // Modelos para sincronização reversa no drag & drop do SO: a engine
    // importa o clip (mesmo caminho do binário standalone) e estes modelos
    // permitem persistir no Project e refletir no Slint.
    {
        let mut b = bridge.borrow_mut();
        b.waveforms = Some(waveforms.clone());
        b.clips_model = Some(clips_model.clone());
        b.track_colors_model = Some(track_colors_model.clone());
    }

    // Waveform: re-render na resolução exata quando o zoom muda.
    waveform::wire_resolution_requests(ui, waveforms.clone(), wgpu_renderer.clone(), clips_model.clone());

    // Explorador de arquivos: navegação, histórico e filtro.
    explorer::wire_explorer(ui);

    // Parâmetros do mixer → core (atualização por canal, sem rebuild).
    {
        let project = project.clone();
        let mixer_models = mixer_models.clone();
        let audio = audio.clone();
        ui.on_mixer_param_changed(move |channel, param, value| {
            apply_channel_param(&project, &mixer_models, &audio, channel, param, value);
        });
    }

    // Parâmetros do CABEÇALHO da faixa → core.
    //
    // Antes estes botões não tinham destino: o M acendia na tela e a
    // faixa continuava tocando. Agora passam pelo mesmo
    // `apply_channel_param` do mixer, então as duas telas não podem
    // divergir — mudar o M no mixer move o botão do cabeçalho e vice
    // versa, porque as duas leem o mesmo modelo.
    {
        let project = project.clone();
        let mixer_models = mixer_models.clone();
        let audio = audio.clone();
        ui.on_track_param_changed(move |channel, param, value| {
            apply_channel_param(&project, &mixer_models, &audio, channel, param, value);
        });
    }

    // Clique na luz de clip (mixer, cabeçalho da faixa e flutuantes).
    {
        let audio = audio.clone();
        ui.on_channel_clip_cleared(move |channel| {
            audio.clear_meter_clip(channel.max(0) as usize);
        });
    }

    // Inserts do mixer (janela principal) → core.
    {
        let project_add = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_insert_added(move |channel| {
            project_add.borrow_mut().add_mixer_insert(channel as usize);
            apply_mixer();
        });
        let project_rm = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_insert_removed(move |channel, slot| {
            project_rm
                .borrow_mut()
                .remove_mixer_insert(channel as usize, slot as usize);
            apply_mixer();
        });
        let project_tg = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_insert_toggled(move |channel, slot| {
            project_tg
                .borrow_mut()
                .toggle_mixer_insert(channel as usize, slot as usize);
            apply_mixer();
        });
    }

    // Copiar/colar a cadeia de inserts do canal selecionado no mixer.
    {
        let clipboard: Rc<RefCell<Option<Vec<daw_core::mixer::InsertSlot>>>> =
            Rc::new(RefCell::new(None));
        let copy_clipboard = clipboard.clone();
        let project_copy = project.clone();
        ui.on_mixer_copy_slots(move |channel| {
            let channel = channel.max(0) as usize;
            let inserts = {
                let project = project_copy.borrow();
                if channel < project.tracks.len() {
                    Some(project.tracks[channel].mixer.inserts.clone())
                } else if channel == project.tracks.len() {
                    Some(project.master_mixer.inserts.clone())
                } else {
                    None
                }
            };
            *copy_clipboard.borrow_mut() = inserts;
        });

        let paste_clipboard = clipboard;
        let project_paste = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_paste_slots(move |channel| {
            let Some(inserts) = paste_clipboard.borrow().clone() else { return; };
            if project_paste.borrow_mut().set_mixer_inserts(channel.max(0) as usize, inserts) {
                apply_mixer();
            }
        });
    }

    // O histórico do projeto também controla as mudanças nos slots do mixer.
    {
        let project_undo = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_undo_requested(move || {
            if project_undo.borrow_mut().undo() {
                apply_mixer();
            }
        });
        let project_redo = project.clone();
        let apply_mixer = handles.apply_mixer_state.clone();
        ui.on_mixer_redo_requested(move || {
            if project_redo.borrow_mut().redo() {
                apply_mixer();
            }
        });
    }

    {
        let transport = transport.clone();
        ui.on_metronome_state_changed(move |enabled| {
            if let Ok(mut t) = transport.lock() {
                t.metronome_enabled = enabled;
            }
        });
    }
    {
        let transport = transport.clone();
        ui.on_metronome_sound_changed(move |sound| {
            if let Ok(mut t) = transport.lock() {
                t.metronome_sound = sound.clamp(0, 4) as u8;
            }
        });
    }
    {
        let transport = transport.clone();
        ui.on_metronome_volume_changed(move |value| {
            if let Ok(mut t) = transport.lock() {
                t.metronome_volume = value.clamp(0.0, 1.0);
            }
        });
    }

    // Undo / redo (Ctrl+Z / Ctrl+Y) → histórico do core.
    {
        let p = project.clone();
        ui.on_undo_requested(move || {
            p.borrow_mut().undo();
        });
        let p = project.clone();
        ui.on_redo_requested(move || {
            p.borrow_mut().redo();
        });
    }

    // Playback/seek/meters: módulo dedicado (app/src/playback.rs).
    playback::install(
        &ui.as_weak(),
        transport.clone(),
        project.clone(),
        meter_pump.clone(),
        audio.clone(),
    );

    ui.on_tool_changed(move |tool| {
        let _ = set_cursor_for_tool(tool);
    });

    {
        let transport = transport.clone();
        ui.on_loop_state_changed(move |enabled| {
            if let Ok(mut t) = transport.lock() {
                t.loop_enabled = enabled;
            }
        });
    }
    {
        let transport = transport.clone();
        ui.on_loop_range_changed(move |start, end| {
            if let Ok(mut t) = transport.lock() {
                t.loop_start_ticks = (start.max(0.0) * t.ppq as f32) as u64;
                t.loop_end_ticks = (end.max(start + 0.01) * t.ppq as f32) as u64;
            }
        });
    }

    // F: foco/zoom do clip selecionado; F novamente restaura a viewport.
    let focus_restore: Rc<RefCell<Option<(f32, f32, f32)>>> = Rc::new(RefCell::new(None));
    let clips_for_focus = clips_model.clone();
    let ui_weak_focus = ui.as_weak();
    ui.on_focus_selected_clip_requested(
        move |id, old_zoom, old_scroll_x, old_scroll_y, view_width| {
            let Some(ui) = ui_weak_focus.upgrade() else {
                return;
            };
            let mut restore = focus_restore.borrow_mut();
            if let Some((zoom, sx, sy)) = restore.take() {
                ui.set_zoom_level(zoom);
                ui.set_timeline_scroll_x(sx);
                ui.set_timeline_scroll_y(sy);
                return;
            }
            let Some(row) = (0..clips_for_focus.row_count())
                .find_map(|i| clips_for_focus.row_data(i).filter(|c| c.id == id))
            else {
                return;
            };
            *restore = Some((old_zoom, old_scroll_x, old_scroll_y));
            let zoom = old_zoom.max(2.5);
            let center = (row.beat + row.len * 0.5) * 40.0 * zoom;
            ui.set_zoom_level(zoom);
            ui.set_timeline_scroll_x((center - view_width * 0.5).max(0.0));
            ui.set_timeline_scroll_y(old_scroll_y);
        },
    );

    ui.on_timeline_bounds_changed(move |x, y, w, h| {
        set_timeline_bounds(x, y, w, h);
    });

    ui.on_window_close(move || {
        std::process::exit(0);
    });

    // Minimizar → janela winit (sem janela nativa do Slint para mexer).
    {
        let window = window.clone();
        ui.on_minimize(move || {
            window.set_minimized(true);
        });
    }

    // Janela já abre maximizada no `host::resumed` (winit).

    // Arrasto da janela frameless - PROFISSIONAL sem tremor.
    // Delega ao SO via WM_NCLBUTTONDOWN/HTCAPTION (drag nativo do
    // Windows) - suave, com AeroSnap, sem cálculo manual.
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HTCAPTION, IsZoomed, SendMessageW, WM_NCLBUTTONDOWN,
        };
        let window = window.clone();
        ui.on_window_drag_started(move || {
            let Ok(handle) = window.window_handle() else {
                return;
            };
            let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() else {
                return;
            };
            let hwnd: HWND = h.hwnd.get() as *mut _;
            // Se já está maximizada, ignora arrasto (usuário deve
            // restaurar primeiro) - evita pulo.
            if unsafe { IsZoomed(hwnd) != 0 } {
                return;
            }
            unsafe {
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0 as LPARAM);
            }
        });
    }

    // Mantido por compatibilidade mas agora é no-op (nativo cuida)
    ui.on_window_drag_moved(|_, _| {});

    ui.on_window_drag_finished(|| {});

    // ---- TEMA ------------------------------------------------------
    // Restaura a paleta salva e aplica quando o usuário escolher
    // outra no seletor do toolbar (persiste o índice selecionado).
    // A timeline wgpu tem paleta própria (`engine.theme`) — espelha o
    // índice aqui e força um novo frame para ela repintar na hora.
    {
        let idx = theme::load().unwrap_or(0);
        theme::apply(ui, idx);
        {
            let mut b = bridge.borrow_mut();
            b.engine.theme = idx as usize;
            b.dirty = true;
            b.engine.request_redraw();
        }
        // Cursores escuros no tema Paper (claro), claros nos escuros.
        let _ = set_dark_cursors(idx == theme::THEME_PAPER);

        let ui_weak_theme = ui.as_weak();
        let bridge_theme = bridge.clone();
        ui.on_theme_selected(move |index| {
            if let Some(ui) = ui_weak_theme.upgrade() {
                let idx = theme::apply(&ui, index);
                theme::save(idx);
                let _ = set_dark_cursors(idx == theme::THEME_PAPER);
                let mut b = bridge_theme.borrow_mut();
                b.engine.theme = idx as usize;
                b.dirty = true;
                b.engine.request_redraw();
            }
        });
    }

    // ---- OS DRAG & DROP (nível winit) --------------------------------
    // O software window do Slint não recebe eventos nativos de drop; o host
    // reencaminha via `window_event` para estes closures. O fluxo é idêntico
    // ao do drag do ClipPool (ghost, snap, pré-decode, importação).
    {
        let ui_weak_hovered = ui.as_weak();
        let sample_cache_h = sample_cache.clone();
        let hovered = Rc::new(move |path: std::path::PathBuf| {
            let Some(ui) = ui_weak_hovered.upgrade() else {
                return;
            };
            os_drop::handle_hovered(&ui, &path, &sample_cache_h);
        });

        let ui_weak_cancelled = ui.as_weak();
        let cancelled = Rc::new(move || {
            if let Some(ui) = ui_weak_cancelled.upgrade() {
                os_drop::handle_hovered_cancelled(&ui);
            }
        });

        let ui_weak_dropped = ui.as_weak();
        let project_d = project.clone();
        let waveforms_d = waveforms.clone();
        let clips_model_d = clips_model.clone();
        let track_colors_model_d = track_colors_model.clone();
        let sample_cache_d = sample_cache.clone();
        let state_d = state.clone();
        let dropped = Rc::new(move |path: std::path::PathBuf| {
            let Some(ui) = ui_weak_dropped.upgrade() else {
                return;
            };
            let st = state_d.borrow();
            os_drop::handle_dropped(
                &ui,
                &path,
                st.win_pos,
                st.scale,
                project_d.clone(),
                waveforms_d.clone(),
                clips_model_d.clone(),
                track_colors_model_d.clone(),
                sample_cache_d.clone(),
            );
        });

        host::DropSink {
            hovered,
            cancelled,
            dropped,
        }
    }
}

/// Aplica um parâmetro de canal vindo da UI no core e nos modelos.
///
/// O mixer e o cabeçalho da faixa chamam esta mesma função. Ter as duas
/// rotas separadas foi como o M do cabeçalho passou a desenhar um estado
/// que o core não tinha: o botão era local à `TrackHeaderRow` e nada
/// confirmava a mudança.
///
/// `value` no volume é a POSIÇÃO do fader (0..1 na curva dB), convertida
/// aqui para o ganho linear que o core e a engine consomem. Sem esta
/// conversão o volume arrastado viraria ganho linear direto, e o fader
/// responderia de forma não-linear — quase todo o curso preso perto do
/// fundo.
fn apply_channel_param(
    project: &Rc<RefCell<Project>>,
    mixer_models: &Rc<RefCell<MixerModels>>,
    audio: &AudioBridge,
    channel: i32,
    param: slint::SharedString,
    value: f32,
) {
    let Some(param) = MixerParam::from_str(param.as_str()) else {
        return;
    };
    if channel < 0 {
        return;
    }
    let channel = channel as usize;
    let core_value = match param {
        MixerParam::Volume => crate::meters::fader_pos_to_linear(value),
        _ => value,
    };
    if !project
        .borrow_mut()
        .set_mixer_param(channel, param, core_value)
    {
        return;
    }
    // O modelo guarda a posição, não o ganho: quem lê é o triângulo e o
    // LCD, e os dois falam em posição.
    mixer_models.borrow().update_channel(channel, param, value);
    audio.mark_dirty();
}
