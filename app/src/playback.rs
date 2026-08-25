//! Playback: liga o transporte (core) à UI.
//!
//! O engine de áudio avança o `Transport` na thread RT (clock
//! autoritativo). Aqui vive o clock VISUAL do playhead: um Timer
//! dentro da event loop (~8ms) que lê o transport e EXTRAPOLA a
//! posição por tempo real (`Instant`) entre os ticks de áudio,
//! publicando SEGUNDOS contínuos (`transport-time-seconds`) — o
//! domínio da Playhead. Ticks inteiros seguem só para o LCD.
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
use slint::{ComponentHandle, Global, Model};

use crate::audio_bridge::AudioBridge;
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

/// Clock visual do playhead: âncora `(segundos, Instant)`.
///
/// A Playhead vive no domínio de TEMPO CONTÍNUO (segundos), nunca
/// em beats/ticks: a projeção avança à taxa 1 s/s (o clock de áudio
/// também avança em tempo real) e o valor real só corrige a âncora
/// gradualmente. A conversão segundos → pixels acontece na UI:
/// `playhead_x = segundos * pixels_per_second`.
struct VisualClock {
    anchor_secs: f64,
    anchor_at: Instant,
}

impl VisualClock {
    /// Re-ancora instantaneamente (play, seek, bpm): salto intencional.
    fn resync(&mut self, secs: f64) {
        self.anchor_secs = secs;
        self.anchor_at = Instant::now();
    }

    /// Posição projetada "agora", em segundos (contínua).
    fn projected(&self) -> f64 {
        self.anchor_secs + self.anchor_at.elapsed().as_secs_f64()
    }

    /// Servo: absorve 15% do drift por tick (~95% do erro some em
    /// ~160ms). NÃO mexe em `anchor_at` — a projeção continua de
    /// onde está, então não há descontinuidade. Com drift contido,
    /// cada passo corrige < 0.5px: invisível.
    fn nudge(&mut self, actual_secs: f64) {
        let drift = actual_secs - self.projected();
        self.anchor_secs += drift * 0.15;
    }
}

/// Publica no `BeatState` o passo que está soando.
///
/// Derivado do mesmo relógio do áudio, com a mesma divisão que o
/// motor usa (semicolcheia). Deduplicado: o passo só muda ~8×/s no
/// máximo, e escrever a propriedade a cada 8 ms invalidaria o grid
/// inteiro sem necessidade.
fn publish_beat_step(ui: &MainWindow, secs: f64, bpm: f64, last: &Cell<i32>) {
    let st = crate::BeatState::get(ui);
    let steps = st.get_grid_resolution();
    if steps <= 0 {
        return;
    }
    let beats = secs * bpm / 60.0;
    let step = (beats * daw_audio::STEPS_PER_BEAT).floor() as i64;
    let step = step.rem_euclid(steps as i64) as i32;
    if step != last.get() {
        last.set(step);
        st.set_playhead(step);
    }
}

