slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use daw_core::clip::{Clip, ClipKind};
use daw_core::mixer::MixerParam;
use daw_core::project::{ChangeKind, DocumentEvent, Project};
use daw_core::track::TrackKind;
use daw_core::transport::Transport;
use slint::Model;

fn next_random(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let r = (*seed >> 33) as f32 / u32::MAX as f32;
    r
}

/// Mapeia um clip do core para o DTO da camada Slint.
fn to_clip_data(clip: &Clip) -> ClipData {
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat as f32,
        len: clip.length_beats as f32,
        is_midi: clip.is_midi(),
    }
}

/// Extrai o estado do mixer do projeto (trilhas + master) como arrays.
fn mixer_state(project: &Project) -> (Vec<bool>, Vec<bool>, Vec<bool>, Vec<f32>, Vec<f32>) {
    let n = project.tracks.len() + 1; // trilhas + master
    let mut mutes = Vec::with_capacity(n);
    let mut soloes = Vec::with_capacity(n);
    let mut arms = Vec::with_capacity(n);
    let mut volumes = Vec::with_capacity(n);
    let mut pans = Vec::with_capacity(n);

    for t in &project.tracks {
        mutes.push(t.mixer.muted);
        soloes.push(t.mixer.soloed);
        arms.push(t.mixer.armed);
        volumes.push(t.mixer.volume);
        pans.push(t.mixer.pan);
    }
    // Master
    mutes.push(project.master_mixer.muted);
    soloes.push(project.master_mixer.soloed);
    arms.push(project.master_mixer.armed);
    volumes.push(project.master_mixer.volume);
    pans.push(project.master_mixer.pan);

    (mutes, soloes, arms, volumes, pans)
}

/// Preenche os modelos da UI a partir do estado inicial do core.
///
/// Retorna os `VecModel` compartilhados para que o assinante de
/// eventos possa atualizá-los granularmente em runtime.
fn initial_bind(
    ui: &MainWindow,
    project: &Project,
) -> (
    Rc<slint::VecModel<slint::SharedString>>,
    Rc<slint::VecModel<ClipData>>,
) {
    let track_names: Vec<slint::SharedString> = project
        .tracks
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    let tracks_model = Rc::new(slint::VecModel::from(track_names));
    ui.set_track_names(tracks_model.clone().into());

    let clips: Vec<ClipData> = project.clips.iter().map(to_clip_data).collect();
    let clips_model = Rc::new(slint::VecModel::from(clips));
    ui.set_clips(clips_model.clone().into());

    // Mixer inicial
    let (mutes, soloes, arms, volumes, pans) = mixer_state(project);
    ui.set_mixer_mutes(Rc::new(slint::VecModel::from(mutes)).into());
    ui.set_mixer_soloes(Rc::new(slint::VecModel::from(soloes)).into());
    ui.set_mixer_arms(Rc::new(slint::VecModel::from(arms)).into());
    ui.set_mixer_volumes(Rc::new(slint::VecModel::from(volumes)).into());
    ui.set_mixer_pans(Rc::new(slint::VecModel::from(pans)).into());

    (tracks_model, clips_model)
}

/// Índice da linha do modelo cujo clip tem o id informado.
fn clip_row(model: &slint::VecModel<ClipData>, id: i32) -> Option<usize> {
    (0..model.row_count()).find(|&i| model.row_data(i).map(|c| c.id == id).unwrap_or(false))
}

/// Aplica um `DocumentEvent` aos modelos da UI.
///
/// `undo = true` reverte a mudança (caso contrário aplica/refaz).
/// Nunca toca no `Project` — apenas nos modelos, pois é chamado
/// de dentro de uma mutação do core (reentrância proibida).
fn apply_change(
    tracks_model: &slint::VecModel<slint::SharedString>,
    clips_model: &slint::VecModel<ClipData>,
    event: &DocumentEvent,
    undo: bool,
) {
    match event {
        DocumentEvent::TrackAdded(track) => {
            if undo {
                let track_id = track.id as i32;
                let mut i = 0;
                while i < clips_model.row_count() {
                    if clips_model
                        .row_data(i)
                        .map(|c| c.track_index == track_id)
                        .unwrap_or(false)
                    {
                        clips_model.remove(i);
                    } else {
                        i += 1;
                    }
                }
                if tracks_model.row_count() > 0 {
                    tracks_model.remove(tracks_model.row_count() - 1);
                }
            } else {
                tracks_model.push(track.name.clone().into());
            }
        }
        DocumentEvent::ClipAdded(clip) => {
            if undo {
                if let Some(i) = clip_row(clips_model, clip.id as i32) {
                    clips_model.remove(i);
                }
            } else {
                clips_model.push(to_clip_data(clip));
            }
        }
        DocumentEvent::ClipSliced {
            original,
            left,
            right,
        } => {
            if undo {
                if let Some(i) = clip_row(clips_model, right.id as i32) {
                    clips_model.remove(i);
                }
                if let Some(i) = clip_row(clips_model, original.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(original));
                }
            } else {
                if let Some(i) = clip_row(clips_model, left.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left));
                }
                clips_model.push(to_clip_data(right));
            }
        }
        DocumentEvent::ClipMoved { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data));
            }
        }
        DocumentEvent::ClipDeleted(clip) => {
            if undo {
                clips_model.push(to_clip_data(clip));
            } else if let Some(i) = clip_row(clips_model, clip.id as i32) {
                clips_model.remove(i);
            }
        }
    }
}

