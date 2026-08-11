slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn next_random(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let r = (*seed >> 33) as f32 / u32::MAX as f32;
    r
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

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

    let ui_weak_max = ui.as_weak();
    ui.on_maximize(move || {
        if let Some(ui) = ui_weak_max.upgrade() {
            let window = ui.window();
            window.set_maximized(!window.is_maximized());
        }
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
