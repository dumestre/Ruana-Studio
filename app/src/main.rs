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
mod async_sampler;
mod beat;
mod clip_cache;
mod cursors;
mod decode_worker;
mod explorer;
mod floats;
mod icon;
mod importer;
mod interaction;
#[macro_use]
mod mixer;
mod models;
mod os_drop;
mod piano_roll;
mod playback;
mod timeline;
mod theme;
mod timeline_canvas;
mod timeline_gpu;
mod waveform;
mod waveform_gpu;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::cursors::{
    init_cursor_manager, reset_cursor_to_default, set_cursor_for_tool, set_dark_cursors,
    set_timeline_bounds,
};
#[cfg(windows)]
use crate::cursors::set_cursor_window_handle;
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
    #[cfg(windows)]
    {
        let ui_weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
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
        });
    }

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
    // O editor da janela principal é VINCULADO ao clip MIDI ativo: abrir um
    // clip carrega as notas dele e cada edição grava de volta (on_mutated).
    let piano_editor: Rc<RefCell<NoteEditor>> = Rc::new(RefCell::new(NoteEditor::new()));
    let active_midi_clip: Rc<RefCell<Option<u64>>> = Rc::new(RefCell::new(None));
    {
        let project = project.clone();
        let active = active_midi_clip.clone();
        let editor = piano_editor.clone();
        let on_mutated: Rc<dyn Fn()> = Rc::new(move || {
            let Some(id) = *active.borrow() else {
                return;
            };
            let notes = editor.borrow().notes().to_vec();
            project.borrow_mut().set_clip_notes(id, notes);
        });
        piano_roll::wire_piano_roll(&ui, piano_editor.clone(), &on_mutated);
    }

    // Duplo clique em clip MIDI → abre o piano roll dockado e carrega as
    // notas do clip no editor (vínculo clip ↔ piano roll).
    {
        let project = project.clone();
        let active = active_midi_clip.clone();
        let editor = piano_editor.clone();
        let ui_weak = ui.as_weak();
        ui.on_piano_roll_open_requested(move |id| {
            let id = id as u64;
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            // Clip precisa existir e ser MIDI.
            let notes = {
                let proj = project.borrow();
                match proj.clip(id) {
                    Some(c) if c.is_midi() => c.notes().to_vec(),
                    _ => return,
                }
            };
            *active.borrow_mut() = Some(id);
            piano_roll::seed_editor(&editor, &notes);
            piano_roll::refresh_notes(&ui, &editor);
            // Mostra o piano roll dockado (aba 2).
            ui.set_bottom_panel_open(true);
            ui.set_bottom_active_tab(2);
        });
    }

    // Beat sequencer: projeção inicial + persistência no core.
    {
        let project = project.clone();
        beat::wire_beat(&ui, project, audio.clone());
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        let ui_weak = ui.as_weak();
        let sampler_decode = async_sampler::SamplerDecode::default();
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
        let sampler_decode = async_sampler::SamplerDecode::default();
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

    // Motor de zoom/scroll: captura o input cru e suaviza por frame.
    interaction::install(&ui);

    // Waveform: re-render na resolução exata quando o zoom muda.
    waveform::wire_resolution_requests(&ui, waveforms.clone(), wgpu_renderer.clone(), clips_model.clone());

    // TimelineCanvas: fundo da timeline (grid + faixas) via wgpu + clips
    // (corpo/header) lidos direto do core + playhead 60fps + VU meters,
    // render-on-change.
    timeline_canvas::wire(&ui, project.clone(), meter_state.clone());

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

    // Janela sempre abre maximizada (pedido usuário) - PROFISSIONAL.
    // Tenta via Slint (set_maximized) e fallback Win32 ShowWindow.
    {
        // Slint: tenta maximizar via API (se exposta)
        ui.window().set_maximized(true);
        let ui_weak_max = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(50), move || {
            if let Some(ui) = ui_weak_max.upgrade() {
                ui.window().set_maximized(true);
                #[cfg(windows)]
                {
                    use raw_window_handle::HasWindowHandle;
                    use windows_sys::Win32::Foundation::HWND;
                    use windows_sys::Win32::UI::WindowsAndMessaging::{IsZoomed, ShowWindow, SW_MAXIMIZE};
                    if let Ok(handle) = ui.window().window_handle().window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                            unsafe {
                                let hwnd: HWND = h.hwnd.get() as *mut _;
                                if IsZoomed(hwnd) != 0 {
                                    let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // Arrasto da janela frameless - PROFISSIONAL sem tremor.
    // Antes: `set_position(origin + dx logical)` causava jitter em
    // HiDPI (logical vs physical) e trava por flood de mensagens.
    // Agora: delega ao SO via WM_NCLBUTTONDOWN/HTCAPTION (drag nativo
    // do Windows) - suave, com AeroSnap, sem cálculo manual.
    #[cfg(windows)]
    {
        let ui_weak_drag_start = ui.as_weak();
        ui.on_window_drag_started(move || {
            if let Some(ui) = ui_weak_drag_start.upgrade() {
                // Se já está maximizada, ignora arrasto (usuário deve
                // restaurar primeiro) - evita pulo
                let is_max = {
                    use raw_window_handle::HasWindowHandle;
                    use windows_sys::Win32::Foundation::HWND;
                    use windows_sys::Win32::UI::WindowsAndMessaging::IsZoomed;
                    if let Ok(handle) = ui.window().window_handle().window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                            let hwnd: HWND = h.hwnd.get() as *mut _;
                            unsafe { IsZoomed(hwnd) != 0 }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if is_max {
                    return;
                }
                if let Ok(handle) = ui.window().window_handle().window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                        unsafe {
                            use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                HTCAPTION, SendMessageW, WM_NCLBUTTONDOWN,
                            };
                            let hwnd: HWND = h.hwnd.get() as *mut _;
                            SendMessageW(
                                hwnd,
                                WM_NCLBUTTONDOWN,
                                HTCAPTION as WPARAM,
                                0 as LPARAM,
                            );
                        }
                    }
                }
            }
        });
    }

    // Mantido por compatibilidade mas agora é no-op (nativo cuida)
    ui.on_window_drag_moved(|_, _| {});

    ui.on_window_drag_finished(|| {});

    // Drag & drop de arquivos direto do Sistema Operacional.
    os_drop::install(
        &ui,
        project.clone(),
        waveforms.clone(),
        clips_model.clone(),
        track_colors_model.clone(),
        sample_cache.clone(),
    );

    // ---- TEMA ------------------------------------------------------
    // Restaura a paleta salva e aplica quando o usuário escolher
    // outra no seletor do toolbar (persiste o índice selecionado).
    {
        let idx = theme::load().unwrap_or(0);
        theme::apply(&ui, idx);
        // Cursores escuros no tema Paper (claro), claros nos escuros.
        let _ = set_dark_cursors(idx == theme::THEME_PAPER);

        let ui_weak_theme = ui.as_weak();
        ui.on_theme_selected(move |index| {
            if let Some(ui) = ui_weak_theme.upgrade() {
                let idx = theme::apply(&ui, index);
                theme::save(idx);
                let _ = set_dark_cursors(idx == theme::THEME_PAPER);
            }
        });
    }

    ui.run()
}