//! Load/drop de sample no sampler com decode em THREAD DE FUNDO.
//!
//! O handler da UI só agenda (pre-checks baratos); a worker decodifica
//! e devolve metadados + picos num canal; um drainer (timer de 16ms)
//! aplica TUDO na event loop — tocar no `Project`, no `AudioBridge` e
//! no modelo Slint só acontece na thread da UI.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{Timer, TimerMode};

use daw_core::project::Project;

use crate::MainWindow;
use crate::audio_bridge::AudioBridge;
use crate::beat;

/// Metadados + picos prontos (puro; atravessa a thread sem Rc da UI).
pub struct SamplerReady {
    pub name: String,
    pub format: String,
    pub duration: String,
    pub peaks: Vec<f32>,
}

/// Origem do pedido: muda a linha-alvo no apply.
#[derive(Clone, Copy, PartialEq)]
pub enum SamplerSource {
    /// Carga via diálogo: linha = parâmetro `track` do pedido.
    Load,
    /// Drop (explorador/SO): linha = a selecionada no momento do apply.
    Drop,
}

struct PendingJob {
    rx: mpsc::Receiver<Option<SamplerReady>>,
    ui: slint::Weak<MainWindow>,
    project: Rc<RefCell<Project>>,
    audio: AudioBridge,
    track: i32,
    path: String,
    source: SamplerSource,
}

/// Decode agendado: fila de trabalhos + um drainer que vive só
/// enquanto houver decodes em aberto.
#[derive(Clone)]
pub struct SamplerDecode {
    timer: Rc<RefCell<Option<Timer>>>,
    jobs: Rc<RefCell<VecDeque<PendingJob>>>,
    in_flight: Rc<Cell<usize>>,
}

impl Default for SamplerDecode {
    fn default() -> Self {
        SamplerDecode {
            timer: Rc::new(RefCell::new(None)),
            jobs: Rc::new(RefCell::new(VecDeque::new())),
            in_flight: Rc::new(Cell::new(0)),
        }
    }
}

impl SamplerDecode {
    /// Agenda o decode de `path`. `display_name` já vem resolvido
    /// (stem no load, display name no drop). Retorna na hora.
    pub fn schedule(
        &self,
        ui: &slint::Weak<MainWindow>,
        project: Rc<RefCell<Project>>,
        audio: AudioBridge,
        track: i32,
        path: std::path::PathBuf,
        source: SamplerSource,
        display_name: String,
    ) {
        self.in_flight.set(self.in_flight.get() + 1);

        let (tx, rx) = mpsc::channel::<Option<SamplerReady>>();
        let path_bg = path.clone();
        std::thread::spawn(move || {
            let ready = decode_sampler_ready(&path_bg, display_name);
            let _ = tx.send(ready);
        });

        self.jobs.borrow_mut().push_back(PendingJob {
            rx,
            ui: ui.clone(),
            project,
            audio,
            track,
            path: path.to_string_lossy().to_string(),
            source,
        });

        // Garante o drainer rodando (já rodando = só enfileira).
        let running = self
            .timer
            .borrow()
            .as_ref()
            .map(|t| t.running())
            .unwrap_or(false);
        if !running {
            let timer = Timer::default();
            *self.timer.borrow_mut() = Some(timer);
            if let Some(t) = self.timer.borrow().as_ref() {
                let jobs = self.jobs.clone();
                let in_flight = self.in_flight.clone();
                let timer_rc = self.timer.clone();
                t.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                    let mut done = false;
                    loop {
                        let job = match jobs.borrow_mut().pop_front() {
                            Some(j) => j,
                            None => break,
                        };
                        match job.rx.try_recv() {
                            Ok(ready) => {
                                done = true;
                                in_flight.set(in_flight.get().saturating_sub(1));
                                if let Some(ready) = ready {
                                    apply_ready(ready, job);
                                }
                            }
                            Err(_) => {
                                // Ainda decodificando: devolve pro fim e
                                // não gira em círculo nesta passada.
                                jobs.borrow_mut().push_back(job);
                                break;
                            }
                        }
                    }
                    if done && jobs.borrow().is_empty() && in_flight.get() == 0 {
                        if let Some(t) = timer_rc.borrow().as_ref() {
                            t.stop();
                        }
                    }
                });
            }
        }
    }
}

/// Decodifica + agrega os picos do painel (worker thread; puro).
fn decode_sampler_ready(path: &Path, name: String) -> Option<SamplerReady> {
    let decoded = daw_media::waveform::decode_audio(path).ok()?;
    let peaks = daw_media::waveform::waveform_points(&decoded, 512);
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();
    let duration = format!("{:.2}s", decoded.duration_seconds);
    Some(SamplerReady {
        name,
        format,
        duration,
        peaks,
    })
}

/// Aplica na thread da UI (mesma sequência do handler síncrono antigo).
fn apply_ready(ready: SamplerReady, job: PendingJob) {
    let Some(ui) = job.ui.upgrade() else {
        return;
    };

    ui.set_sampler_sample_name(ready.name.clone().into());
    ui.set_sampler_sample_loaded(true);
    ui.set_sampler_sample_format(ready.format.clone().into());
    ui.set_sampler_sample_duration(ready.duration.clone().into());
    ui.set_sampler_sample_peaks(Rc::new(slint::VecModel::from(ready.peaks)).into());

    // Linha-alvo: drop grava na LINHA SELECIONADA na hora do apply.
    let index = if job.source == SamplerSource::Drop {
        ui.get_beat_selected_track().max(0) as usize
    } else {
        job.track.max(0) as usize
    };

    {
        let mut proj = job.project.borrow_mut();
        if let Some(pattern) = proj.current_beat_pattern_mut() {
            if let Some(params) = pattern.track_params.get_mut(index) {
                params.sample_path = Some(job.path.clone());
                params.name = ready.name.clone();
            }
        }
        beat::refresh_beat(&ui, &proj);
    }

    job.audio.preload(&job.path);
    job.audio.mark_dirty();
    job.audio.preview(&job.path, 0.9, 0.0);
}