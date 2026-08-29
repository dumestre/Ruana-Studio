//! Bridge do beat sequencer: persistência no core.
//!
//! Core → UI: `refresh_beat` re-projeta patterns e parâmetros das
//! trilhas no global `BeatState`.
//!
//! UI → Core: os callbacks do global gravam no `Project` e disparam
//! `refresh_beat`, mantendo o core como fonte única de verdade.
//!
//! Áudio: toda mutação marca o snapshot como sujo (ver
//! [`crate::audio_bridge`]), e os gestos que o usuário espera ouvir —
//! soltar um sample, ligar um passo, clicar no botão da trilha —
//! disparam pré-escuta na hora, com o transporte parado ou não.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel};

use daw_core::project::Project;
use daw_midi::{DrumPattern, DrumTrackParams};

use crate::audio_bridge::AudioBridge;
use crate::{BeatState, MainWindow};

/// Cache global (sessão) do SAMPLER por sample path.
///
/// Toda linha do beat com sample é um instrumento; o painel Sampler
/// mostra o instrumento da linha selecionada. Decodificar o arquivo a
/// cada clique de seleção seria lento — cacheamos aqui a "vista"
/// (nome/formato/duração/peaks), sempre derivada do MESMO decoder que
/// alimenta o engine.
struct SamplerViewData {
    name: String,
    format: String,
    duration: String,
    peaks: Vec<f32>,
}

static SAMPLE_VIEW_CACHE: std::sync::OnceLock<
    Mutex<HashMap<String, Arc<SamplerViewData>>>,
> = std::sync::OnceLock::new();

fn sample_cache() -> &'static Mutex<HashMap<String, Arc<SamplerViewData>>> {
    SAMPLE_VIEW_CACHE.get_or_init(Default::default)
}

/// Nome curto de um sample a partir do caminho.
fn sample_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Decodifica (uma vez por path) e devolve a vista do sampler.
fn sampler_view_data(path: &str) -> Option<Arc<SamplerViewData>> {
    if let Some(hit) = sample_cache()
        .lock()
        .ok()
        .and_then(|m| m.get(path).cloned())
    {
        return Some(hit);
    }
    let path_buf = std::path::PathBuf::from(path);
    let decoded = daw_media::waveform::decode_audio(&path_buf).ok()?;
    let name = path_buf
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Sample")
        .to_string();
    let format = path_buf
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();
    let duration = format!("{:.2}s", decoded.duration_seconds);
    let peaks = daw_media::waveform::waveform_points(&decoded, 512);
    let entry = Arc::new(SamplerViewData {
        name,
        format,
        duration,
        peaks,
    });
    if let Ok(mut map) = sample_cache().lock() {
        map.insert(path.to_string(), entry.clone());
    }
    Some(entry)
}

/// Limpa o painel Sampler (linha sem sample / sem padrão).
fn clear_sampler(ui: &MainWindow) {
    ui.set_sampler_sample_name(SharedString::default());
    ui.set_sampler_sample_loaded(false);
    ui.set_sampler_sample_format(SharedString::default());
    ui.set_sampler_sample_duration(SharedString::default());
    ui.set_sampler_sample_peaks(Rc::new(VecModel::<f32>::default()).into());
}

/// Sincroniza o painel Sampler com a linha selecionada do beat.
///
/// Cada linha do beat com sample tem UM sampler: é este painel,
/// apontando para a linha selecionada. Chamado quando a seleção da
/// linha muda e no fim de `refresh_beat` (drop/clear refletem na hora).
fn sync_sampler_to_beat(ui: &MainWindow, project: &Project) {
    let index = ui.get_beat_selected_track().max(0) as usize;
    let path = project
        .current_beat_pattern()
        .and_then(|p| p.track_params.get(index))
        .and_then(|t| t.sample_path.clone());
    let Some(path) = path else {
        clear_sampler(ui);
        return;
    };
    let Some(view) = sampler_view_data(&path) else {
        clear_sampler(ui);
        return;
    };
    ui.set_sampler_sample_name(view.name.clone().into());
    ui.set_sampler_sample_loaded(true);
    ui.set_sampler_sample_format(view.format.clone().into());
    ui.set_sampler_sample_duration(view.duration.clone().into());
    ui.set_sampler_sample_peaks(Rc::new(VecModel::from(view.peaks.clone())).into());
}

/// Aplica uma mutação no projeto e re-projeta o beat na sequência.
///
/// O snapshot do áudio é marcado como sujo aqui: qualquer coisa que
/// muda o pattern muda o que a thread RT precisa ver.
fn commit(
    ui: &MainWindow,
    project: &RefCell<Project>,
    audio: &AudioBridge,
    f: impl FnOnce(&mut Project),
) {
    f(&mut project.borrow_mut());
    audio.mark_dirty();
    refresh_beat(ui, &project.borrow());
}

