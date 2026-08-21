//! Playback: liga o transporte (core) à UI.
//!
//! O engine de áudio avança o `Transport` na thread RT (clock
//! autoritativo). Aqui vive o clock VISUAL do playhead: um Timer
//! dentro da event loop (~8ms) que lê o transport e EXTRAPOLA a
//! posição por tempo real (`Instant`) entre os ticks de áudio.
//!
//! A correção de drift é gradual (servo/PLL): a cada tick a âncora
//! absorve uma fração do erro em vez de re-ancorar de uma vez —
//! qualquer resquício de deriva vira correção invisível, nunca um
//! salto na agulha.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use daw_core::project::Project;
use daw_core::transport::Transport;
use slint::{ComponentHandle, Model};

use crate::MainWindow;

/// Log de runtime (playback) — só ativo com `RUANA_LOG` definido.
fn rt_log(msg: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var("RUANA_LOG").is_ok()) {
        eprintln!("{}", msg);
    }
}

/// PRNG determinístico dos meters (VU fake).
fn next_random(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) as f32 / u32::MAX as f32
}

/// Clock visual do playhead: âncora `(ticks, Instant)` + taxa em
/// ticks/seg. A posição exibida é projetada pela âncora; o valor
/// real do áudio só a corrige gradualmente.
struct VisualClock {
    anchor_ticks: f64,
    anchor_at: Instant,
    ticks_per_sec: f64,
}

impl VisualClock {
    /// Re-ancora instantaneamente (play, seek): salto intencional.
    fn resync(&mut self, ticks: f64) {
        self.anchor_ticks = ticks;
        self.anchor_at = Instant::now();
    }

    /// Posição projetada "agora".
    fn projected(&self) -> f64 {
        self.anchor_ticks + self.anchor_at.elapsed().as_secs_f64() * self.ticks_per_sec
    }

    /// Servo: absorve 15% do drift por tick (~95% do erro some em
    /// ~160ms). NÃO mexe em `anchor_at` — a projeção continua de
    /// onde está, então não há descontinuidade. Com drift contido,
    /// cada passo corrige < 0.5px: invisível.
    fn nudge(&mut self, actual_ticks: f64) {
        let drift = actual_ticks - self.projected();
        self.anchor_ticks += drift * 0.15;
    }
}