/// Instala os handlers de transporte na UI: play/pause/stop, seek e
/// os timers de playhead + meters. Chamar uma vez no startup.
///
/// O motor de áudio já está rodando quando isto é chamado (ver
/// [`crate::audio_bridge`]): play/stop aqui só ligam e desligam o
/// transporte, sem abrir nem fechar o dispositivo.
pub fn install(
    ui: &slint::Weak<MainWindow>,
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
    meter_model: Rc<slint::VecModel<f32>>,
    meter_state: Rc<RefCell<Vec<f32>>>,
    audio: AudioBridge,
) {
    // Handlers precisam da instância forte; o Weak é só para os
    // closures dos timers (evita ciclo com a event loop).
    let Some(ui) = ui.upgrade() else { return };
    let ui_weak = ui.as_weak();
    let mut seed: u64 = 0x1234_5678_9abc_def0;

    let meter_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let playhead_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let visual_clock: Rc<RefCell<Option<VisualClock>>> = Rc::new(RefCell::new(None));
    // Guarda de idempotência: espaço/stop repetidos não re-executam
    // o start/stop (evita timers duplicados).
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
        let audio = audio.clone();
        ui.on_play_state_changed(move |playing| {
            if playing {
                if is_playing.get() {
                    return;
                }
                is_playing.set(true);

                // O snapshot precisa estar publicado ANTES do primeiro
                // buffer tocar, senão o começo sai em silêncio.
                audio.flush_now(&project.borrow());

                {
                    let mut t = transport.lock().unwrap();
                    t.playing = true;
                    // Lembra onde o play começou: SPACE (stop) devolve
                    // a agulha pra cá (comportamento padrão de DAW).
                    t.play_start_ticks = t.position_ticks;
                    // Âncora inicial do clock visual (segundos contínuos)
                    *visual_clock.borrow_mut() = Some(VisualClock {
                        anchor_secs: t.position_seconds(),
                        anchor_at: Instant::now(),
                    });
                }
                rt_log("[PLAY] transport.playing=true, playhead timer started");

                // Playhead timer: roda NA event loop. Lê o clock
                // autoritativo (áudio) via try_lock, corrige drift
                // gradualmente (servo) e projeta por tempo real.
                // Publica SEGUNDOS (float, sem quantização) para a
                // agulha e ticks inteiros apenas para o LCD.
                {
                    let ui_weak_t = ui_weak.clone();
                    let transport = transport.clone();
                    let clock = visual_clock.clone();
                    let last_lcd = Cell::new(i32::MIN);
                    let last_step = Cell::new(i32::MIN);
                    playhead_timer.borrow().start(
                        slint::TimerMode::Repeated,
                        Duration::from_millis(8),
                        move || {
                            let mut guard = clock.borrow_mut();
                            let Some(c) = guard.as_mut() else { return };
                            let mut lcd_ticks: Option<i64> = None;
                            let mut bpm = 120.0f64;
                            if let Ok(t) = transport.try_lock() {
                                if !t.playing {
                                    return;
                                }
                                bpm = t.bpm as f64;
                                let actual = t.position_seconds();
                                let drift = actual - c.projected();
                                if drift.abs() > 0.25 {
                                    // Descontinuidade real (glitch longo):
                                    // re-ancora na posição verdadeira.
                                    c.resync(actual);
                                } else {
                                    // Jitter/deriva pequena: servo.
                                    c.nudge(actual);
                                }
                                // LCD (bars:beat:ticks) — domínio musical,
                                // derivado dos segundos com o BPM atual.
                                lcd_ticks = Some(
                                    (c.projected().max(0.0)
                                        * t.bpm as f64
                                        / 60.0
                                        * t.ppq as f64) as i64,
                                );
                            }
                            let secs = c.projected().max(0.0);
                            drop(guard);
                            if let Some(ui) = ui_weak_t.upgrade() {
                                // Playhead: valor contínuo, atualiza TODO tick.
                                ui.set_transport_time_seconds(secs as f32);
                                // LCD: inteiro dedupado (granularidade de tick).
                                if let Some(ticks) = lcd_ticks {
                                    let ticks = (ticks.min(i32::MAX as i64)) as i32;
                                    if ticks != last_lcd.get() {
                                        last_lcd.set(ticks);
                                        ui.set_transport_position(ticks);
                                    }
                                }
                                // Passo do beat sequencer: derivado do
                                // MESMO relógio do áudio, então a coluna
                                // acesa é a que está soando (antes era um
                                // Timer de 140ms sem relação com nada).
                                publish_beat_step(&ui, secs, bpm, &last_step);
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

                // SPACE = STOP: agulha volta pra onde o play começou.
                let (ret_ticks, ret_secs) = {
                    let mut t = transport.lock().unwrap();
                    t.playing = false;
                    t.position_ticks = t.play_start_ticks;
                    (t.position_ticks, t.position_seconds())
                };
                meter_timer.borrow().stop();
                // O dispositivo de áudio NÃO fecha aqui: continua
                // aberto para a pré-escuta de samples com o transporte
                // parado. Ver `audio_bridge`.
                if let Some(ui) = ui_weak.upgrade() {
                    let st = crate::BeatState::get(&ui);
                    st.set_playhead(-1);
                    ui.set_transport_time_seconds(ret_secs as f32);
                    ui.set_transport_position(ret_ticks.min(i32::MAX as u64) as i32);
                }
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
            let secs;
            {
                let mut t = transport.lock().unwrap();
                if t.bpm == bpm {
                    return;
                }
                t.bpm = bpm;
                // Mesma posição musical, novo mapeamento em segundos.
                secs = t.position_seconds();
            }
            // Re-ancora o clock visual no novo mapeamento, mantendo a
            // posição projetada contínua. Em TELA a agulha não salta:
            // px/seg ∝ bpm cancela com seg/beat ∝ 1/bpm (x depende só
            // da posição musical). Publica já os novos segundos junto
            // com o novo BPM para não haver frame de transição.
            if let Some(c) = visual_clock.borrow_mut().as_mut() {
                c.resync(secs);
            }
            if let Some(ui) = ui_weak_b.upgrade() {
                ui.set_bpm(bpm as i32);
                ui.set_transport_time_seconds(secs as f32);
            }
        });
    }

    // ---- SEEK -------------------------------------------------------
    {
        let transport = transport.clone();
        let ui_weak_s = ui_weak.clone();
        let visual_clock = visual_clock.clone();
        ui.on_seek_to(move |beat| {
            let (pos_ticks, pos_secs) = {
                let mut t = transport.lock().unwrap();
                // Clique na régua: pixel → beat exato, SEM snap.
                let ticks = (beat.max(0.0) as f64 * t.ppq as f64) as u64;
                t.seek(ticks);
                (t.position_ticks, t.position_seconds())
            };
            // Re-ancora o clock visual: agulha salta imediatamente,
            // sem esperar o timer detectar o drift.
            if let Some(c) = visual_clock.borrow_mut().as_mut() {
                c.resync(pos_secs);
            }
            if let Some(ui) = ui_weak_s.upgrade() {
                ui.set_transport_time_seconds(pos_secs as f32);
                ui.set_transport_position(pos_ticks.min(i32::MAX as u64) as i32);
            }
        });
    }
}