/// Re-projeta o estado do beat sequencer (core) nos models da UI.
pub fn refresh_beat(ui: &MainWindow, project: &Project) {
    let st = BeatState::get(ui);
    let bp = &project.beat_patterns;

    // patterns: cada trilha vira um array plano de bools
    // (índice global = track * steps + step).
    let rows: Vec<ModelRc<bool>> = bp
        .patterns
        .iter()
        .map(|p| Rc::new(VecModel::from(p.grid.concat())).into())
        .collect();
    st.set_patterns(Rc::new(VecModel::from(rows)).into());

    // Nomes dos patterns para o Clip Pool.
    let pattern_names: Vec<SharedString> =
        bp.patterns.iter().map(|p| p.name.clone().into()).collect();
    st.set_pattern_names(Rc::new(VecModel::from(pattern_names)).into());

    // Durações em beats (para o Clip Pool criar o clip com o
    // comprimento correspondente ao arrastar o padrão pra timeline).
    let pattern_durations: Vec<f32> =
        bp.patterns.iter().map(|p| p.length_beats()).collect();
    st.set_pattern_durations(Rc::new(VecModel::from(pattern_durations)).into());

    let Some(current) = bp.current_pattern() else {
        // Sem padrão → sampler limpo.
        sync_sampler_to_beat(ui, project);
        return;
    };

    st.set_current_pattern(bp.current as i32);
    st.set_grid_resolution(current.resolution as i32);
    st.set_humanize(current.humanize);
    st.set_swing(current.swing);

    let names: Vec<SharedString> = current
        .track_params
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    st.set_instrument_names(Rc::new(VecModel::from(names)).into());

    // Sample de cada linha: "" quando vazia. A linha precisa mostrar
    // que tem som carregado — renomear a trilha não é sinal suficiente
    // (uma linha renomeada à mão pareceria carregada).
    let samples: Vec<SharedString> = current
        .track_params
        .iter()
        .map(|t| {
            t.sample_path
                .as_deref()
                .map(sample_label)
                .unwrap_or_default()
                .into()
        })
        .collect();
    st.set_track_sample_names(Rc::new(VecModel::from(samples)).into());

    let active: Vec<bool> = current.track_params.iter().map(|t| t.active).collect();
    let solo: Vec<bool> = current.track_params.iter().map(|t| t.solo).collect();
    let pan: Vec<f32> = current.track_params.iter().map(|t| t.pan).collect();
    let volume: Vec<f32> = current.track_params.iter().map(|t| t.volume).collect();
    let channels: Vec<i32> = current
        .track_params
        .iter()
        .map(|t| t.mixer_channel)
        .collect();

    st.set_track_active(Rc::new(VecModel::from(active)).into());
    st.set_track_solo(Rc::new(VecModel::from(solo)).into());
    st.set_track_pan(Rc::new(VecModel::from(pan)).into());
    st.set_track_volume(Rc::new(VecModel::from(volume)).into());
    st.set_track_mixer_channel(Rc::new(VecModel::from(channels)).into());

    // Por fim, o SAMPLER da linha selecionada: qualquer mudança no
    // beat (drop de sample, clear, delete...) reflete na hora.
    sync_sampler_to_beat(ui, project);
}

/// Toca o sample de uma linha do pattern.
///
/// Respeita volume/pan da linha, então a pré-escuta soa como vai soar
/// no pattern — não em ganho unitário.
fn preview_row(project: &RefCell<Project>, audio: &AudioBridge, track: i32) {
    if track < 0 {
        return;
    }
    let proj = project.borrow();
    let Some(t) = proj
        .beat_patterns
        .current_pattern()
        .and_then(|p| p.track_params.get(track as usize))
    else {
        return;
    };
    let Some(path) = t.sample_path.as_deref() else {
        return;
    };
    audio.preview(path, t.volume, t.pan);
}

