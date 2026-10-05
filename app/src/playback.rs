//! Playback: liga o transporte (core) à UI.
//!
//! O engine de áudio avança o `Transport` na thread RT (clock
//! autoritativo). Aqui vive o clock VISUAL do playhead: um Timer
//! dentro da event loop (~8ms) que lê o transport e EXTRAPOLA a
//! posição por tempo real (`Instant`) entre os ticks de áudio,
//! publicando SEGUNDOS contínuos (`transport-time-seconds`) — o
//! domínio da Playhead. Ticks inteiros seguem só para o LCD.
//!
//! Projeção PURA (estilo PlayheadLab): a posição avança
//! linearmente a partir da âncora (`anchor + elapsed`), sem servo
//! de posição por tick — cada micro-correção à âncora vira
//! micro-não-linearidade no movimento, que é o que a olho percebe
//! como "pulando". A âncora só é re-ancorada em saltos intencionais
//! (play/seek/bpm) ou num drift real de áudio (> 0.25 s). O passo do
//! beat deriva do MESMO relógio, então a coluna acesa continua
//! alinhada com a agulha.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use daw_core::project::Project;
use daw_core::transport::{PlayMode, Transport};
use slint::{ComponentHandle, Global};

use crate::MainWindow;
use crate::audio_bridge::AudioBridge;
use crate::meters::MeterPump;

/// Log de runtime (playback) — só ativo com `RUANA_LOG` definido.
fn rt_log(msg: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var("RUANA_LOG").is_ok()) {
        eprintln!("{}", msg);
    }
}

