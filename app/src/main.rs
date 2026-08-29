//! RuanaStudio — entrypoint.
//!
//! O `main` fica deliberadamente enxuto: cria a janela, monta o
//! estado compartilhado (core + áudio + modelos) e delega o
//! wiring dos handlers para os módulos por domínio:
//!
//! - `waveform`   — cache, rasterização e re-render por zoom
//! - `importer`   — importação de arquivos de áudio
//! - `explorer`   — explorador de arquivos (navegação/filtro)
//! - `piano_roll` — piano roll (grid, ações de notas)
//! - `mixer`      — mixer (estado, modelos compartilhados)
//! - `models`     — projeção core → Slint e eventos do documento
//! - `timeline`   — edição de clips na timeline
//! - `floats`     — janelas flutuantes (mixer/sampler/piano roll)
//! - `os_drop`    — drag & drop de arquivos do Sistema Operacional

slint::include_modules!();

mod audio_bridge;
mod beat;
mod clip_cache;
mod cursors;
mod explorer;
mod floats;
mod icon;
mod importer;
#[macro_use]
mod mixer;
mod models;
mod os_drop;
mod piano_roll;
mod playback;
mod timeline;
mod waveform;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::cursors::{
    init_cursor_manager, reset_cursor_to_default, set_cursor_for_tool, set_cursor_window_handle,
    set_timeline_bounds,
};
use crate::mixer::MixerModels;
use crate::models::{bars_for_end_beat, bind_document_events, initial_bind};
use crate::waveform::WaveformCache;
use daw_core::mixer::MixerParam;
use daw_core::project::Project;
use daw_core::transport::Transport;
use daw_midi::NoteEditor;
use raw_window_handle::HasWindowHandle;
use slint::Model;

