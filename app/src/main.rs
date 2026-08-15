slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use daw_core::project::Project;
use slint::Model;

fn next_random(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let r = (*seed >> 33) as f32 / u32::MAX as f32;
    r
}

/// Projeta o modelo do daw-core para os modelos da UI.
///
/// A UI nunca recebe estruturas do core diretamente;
/// ela recebe DTOs (ClipData) próprios da camada Slint.
///
/// Retorna (modelo de tracks, modelo de clips) para permitir
/// adicionar tracks/clips em runtime.
fn bind_project(
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

    let clips: Vec<ClipData> = project
        .clips
        .iter()
        .map(|c| ClipData {
            id: c.id as i32,
            name: c.name.clone().into(),
            track_index: c.track_id as i32,
            beat: c.start_beat as f32,
            len: c.length_beats as f32,
            is_midi: c.is_midi(),
        })
        .collect();
    let clips_model = Rc::new(slint::VecModel::from(clips));
    ui.set_clips(clips_model.clone().into());

    (tracks_model, clips_model)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Projeto placeholder do daw-core — o core não conhece a UI.
    let project = Project::demo();
    let (tracks_model, _clips_model) = bind_project(&ui, &project);

    // Adiciona tracks via botão "+" da timeline.
    {
        ui.on_add_track_requested(move |kind| {
            let base = match kind.as_str() {
                "AUDIO" => "Audio",
                "MIDI" => "MIDI",
                "INSTRUMENT" => "Instrument",
                "DRUM" => "Drums",
                "FX" => "FX",
                "BUS" => "Bus",
                "MASTER" => "Master",
                _ => "Track",
            };
            let count = tracks_model.row_count() + 1;
            tracks_model.push(slint::SharedString::from(format!("{base} {count}")));
        });
    }

    let mut seed = 0x1234_5678_9abc_def0u64;

    // Playback: move o playhead enquanto "tocando".
    {
        let ui_weak = ui.as_weak();
        let timer = Rc::new(RefCell::new(slint::Timer::default()));
        let timer_ref = timer.clone();
        ui.on_play_state_changed(move |playing| {
            if playing {
                let ui_weak = ui_weak.clone();
                timer_ref.borrow().start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(50),
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let pos = ui.get_transport_position();
                            ui.set_transport_position(pos + 48);
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