/// Instala os handlers de transporte na UI: play/pause/stop, seek e
/// os timers de playhead + meters. Chamar uma vez no startup.
pub fn install(
    ui: &slint::Weak<MainWindow>,
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
    meter_model: Rc<slint::VecModel<f32>>,
    meter_state: Rc<RefCell<Vec<f32>>>,
) {
    // Handlers precisam da instância forte; o Weak é só para os
    // closures dos timers (evita ciclo com a event loop).
    let Some(ui) = ui.upgrade() else { return };
    let ui_weak = ui.as_weak();
    let mut seed: u64 = 0x1234_5678_9abc_def0;

    // AudioEngine: fica vivo enquanto estiver tocando.
    let audio_engine: Rc<RefCell<Option<daw_audio::AudioEngine>>> =
        Rc::new(RefCell::new(None));
    let _audio_snapshot: Rc<RefCell<Option<Arc<Mutex<daw_audio::ProjectSnapshot>>>>> =
        Rc::new(RefCell::new(None));
    let _audio_buffers: Rc<RefCell<Option<Arc<Mutex<daw_audio::BufferCache>>>>> =
        Rc::new(RefCell::new(None));

    let meter_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let playhead_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let visual_clock: Rc<RefCell<Option<VisualClock>>> = Rc::new(RefCell::new(None));
    // Guarda de idempotência: espaço/stop repetidos não re-executam
    // o start/stop (evita recriar engine e timers duplicados).
    let is_playing = Rc::new(Cell::new(false));

    // ---- PLAY / STOP ------------------------------------------------
    {
        let ui_weak = ui_weak.clone();
        let transport = transport.clone();
        let project = project.clone();
        let visual_clock = visual_clock.clone();
        let is_playing = is_playing.clone();
        let playhead_timer = playhead_timer.clone();
        let meter_timer = meter_timer.clone();
        let audio_engine = audio_engine.clone();
        ui.on_play_state_changed(move |playing| {
            if playing {
                if is_playing.get() {
                    return;
                }
                is_playing.set(true);
                // Inicia o audio engine (abre stream CPAL)
                match daw_audio::AudioEngine::start(transport.clone(), 48000) {
                    Ok((engine, snapshot, buffers)) => {
                        // Atualiza o snapshot e pré-carrega clips
                        {
                            let proj = project.borrow();
                            let mut snap = snapshot.lock().unwrap();
                            snap.update_from(&proj);
                        }
                        engine.preload_clips(&project.borrow());
                        *_audio_snapshot.borrow_mut() = Some(snapshot);
                        *_audio_buffers.borrow_mut() = Some(buffers);
                        *audio_engine.borrow_mut() = Some(engine);
                    }
                    Err(e) => {
                        eprintln!("audio engine: {}", e);
                    }
                }
                {
                    let mut t = transport.lock().unwrap();
                    t.playing = true;
                    // Âncora inicial do clock visual
                    let tps = t.bpm as f64 / 60.0 * t.ppq as f64;
                    *visual_clock.borrow_mut() = Some(VisualClock {
                        anchor_ticks: t.position_ticks as f64,
                        anchor_at: Instant::now(),
                        ticks_per_sec: tps,
                    });
                }
                rt_log("[PLAY] transport.playing=true, playhead timer started");

                // Playhead timer: roda NA event loop. Lê o clock
                // autoritativo (áudio) via try_lock, corrige drift
                // gradualmente (servo) e projeta por tempo real.
                {
                    let ui_weak_t = ui_weak.clone();
                    let transport = transport.clone();
                    let clock = visual_clock.clone();
                    let last_shown = Cell::new(i64::MIN);
                    playhead_timer.borrow().start(
                        slint::TimerMode::Repeated,
                        Duration::from_millis(8),
                        move || {
                            let mut guard = clock.borrow_mut();
                            let Some(c) = guard.as_mut() else { return };
                            if let Ok(t) = transport.try_lock() {
                                if !t.playing {
                                    return;
                                }
                                let actual = t.position_ticks as f64;
                                let drift = actual - c.projected();
                                if drift.abs() > c.ticks_per_sec * 0.25 {
                                    // Descontinuidade real (glitch longo):
                                    // re-ancora na posição verdadeira.
                                    c.resync(actual);
                                } else {
                                    // Jitter/deriva pequena: servo.
                                    c.nudge(actual);
                                }
                            }
                            let shown = c.projected().max(0.0) as i64;
                            drop(guard);
                            if shown != last_shown.get() {
                                last_shown.set(shown);
                                if let Some(ui) = ui_weak_t.upgrade() {
                                    ui.set_transport_position(
                                        shown.min(i32::MAX as i64) as i32,
                                    );
                                }
                            }
                        },
                    );
                }
                // Meter timer: VU fake com smoothing no Rust (attack
                // instantâneo, decay lento). SEM animação Slint — a
                // animação contínua satura o renderer e trava o playhead.
                let project = project.clone();
                let meter_model = meter_model.clone();
                let meter_state = meter_state.clone();
                meter_timer.borrow().start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(80),
                    move || {
                        let count = project.borrow().tracks.len() + 1;
                        let mut levels = meter_state.borrow_mut();
                        if levels.len() != count {
                            levels.resize(count, 0.0);
                        }
                        for level in levels.iter_mut() {
                            let r = next_random(&mut seed);
                            let new = r * r;
                            *level = if new > *level { new } else { *level * 0.82 };
                        }
                        for (i, &v) in levels.iter().enumerate() {
                            meter_model.set_row_data(i, v);
                        }
                    },
                );
            } else {
                if !is_playing.get() {
                    return;
                }
                is_playing.set(false);
                rt_log("[STOP] stopping transport + timers");
                playhead_timer.borrow().stop();
                *visual_clock.borrow_mut() = None;
                transport.lock().unwrap().playing = false;
                meter_timer.borrow().stop();
                // Para o audio engine
                *audio_engine.borrow_mut() = None;
                *_audio_snapshot.borrow_mut() = None;
                *_audio_buffers.borrow_mut() = None;
            }
        });
    }

    // ---- BPM --------------------------------------------------------
    {
        let transport = transport.clone();
        let visual_clock = visual_clock.clone();
        let ui_weak_b = ui_weak.clone();
        ui.on_set_bpm(move |bpm| {
            let bpm = bpm.round().clamp(20.0, 300.0) as u32;
            let tps;
            {
                let mut t = transport.lock().unwrap();
                if t.bpm == bpm {
                    return;
                }
                t.bpm = bpm;
                tps = bpm as f64 / 60.0 * t.ppq as f64;
            }
            // Reflete no LCD (property two-way até o Transport).
            if let Some(ui) = ui_weak_b.upgrade() {
                ui.set_bpm(bpm as i32);
            }
            // Re-ancora o clock visual com a NOVA taxa, mantendo a
            // posição projetada contínua — a agulha não salta; o
            // servo absorve a diferença do clock de áudio.
            if let Some(c) = visual_clock.borrow_mut().as_mut() {
                let projected = c.projected();
                c.ticks_per_sec = tps;
                c.resync(projected);
            }
        });
    }

    // ---- SEEK -------------------------------------------------------
    {
        let transport = transport.clone();
        let ui_weak_s = ui_weak.clone();
        let visual_clock = visual_clock.clone();
        ui.on_seek_to(move |beat| {
            let ticks = (beat * 960.0) as u64;
            let pos = {
                let mut t = transport.lock().unwrap();
                t.seek(ticks);
                t.position_ticks
            };
            // Re-ancora o clock visual: agulha salta imediatamente,
            // sem esperar o timer detectar o drift.
            if let Some(c) = visual_clock.borrow_mut().as_mut() {
                c.resync(pos as f64);
            }
            if let Some(ui) = ui_weak_s.upgrade() {
                ui.set_transport_position(pos.min(i32::MAX as u64) as i32);
            }
        });
    }
}