/// Clock visual do playhead: âncora `(segundos, Instant)`.
///
/// A Playhead vive no domínio de TEMPO CONTÍNUO (segundos), nunca
/// em beats/ticks: a projeção avança à taxa 1 s/s (o clock de áudio
/// também avança em tempo real) e o valor real só corrige a âncora
/// em saltos intencionais. A conversão segundos → pixels acontece na
/// UI: `playhead_x = segundos * pixels_per_second`.
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
    ///
    /// Função LINEAR do tempo: sem correção de posição por tick, o
    /// movimento da agulha é uniforme (mesma geometria do PlayheadLab).
    /// deriva vs o relógio de áudio é da ordem de ppm (~ms/minuto),
    /// irrelevante em escala de música; o gate do timer re-ancora se
    /// houver um glitch real de áudio.
    fn projected(&self) -> f64 {
        self.anchor_secs + self.anchor_at.elapsed().as_secs_f64()
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
    meter_pump: Rc<RefCell<MeterPump>>,
    audio: AudioBridge,
) {
    // Handlers precisam da instância forte; o Weak é só para os
    // closures dos timers (evita ciclo com a event loop).
    let Some(ui) = ui.upgrade() else { return };
    let ui_weak = ui.as_weak();

    let meter_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let playhead_timer = Rc::new(RefCell::new(slint::Timer::default()));
    let visual_clock: Rc<RefCell<Option<VisualClock>>> = Rc::new(RefCell::new(None));
    // O que está no ar agora (`None` = parado). Substitui o antigo
    // `is_playing`: não basta saber que "toca", é preciso saber O QUÊ,
    // porque trocar de modo é uma troca de conteúdo, não um no-op.
    let mode_now: Rc<Cell<Option<PlayMode>>> = Rc::new(Cell::new(None));

    // ---- METERS ------------------------------------------------------
    //
    // BOMBEAMENTO PERMANENTE, fora do play/stop. O medidor mede o que
    // o DISPOSITIVO está entregando, não o que o transporte mandou:
    // pré-escuta de sample, soltar um clip e o crossfade de loop soam
    // com o play parado, e um VU congelado nesses momentos mente. Como
    // o envelope decai sozinho, parar o transporte só faz a barra
    // cair — que é exatamente o que se vê.
    {
        let project = project.clone();
        let meter_pump = meter_pump.clone();
        let audio = audio.clone();
        meter_timer.borrow().start(
            slint::TimerMode::Repeated,
            crate::meters::PUMP_INTERVAL,
            move || {
                // `try_borrow`: se o documento estiver emprestado no
                // meio de uma mutação, este tick é pulado em vez de
                // entrar em pânico.
                let Ok(proj) = project.try_borrow() else { return };
                meter_pump
                    .borrow_mut()
                    .tick(&audio, proj.tracks.len() + 1);
            },
        );
    }

    // ---- PLAY / STOP ------------------------------------------------
    //
    // Um único controlador serve aos DOIS gatilhos de play:
    //  • play global (`play-state-changed`) →
    //    `PlayMode::Timeline`: clips de áudio e MIDI tocam, o playhead
    //    anda e o step sequencer fica MUDO.
    //  • play do beat (`beat-play-state-changed`) → `PlayMode::BeatOnly`:
    //    o pattern soa e o grid acende, mas a timeline fica parada
    //    (nenhum clip toca e o playhead não anda).
    //
    // O play isolado do beat vem do botão no header; o play global
    // reproduz os clips da timeline. Os modos são EXCLUSIVOS e o
    // motor de áudio tem UM transporte só:
    // pedir play no outro modo enquanto este toca é uma TROCA (stop +
    // start), nunca uma sobreposição. Por isso o estado local é
    // `Option<PlayMode>` e não um booleano.
    {
        let transport = transport.clone();

        // ---- START ------------------------------------------------
        let start: Rc<dyn Fn(PlayMode)> = {
            let ui_weak_s = ui_weak.clone();
            let transport = transport.clone();
            let project = project.clone();
            let visual_clock = visual_clock.clone();
            let playhead_timer = playhead_timer.clone();
            let audio = audio.clone();
            let mode_now = mode_now.clone();

            Rc::new(move |mode: PlayMode| {
                mode_now.set(Some(mode));

                // O snapshot precisa estar publicado ANTES do primeiro
                // buffer tocar, senão o começo sai em silêncio.
                audio.flush_now(&project.borrow());

                // Luz de clip zerada a cada play: o latch lembra que a
                // ÚLTIMA vez estourou, e sem isso o vermelho de uma
                // tomada antiga continuaria aceso na sessão seguinte.
                audio.clear_meter_clips();

                {
                    let mut t = transport.lock().unwrap();
                    t.playing = true;
                    t.mode = mode;
                    // Lembra onde o play começou: STOP devolve a agulha
                    // pra cá (comportamento padrão de DAW).
                    t.play_start_ticks = t.position_ticks;
                    // Âncora inicial do clock visual (segundos contínuos)
                    *visual_clock.borrow_mut() = Some(VisualClock {
                        anchor_secs: t.position_seconds(),
                        anchor_at: Instant::now(),
                    });
                }
                rt_log(match mode {
                    PlayMode::BeatOnly => "[PLAY] beat-only: pattern soa, timeline parada",
                    PlayMode::Timeline => "[PLAY] timeline: clips + playhead, step sequencer mudo",
                });

                // O Rust é a fonte da verdade dos DOIS flags de UI:
                // um play que trocou de modo tem que desmarcar o
                // botão do outro transporte, senão os ícones mentem.
                if let Some(ui) = ui_weak_s.upgrade() {
                    ui.set_playing(matches!(mode, PlayMode::Timeline));
                    ui.set_beat_playing(matches!(mode, PlayMode::BeatOnly));
                }

                // Playhead timer: roda NA event loop. Lê o clock
                // autoritativo (áudio) via try_lock e projeta por tempo
                // real (projeção pura). Publica SEGUNDOS (float, sem
                // quantização) para a agulha e ticks inteiros apenas para
                // o LCD. No modo "só beat" NÃO publica posição (timeline
                // parada), mas continua avançando o passo do beat.
                {
                    let ui_weak_t = ui_weak_s.clone();
                    let transport = transport.clone();
                    let clock = visual_clock.clone();
                    let last_lcd = Cell::new(i32::MIN);
                    let last_step = Cell::new(i32::MIN);
                    playhead_timer.borrow().start(
                        slint::TimerMode::Repeated,
                        Duration::from_millis(4),
                        move || {
                            let mut guard = clock.borrow_mut();
                            let Some(c) = guard.as_mut() else { return };
                            let mut lcd_ticks: Option<i64> = None;
                            let mut bpm = 120.0f64;
                            let mut beat_only = false;
                            if let Ok(t) = transport.try_lock() {
                                if !t.playing {
                                    return;
                                }
                                beat_only = t.mode == PlayMode::BeatOnly;
                                bpm = t.bpm as f64;
                                let actual = t.position_seconds();
                                let drift = actual - c.projected();
                                if drift.abs() > 0.25 {
                                    // Glitch real (dropout/Linux): re-ancora
                                    // na posição verdadeira. Fora isso, projeção
                                    // pura — nenhuma micro-correção por tick.
                                    c.resync(actual);
                                }
                                // LCD (bars:beat:ticks) — domínio musical,
                                // derivado dos segundos com o BPM atual.
                                lcd_ticks = Some(
                                    (c.projected().max(0.0) * t.bpm as f64 / 60.0 * t.ppq as f64)
                                        as i64,
                                );
                            }
                            let secs = c.projected().max(0.0);
                            drop(guard);
                            if let Some(ui) = ui_weak_t.upgrade() {
                                // Modo "só beat": o playhead da timeline
                                // NÃO anda (nem o LCD) — só o grid do
                                // beat acende via publish_beat_step.
                                if !beat_only {
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
                                }
                                // Passo do beat sequencer: derivado do
                                // MESMO relógio do áudio, então a coluna
                                // acesa é a que está soando (antes era um
                                // Timer de 140ms sem relação com nada).
                                // Na timeline o sequenciador está desligado:
                                // o passo fica em -1 (nenhuma coluna acesa).
                                if beat_only {
                                    publish_beat_step(&ui, secs, bpm, &last_step);
                                } else if last_step.replace(-1) != -1 {
                                    crate::BeatState::get(&ui).set_playhead(-1);
                                }
                            }
                        },
                    );
                }
            })
        };

        // ---- STOP -------------------------------------------------
        // Para o que estiver tocando, em qualquer modo (o botão STOP
        // e o espaço são absolutos). A agulha volta pra onde o play
        // começou.
        let stop: Rc<dyn Fn()> = {
            let ui_weak_x = ui_weak.clone();
            let transport = transport.clone();
            let visual_clock = visual_clock.clone();
            let playhead_timer = playhead_timer.clone();
            let mode_now = mode_now.clone();

            Rc::new(move || {
                let was = mode_now.replace(None);
                if was.is_none() {
                    return;
                }
                rt_log(&format!("[STOP] mode={was:?} + timers"));
                playhead_timer.borrow().stop();
                *visual_clock.borrow_mut() = None;

                // STOP: agulha volta pra onde o play começou.
                let (ret_ticks, ret_secs) = {
                    let mut t = transport.lock().unwrap();
                    t.pause();
                    t.position_ticks = t.play_start_ticks;
                    (t.position_ticks, t.position_seconds())
                };
                // O dispositivo de áudio NÃO fecha aqui: continua
                // aberto para a pré-escuta de samples com o transporte
                // parado. Ver `audio_bridge`.
                if let Some(ui) = ui_weak_x.upgrade() {
                    let st = crate::BeatState::get(&ui);
                    st.set_playhead(-1);
                    ui.set_playing(false);
                    ui.set_beat_playing(false);
                    ui.set_transport_time_seconds(ret_secs as f32);
                    ui.set_transport_position(ret_ticks.min(i32::MAX as u64) as i32);
                }
            })
        };

        // `play` = quer tocar; `mode` = o quê. Pedir play já no mesmo
        // modo é no-op (espaço repetido não reinicia os timers);
        // pedir no outro modo troca.
        let control = move |play: bool, mode: PlayMode| {
            let current = mode_now.get();
            match (play, current) {
                (true, Some(current)) if current == mode => {}
                (true, Some(_)) => {
                    stop();
                    start(mode);
                }
                (true, None) => start(mode),
                (false, Some(_)) => stop(),
                (false, None) => {}
            }
        };

        let control = Rc::new(RefCell::new(Some(control)));

        let c = control.clone();
        ui.on_play_state_changed(move |playing| {
            if let Some(f) = c.borrow_mut().as_mut() {
                f(playing, PlayMode::Timeline);
            }
        });

        let c = control.clone();
        ui.on_beat_play_state_changed(move |playing| {
            if let Some(f) = c.borrow_mut().as_mut() {
                f(playing, PlayMode::BeatOnly);
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