/// Conecta todos os callbacks do beat sequencer ao core.
pub fn wire_beat(ui: &MainWindow, project: Rc<RefCell<Project>>, audio: AudioBridge) {
    let st = BeatState::get(ui);

    // Snapshot para Copy Beat / Copy Track (paste consumirá quando
    // a UI ganhar a ação).
    let beat_clipboard: Rc<RefCell<Option<DrumPattern>>> = Rc::new(RefCell::new(None));
    let track_clipboard: Rc<RefCell<Option<(Vec<bool>, DrumTrackParams)>>> =
        Rc::new(RefCell::new(None));

    // =========================================================
    // PASSOS
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_step_committed(move |index, value| {
            let mut row = 0usize;
            commit(&ui, &project, &audio, |proj| {
                let stride = proj
                    .current_beat_pattern()
                    .map(|p| p.steps())
                    .unwrap_or(16)
                    .max(1);
                row = index as usize / stride;
                proj.set_beat_step(row, index as usize % stride, value);
            });

            // Ligar um passo toca o som da linha na hora (FL Studio faz
            // isso). Desligar não toca nada.
            if value {
                preview_row(&project, &audio, row as i32);
            }
        });
    }

    // =========================================================
    // PADRÃO ATUAL / RESOLUÇÃO / HUMANIZE / SWING
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_pattern_changed(move |index| {
            if index < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.select_beat_pattern(index as usize);
            });
        });
    }

    // =========================================================
    // SELECTOR DE LINHA → SAMPLER
    //
    // Cada linha do beat com sample tem UM sampler (o painel da aba
    // 1, apontando para a linha selecionada). Selecionar a linha
    // sincroniza o painel com o instrumento daquela track.
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_track_selected(move |index| {
            ui.set_beat_selected_track(index);
            sync_sampler_to_beat(&ui, &project.borrow());
        });
    }

    // Botão "SP" da linha: abre a aba do sampler mostrando a linha.
    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_sampler_open(move |index| {
            ui.set_beat_selected_track(index);
            sync_sampler_to_beat(&ui, &project.borrow());
            ui.set_bottom_panel_open(true);
            ui.set_bottom_active_tab(1);
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_grid_resolution_committed(move |resolution| {
            if resolution <= 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.set_beat_resolution(resolution as u32);
            });
        });
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_humanize_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.humanize = value.clamp(0.0, 1.0);
            }
            audio.mark_dirty();
        });
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_swing_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.swing = value.clamp(0.0, 1.0);
            }
            audio.mark_dirty();
        });
    }

    // =========================================================
    // BEAT (padrão inteiro)
    // =========================================================

    {
        let clipboard = beat_clipboard.clone();
        let project = project.clone();
        st.on_copy_beat(move || {
            *clipboard.borrow_mut() = project.borrow().current_beat_pattern().cloned();
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_duplicate_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.duplicate_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_clear_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.clear_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_delete_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.delete_beat_pattern();
            });
        });
    }

    // =========================================================
    // TRILHAS
    // =========================================================

    {
        let clipboard = track_clipboard.clone();
        let project = project.clone();
        st.on_copy_track(move |track| {
            let proj = project.borrow();
            let i = track.max(0) as usize;
            if let Some(p) = proj.current_beat_pattern() {
                if i < p.num_tracks() {
                    *clipboard.borrow_mut() = Some((p.grid[i].clone(), p.track_params[i].clone()));
                }
            }
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_duplicate_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.duplicate_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_delete_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.delete_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_clear_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.clear_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_insert_pattern(move |_track, interval| {
            if interval <= 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.insert_beat_steps(interval as usize);
            });
        });
    }

    // =========================================================
    // PARÂMETROS DE TRILHA (active/solo/pan/volume/channel)
    //
    // Mudam continuamente (knobs), então gravam direto sem
    // re-projeção — o binding global ↔ popup já reflete na tela.
    // =========================================================

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_track_param_committed(move |track, param, value| {
            if track < 0 {
                return;
            }
            if let Some(t) = project.borrow_mut().beat_track_params_mut(track as usize) {
                match param.as_str() {
                    "active" => t.active = value >= 0.5,
                    "solo" => t.solo = value >= 0.5,
                    "pan" => t.pan = value.clamp(-1.0, 1.0),
                    "volume" => t.volume = value.clamp(0.0, 1.5),
                    "channel" => t.mixer_channel = value.round().clamp(0.0, 15.0) as i32,
                    _ => {}
                }
            }
            audio.mark_dirty();
        });
    }

    // =========================================================
    // DROP DE SAMPLE NA TRILHA (drag & drop do browser)
    //
    // Persiste o path no core e renomeia a trilha com o nome do
    // arquivo (mesmo comportamento do channel rack do FL Studio).
    // =========================================================

    {
        let win = ui.clone_strong();
        let handler = win.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        win.on_beat_sample_dropped(move |track, _name, path| {
            eprintln!("[drop] beat-sample-dropped track={track} path={path}");
            if track < 0 {
                return;
            }
            // Seleciona a linha alvo: o sampler mostra o sample da
            // linha selecionada, então soltar numa linha deve trazer
            // o sampler dela para a frente (feedback visual imediato,
            // como arrastar para um canal frente no FL).
            handler.set_beat_selected_track(track);
            commit(&handler, &project, &audio, |proj| {
                let Some(t) = proj.beat_track_params_mut(track as usize) else {
                    return;
                };
                t.sample_path = if path.is_empty() {
                    None
                } else {
                    Some(path.to_string())
                };

                if let Some(stem) = std::path::Path::new(path.as_str())
                    .file_stem()
                    .and_then(|n| n.to_str())
                {
                    t.name = stem.to_string();
                }
            });

            // "arrasta e reproduz": soltar o sample toca ele na hora,
            // com o transporte parado ou não. O decode acontece aqui
            // (thread da UI) e fica no cache, então quando o passo
            // disparar a thread RT já encontra o buffer pronto.
            if !path.is_empty() {
                audio.preload(path.as_str());
                preview_row(&project, &audio, track);
            }
        });
    }

    // =========================================================
    // PRÉ-ESCUTA (clique no botão da trilha)
    //
    // Mesma coisa que clicar no canal do channel rack do FL:
    // audiciona a linha sem mexer no pattern.
    // =========================================================

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_preview_track(move |track| {
            preview_row(&project, &audio, track);
        });
    }

    // Projeção inicial do estado do core.
    refresh_beat(ui, &project.borrow());
}
