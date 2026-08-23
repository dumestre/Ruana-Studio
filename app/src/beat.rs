//! Bridge do beat sequencer: persistência no core.
//!
//! Core → UI: `refresh_beat` re-projeta patterns e parâmetros das
//! trilhas no global `BeatState`.
//!
//! UI → Core: os callbacks do global gravam no `Project` e disparam
//! `refresh_beat`, mantendo o core como fonte única de verdade.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel};

use daw_core::project::Project;
use daw_midi::{DrumPattern, DrumTrackParams};

use crate::{BeatState, MainWindow};

/// Aplica uma mutação no projeto e re-projeta o beat na sequência.
fn commit(
    ui: &MainWindow,
    project: &RefCell<Project>,
    f: impl FnOnce(&mut Project),
) {
    f(&mut project.borrow_mut());
    refresh_beat(ui, &project.borrow());
}

/// Re-projeta o estado do beat sequencer (core) nos models da UI.
fn refresh_beat(ui: &MainWindow, project: &Project) {
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
    let pattern_names: Vec<SharedString> = bp
        .patterns
        .iter()
        .map(|p| p.name.clone().into())
        .collect();
    st.set_pattern_names(Rc::new(VecModel::from(pattern_names)).into());

    let Some(current) = bp.current_pattern() else {
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

    let active: Vec<bool> = current.track_params.iter().map(|t| t.active).collect();
    let solo: Vec<bool> = current.track_params.iter().map(|t| t.solo).collect();
    let pan: Vec<f32> = current.track_params.iter().map(|t| t.pan).collect();
    let volume: Vec<f32> = current.track_params.iter().map(|t| t.volume).collect();
    let channels: Vec<i32> =
        current.track_params.iter().map(|t| t.mixer_channel).collect();

    st.set_track_active(Rc::new(VecModel::from(active)).into());
    st.set_track_solo(Rc::new(VecModel::from(solo)).into());
    st.set_track_pan(Rc::new(VecModel::from(pan)).into());
    st.set_track_volume(Rc::new(VecModel::from(volume)).into());
    st.set_track_mixer_channel(Rc::new(VecModel::from(channels)).into());
}

/// Conecta todos os callbacks do beat sequencer ao core.
pub fn wire_beat(ui: &MainWindow, project: Rc<RefCell<Project>>) {
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
        st.on_step_committed(move |index, value| {
            commit(&ui, &project, |proj| {
                let stride = proj
                    .current_beat_pattern()
                    .map(|p| p.steps())
                    .unwrap_or(16)
                    .max(1);
                proj.set_beat_step(
                    index as usize / stride,
                    index as usize % stride,
                    value,
                );
            });
        });
    }

    // =========================================================
    // PADRÃO ATUAL / RESOLUÇÃO / HUMANIZE / SWING
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_pattern_changed(move |index| {
            if index < 0 {
                return;
            }
            commit(&ui, &project, |proj| {
                proj.select_beat_pattern(index as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_grid_resolution_committed(move |resolution| {
            if resolution <= 0 {
                return;
            }
            commit(&ui, &project, |proj| {
                proj.set_beat_resolution(resolution as u32);
            });
        });
    }

    {
        let project = project.clone();
        st.on_humanize_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.humanize = value.clamp(0.0, 1.0);
            }
        });
    }

    {
        let project = project.clone();
        st.on_swing_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.swing = value.clamp(0.0, 1.0);
            }
        });
    }

    // =========================================================
    // BEAT (padrão inteiro)
    // =========================================================

    {
        let clipboard = beat_clipboard.clone();
        let project = project.clone();
        st.on_copy_beat(move || {
            *clipboard.borrow_mut() =
                project.borrow().current_beat_pattern().cloned();
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_duplicate_beat(move || {
            commit(&ui, &project, |proj| {
                proj.duplicate_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_clear_beat(move || {
            commit(&ui, &project, |proj| {
                proj.clear_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_delete_beat(move || {
            commit(&ui, &project, |proj| {
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
                    *clipboard.borrow_mut() =
                        Some((p.grid[i].clone(), p.track_params[i].clone()));
                }
            }
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_duplicate_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, |proj| {
                proj.duplicate_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_delete_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, |proj| {
                proj.delete_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_clear_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, |proj| {
                proj.clear_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_insert_pattern(move |_track, interval| {
            if interval <= 0 {
                return;
            }
            commit(&ui, &project, |proj| {
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
        st.on_track_param_committed(move |track, param, value| {
            if track < 0 {
                return;
            }
            if let Some(t) =
                project.borrow_mut().beat_track_params_mut(track as usize)
            {
                match param.as_str() {
                    "active" => t.active = value >= 0.5,
                    "solo" => t.solo = value >= 0.5,
                    "pan" => t.pan = value.clamp(-1.0, 1.0),
                    "volume" => t.volume = value.clamp(0.0, 1.5),
                    "channel" => t.mixer_channel = value.round().clamp(0.0, 15.0) as i32,
                    _ => {}
                }
            }
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
        win.on_beat_sample_dropped(move |track, _name, path| {
            if track < 0 {
                return;
            }
            commit(&handler, &project, |proj| {
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
        });
    }

    // Projeção inicial do estado do core.
    refresh_beat(ui, &project.borrow());
}