#[cfg(windows)]
fn choose_audio_file() -> Option<String> {
    use std::mem::size_of;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows::core::PCWSTR;
    let filter: Vec<u16> = "Audio Files\0*.wav;*.mp3;*.flac;*.ogg;*.oga\0All Files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut path = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW::default();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = HWND::default();
    dialog.lpstrFilter = PCWSTR(filter.as_ptr());
    dialog.lpstrFile = windows::core::PWSTR(path.as_mut_ptr());
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        let len = path.iter().position(|c| *c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&path[..len]))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn choose_audio_file() -> Option<String> {
    None
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    init_cursor_manager().expect("Failed to initialize cursor manager");

    let window = ui.window();
    window.on_close_requested(move || {
        let _ = reset_cursor_to_default();
        slint::CloseRequestResponse::HideWindow
    });

    // Ícone da janela/barra de tarefas: aplica já na criação e de novo
    // após o event loop subir (a janela winit pode ser criada tardiamente).
    icon::apply(window);
    {
        let ui_weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
            if let Some(ui) = ui_weak.upgrade() {
                icon::apply(ui.window());
            }
        });
    }

    // Cursor customizado: precisa do HWND da janela já criada.
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        #[cfg(windows)]
        {
            if let Some(ui) = ui_weak.upgrade() {
                let window = ui.window();
                let handle = window.window_handle();
                if let Ok(raw_handle) = handle.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(h) = raw_handle.as_raw() {
                        set_cursor_window_handle(h.hwnd.get() as *mut std::ffi::c_void);
                        let _ = set_cursor_for_tool(0);
                    }
                }
            }
        }
    });

    // Estado autoritativo da DAW: vive no core, nunca na UI.
    let project = Rc::new(RefCell::new(Project::startup()));
    let transport = Arc::new(Mutex::new(Transport::default()));

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
    let _audio_sync = audio_bridge::start_sync_timer(audio.clone(), project.clone());

    // Cache de waveforms: clip_id → (picos, imagem pré-renderizada).
    let waveforms: Rc<RefCell<WaveformCache>> = Rc::new(RefCell::new(HashMap::new()));

    // Cache de samples em RAM (path → decodificado + waveform):
    // alimenta colagem instantânea (Ctrl+C / Shift+clique) e o
    // pré-decode durante o drag do SO.
    let sample_cache: Rc<RefCell<clip_cache::SampleCache>> = Rc::new(RefCell::new(HashMap::new()));

    let (tracks_model, clips_model, track_colors_model) =
        initial_bind(&ui, &project.borrow(), &waveforms.borrow());
    bind_document_events(
        &project.borrow(),
        tracks_model,
        clips_model.clone(),
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
    // propaga para a janela principal e todas as flutuantes de uma vez.
    let mixer_models: Rc<RefCell<MixerModels>> =
        Rc::new(RefCell::new(MixerModels::from_project(&project.borrow())));
    apply_mixer_models!(&ui, &mixer_models.borrow());

    // Meters: modelo compartilhado + estado de decay (smoothing no Rust).
    let meter_model: Rc<slint::VecModel<f32>> =
        Rc::new(slint::VecModel::from(vec![
            0.0;
            project.borrow().tracks.len() + 1
        ]));
    ui.set_mixer_meters(meter_model.clone().into());
    let meter_state: Rc<RefCell<Vec<f32>>> =
        Rc::new(RefCell::new(vec![0.0; project.borrow().tracks.len() + 1]));

    // Piano roll da janela principal (estado por janela; cada float tem o seu).
    piano_roll::wire_piano_roll(&ui, Rc::new(RefCell::new(NoteEditor::new())));

    // Beat sequencer: projeção inicial + persistência no core.
    {
        let project = project.clone();
        beat::wire_beat(&ui, project, audio.clone());
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        let ui_weak = ui.as_weak();
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
            let Ok(decoded) = daw_media::waveform::decode_audio(&path_buf) else {
                return;
            };
            let peaks = daw_media::waveform::waveform_points(&decoded, 512);
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let name = path_buf
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Sample");
            let format = path_buf
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            ui.set_sampler_sample_name(name.into());
            ui.set_sampler_sample_loaded(true);
            ui.set_sampler_sample_format(format.into());
            ui.set_sampler_sample_duration(format!("{:.2}s", decoded.duration_seconds).into());
            ui.set_sampler_sample_peaks(Rc::new(slint::VecModel::from(peaks)).into());

            let mut proj = project.borrow_mut();
            if let Some(pattern) = proj.current_beat_pattern_mut() {
                let index = track.max(0) as usize;
                if let Some(params) = pattern.track_params.get_mut(index) {
                    params.sample_path = Some(path.clone());
                    params.name = name.to_string();
                }
            }
            audio.preload(&path);
            audio.mark_dirty();
            beat::refresh_beat(&ui, &proj);
            drop(proj);
            audio.preview(&path, 0.9, 0.0);
        });
    }

    // Drop de áudio no sampler (explorer → sampler ou SO → sampler).
    {
        let project = project.clone();
        let audio = audio.clone();
        let ui_weak = ui.as_weak();
        ui.on_debug_log(move |msg| {
            eprintln!("[slint] {msg}");
        });
        ui.on_sampler_sample_dropped(move |name, path| {
            eprintln!("[drop] sampler-sample-dropped name={name} path={path}");
            if path.is_empty() {
                return;
            }
            let path_buf = std::path::PathBuf::from(&path);
            let Ok(decoded) = daw_media::waveform::decode_audio(&path_buf) else {
                return;
            };
            let peaks = daw_media::waveform::waveform_points(&decoded, 512);
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let display_name = if name.is_empty() {
                path_buf
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Sample")
                    .to_string()
            } else {
                name.to_string()
            };
            let format = path_buf
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            ui.set_sampler_sample_name(display_name.clone().into());
            ui.set_sampler_sample_loaded(true);
            ui.set_sampler_sample_format(format.into());
            ui.set_sampler_sample_duration(format!("{:.2}s", decoded.duration_seconds).into());
            ui.set_sampler_sample_peaks(Rc::new(slint::VecModel::from(peaks)).into());

            let mut proj = project.borrow_mut();
            if let Some(pattern) = proj.current_beat_pattern_mut() {
                // Drop no sampler grava na LINHA SELECIONADA do beat —
                // cada linha com sample tem o seu sampler.
                let index = ui.get_beat_selected_track().max(0) as usize;
                if let Some(params) = pattern.track_params.get_mut(index) {
                    params.sample_path = Some(path.to_string());
                    params.name = display_name;
                }
            }
            audio.preload(&path);
            audio.mark_dirty();
            beat::refresh_beat(&ui, &proj);
            drop(proj);
            audio.preview(&path, 0.9, 0.0);
        });
    }

    // Janelas flutuantes (mixer/sampler/piano roll): wiring dedicado.
    let handles = floats::install(
        &ui,
        project.clone(),
        mixer_models.clone(),
        meter_model.clone(),
        audio.clone(),
    );

    // Timeline: edição de clips (drop, slice, glue, move/resize,
    // delete/rename/cor, clipboard, duplicate, add-track, undo/redo).
    timeline::install(
        &ui,
        project.clone(),
        waveforms.clone(),
        clips_model.clone(),
        track_colors_model.clone(),
        sample_cache.clone(),
        handles.apply_mixer_state.clone(),
        handles.fit_float_mixer_windows.clone(),
    );

    // Waveform: re-render na resolução exata quando o zoom muda.
    waveform::wire_resolution_requests(&ui, waveforms.clone(), clips_model.clone());

    // Explorador de arquivos: navegação, histórico e filtro.
    explorer::wire_explorer(&ui);

    // Parâmetros do mixer → core (atualização por canal, sem rebuild).
    {
        let project = project.clone();
        let mixer_models = mixer_models.clone();
        let audio = audio.clone();
        ui.on_mixer_param_changed(move |channel, param, value| {
            let Some(param) = MixerParam::from_str(param.as_str()) else {
                return;
            };
            let channel = channel as usize;
            project.borrow_mut().set_mixer_param(channel, param, value);
            mixer_models.borrow().update_channel(channel, param, value);
            audio.mark_dirty();
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
        meter_model.clone(),
        meter_state.clone(),
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

    let ui_weak_min = ui.as_weak();
    ui.on_minimize(move || {
        if let Some(ui) = ui_weak_min.upgrade() {
            let window = ui.window();
            window.set_minimized(true);
        }
    });

    // Drag & drop de arquivos direto do Sistema Operacional.
    os_drop::install(
        &ui,
        project.clone(),
        waveforms.clone(),
        clips_model.clone(),
        track_colors_model.clone(),
        sample_cache.clone(),
    );

    ui.run()
}