/// Assina os eventos do documento e mantém os modelos da UI
/// sincronizados de forma granular.
fn bind_document_events(
    project: &Project,
    tracks_model: Rc<slint::VecModel<slint::SharedString>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
) {
    project.subscribe(move |event, kind| {
        apply_change(&tracks_model, &clips_model, event, kind == ChangeKind::Undo);
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Estado autoritativo da DAW: vive no core, nunca na UI.
    let project = Rc::new(RefCell::new(Project::demo()));
    let transport = Rc::new(RefCell::new(Transport::default()));

    let (tracks_model, clips_model) = initial_bind(&ui, &project.borrow());
    bind_document_events(&project.borrow(), tracks_model, clips_model);

    // Drop de clip (ClipPool / Explorer) → core (evento reprojeta).
    {
        let project = project.clone();
        ui.on_clip_dropped(move |track, beat, name, is_midi| {
            let kind = if is_midi {
                ClipKind::Midi
            } else {
                ClipKind::Audio
            };
            let track_id = track.max(0) as u64;
            let start = beat.round().max(0.0) as u32;
            let mut proj = project.borrow_mut();
            if proj.track(track_id).is_some() {
                proj.add_clip(track_id, kind, name.as_str(), start, 4);
            }
        });
    }

    // Adiciona tracks via botão "+" da timeline → core.
    {
        let project = project.clone();
        ui.on_add_track_requested(move |kind| {
            let kind = match kind.as_str() {
                "AUDIO" => TrackKind::Audio,
                "INSTRUMENT" => TrackKind::Instrument,
                "DRUM" => TrackKind::Drum,
                "FX" => TrackKind::Fx,
                "BUS" => TrackKind::Bus,
                "MASTER" => TrackKind::Master,
                _ => TrackKind::Midi,
            };
            let mut proj = project.borrow_mut();
            let count = proj.tracks.len() + 1;
            proj.add_track(kind, format!("{} {count}", kind.base_name()));
        });
    }

    // Slice de clip (ferramenta de corte) → core.
    {
        let project = project.clone();
        ui.on_clip_slice_requested(move |id, position| {
            let id = id as u64;
            let mut proj = project.borrow_mut();
            let Some(clip) = proj.clip(id).cloned() else {
                return;
            };
            let at = clip.start_beat + position.round().max(0.0) as u32;
            proj.slice_clip(id, at);
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

    // Parâmetros do mixer → core.
    {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        ui.on_mixer_param_changed(move |channel, param, value| {
            let param = match MixerParam::from_str(param.as_str()) {
                Some(p) => p,
                None => return,
            };
            let mut proj = project.borrow_mut();
            proj.set_mixer_param(channel as usize, param, value);
            // Reprojeta o mixer imediatamente (o core é fonte de verdade)
            let proj = project.borrow();
            let (mutes, soloes, arms, volumes, pans) = mixer_state(&proj);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_mixer_mutes(Rc::new(slint::VecModel::from(mutes)).into());
                ui.set_mixer_soloes(Rc::new(slint::VecModel::from(soloes)).into());
                ui.set_mixer_arms(Rc::new(slint::VecModel::from(arms)).into());
                ui.set_mixer_volumes(Rc::new(slint::VecModel::from(volumes)).into());
                ui.set_mixer_pans(Rc::new(slint::VecModel::from(pans)).into());
            }
        });
    }

    let mut seed = 0x1234_5678_9abc_def0u64;

    // Playback: o playhead vive no Transport do core; o timer só
    // avança o core e projeta a posição na UI.
    {
        let ui_weak = ui.as_weak();
        let timer = Rc::new(RefCell::new(slint::Timer::default()));
        let timer_ref = timer.clone();
        let transport = transport.clone();
        ui.on_play_state_changed(move |playing| {
            transport.borrow_mut().toggle();
            if playing {
                let ui_weak = ui_weak.clone();
                let transport = transport.clone();
                timer_ref.borrow().start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(50),
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let mut t = transport.borrow_mut();
                            t.advance(48);
                            ui.set_transport_position(t.position_ticks as i32);
                        }
                    },
                );
            } else {
                timer_ref.borrow().stop();
            }
        });
    }

    // Meters animados (áudio falso) — só rodam enquanto "tocando".
    let meter_timer = Rc::new(slint::Timer::default());
    {
        let ui_weak = ui.as_weak();
        let timer = meter_timer.clone();
        ui.on_play_state_changed(move |playing| {
            if playing {
                let ui_weak = ui_weak.clone();
                let timer = timer.clone();
                timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(80),
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let mut levels: Vec<f32> = Vec::new();
                            for _ in 0..5 {
                                let r = next_random(&mut seed);
                                levels.push(r * r);
                            }
                            ui.set_mixer_meters(Rc::new(slint::VecModel::from(levels)).into());
                        }
                    },
                );
            } else {
                timer.stop();
            }
        });
    }

    ui.on_close(move || {
        std::process::exit(0);
    });

    let ui_weak_min = ui.as_weak();
    ui.on_minimize(move || {
        if let Some(ui) = ui_weak_min.upgrade() {
            let window = ui.window();
            window.set_minimized(true);
        }
    });

    ui.run()
}