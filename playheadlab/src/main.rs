// Laboratório de playhead: reproduz o clock + scrub do app
// isoladamente pra validar fluidez antes de portar.

mod clock;

use clock::{SeekThrottle, VisualClock};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

slint::include_modules!();

struct State {
    clock: VisualClock,
    throttle: SeekThrottle,
    /// Taxas medidas pro HUD (reset por frame).
    seeks_frame: std::cell::Cell<u32>,
    moves_frame: std::cell::Cell<u32>,
}

fn main() {
    let ui = MainWindow::new().unwrap();

    let state = Rc::new(RefCell::new(State {
        clock: VisualClock::new(),
        throttle: SeekThrottle::new(),
        seeks_frame: std::cell::Cell::new(0),
        moves_frame: std::cell::Cell::new(0),
    }));

    // ---------- Callback do Slint: seek da régua ----------
    {
        let st = state.clone();
        ui.on_seek(move |secs: f32| {
            st.borrow().throttle.request(secs as f64);
            st.borrow().moves_frame.set(st.borrow().moves_frame.get() + 1);
        });
    }

    // ---------- Botão play/pause ----------
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_toggle_play(move || {
            let mut s = st.borrow_mut();
            let now_playing = !s.clock.is_playing();
            s.clock.set_playing(now_playing);
            drop(s);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_playing(now_playing);
            }
        });
    }

    // ---------- Timer 30 Hz ----------
    // 1. Aplica até UM seek pendente (elimina "ímã").
    // 2. Publica posição visual do clock.
    // 3. Atualiza HUD (seeks/s, moves/s).
    let ui_weak = ui.as_weak();
    let st_weak = state.clone();
    let play_timer = slint::Timer::default();
    play_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            let mut st = st_weak.borrow_mut();
            if let Some(s) = st.throttle.due() {
                st.clock.seek(s);
                st.seeks_frame.set(st.seeks_frame.get() + 1);
            }
            let secs = st.clock.projected().max(0.0);

            let sf = st.seeks_frame.get();
            let mf = st.moves_frame.get();
            st.seeks_frame.set(0);
            st.moves_frame.set(0);
            drop(st);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_transport_secs(secs as f32);
                ui.set_seeks_per_sec(sf as i32);
                ui.set_moves_per_sec(mf as i32);
            }
        },
    );

    ui.run().unwrap();
}
