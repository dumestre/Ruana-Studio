slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use daw_core::clip::{Clip, ClipKind};
use daw_core::project::Project;
use daw_core::track::TrackKind;
use daw_core::transport::Transport;

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

/// Reconstrói os modelos de tracks e clips a partir do core.
///
/// O core (Project) é a fonte de verdade; a UI é uma projeção.
/// Reprojeção total é suficiente na Fase A — reprojeção granular
/// (push/set_row_data) chega com os eventos reativos na Fase B.
fn reproject(ui: &MainWindow, project: &Project) {
    let track_names: Vec<slint::SharedString> = project
        .tracks
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    ui.set_track_names(Rc::new(slint::VecModel::from(track_names)).into());

    let clips: Vec<ClipData> = project.clips.iter().map(to_clip_data).collect();
    ui.set_clips(Rc::new(slint::VecModel::from(clips)).into());
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Estado autoritativo da DAW: vive no core, nunca na UI.
    let project = Rc::new(RefCell::new(Project::demo()));
    let transport = Rc::new(RefCell::new(Transport::default()));

    reproject(&ui, &project.borrow());

    // Drop de clip (ClipPool / Explorer) → core → reprojeção.
    {
        let ui_weak = ui.as_weak();
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
            if let Some(ui) = ui_weak.upgrade() {
                reproject(&ui, &proj);
            }
        });
    }

    // Adiciona tracks via botão "+" da timeline → core.
    {
        let ui_weak = ui.as_weak();
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
            if let Some(ui) = ui_weak.upgrade() {
                reproject(&ui, &proj);
            }
        });
    }

    // Slice de clip (ferramenta de corte) → core.
    {
        let ui_weak = ui.as_weak();
        let project = project.clone();
        ui.on_clip_slice_requested(move |id, position| {
            let id = id as u64;
            let mut proj = project.borrow_mut();
            let Some(clip) = proj.clip(id).cloned() else {
                return;
            };
            let at = clip.start_beat + position.round().max(0.0) as u32;
            proj.slice_clip(id, at);
            if let Some(ui) = ui_weak.upgrade() {
                reproject(&ui, &proj);
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
