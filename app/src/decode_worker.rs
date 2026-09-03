//! Decode de áudio em THREAD DE FUNDO.
//!
//! Reúne num só lugar todo o processamento pesado que NÃO pode rodar
//! na thread da UI nem na thread de áudio:
//!
//! - Decodificação (symphonia/hound) de arquivos de áudio.
//! - Construção da waveform LOD multinível (barras min/max por coluna).
//!
//! # Modelo
//!
//! Há UMA worker thread (`ruana-decode`) consumindo uma fila global.
//! A thread da UI agenda um trabalho com um `mpsc::Receiver` e um
//! callback tipado; um drainer (Timer na event loop) drena os
//! resultados e chama o callback NA THREAD DA UI — tocar em modelos
//! Slint / core / engine só acontece na event loop.
//!
//! A worker NÃO tem `Rc` da UI: os dados que cruzam a thread
//! (`DecodedWave`, `ImportedWave`) são puros e `Send`. Os callbacks capturam
//! `Rc`, mas só executam na thread da UI.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use crate::waveform::{LodBars, build_lods};

/// Intervalo de drenagem dos resultados na event loop.
const DRAIN_MS: Duration = Duration::from_millis(16);

// ============================================================
// TIPOS DE RESULTADO (puros, Send)
// ============================================================

/// Áudio decodificado + waveform agregada.
///
/// Resultado de um decode: samples mono.
/// Os campos waveform-image/mip/lods foram removidos — o LOD inteiro
/// é construído no cache de timeline (WaveformCache) a partir de `samples`.
#[allow(dead_code)]
pub struct DecodedWave {
    pub name: String,
    pub path: String,
    pub duration_seconds: f32,
    pub format: String,
    pub interleaved: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

/// Resultado do import: áudio decodificado + silêncio aparado + LODs.
///
/// Computado na worker; a UI só faz a mutação de documento (barata).
/// O cache de timeline (WaveformCache) recebe estes LODs prontos.
pub struct ImportedWave {
    pub name: String,
    pub length_beats: f32,
    pub source_start_beats: f32,
    /// LOD multinível (barras min/max por coluna).
    pub lods: Vec<LodBars>,
    /// Total de quadros (samples mono) deste trecho aparado.
    pub frames: usize,
}

// ============================================================
// FILA GLOBAL DE TRABALHOS (worker thread)
// ============================================================

struct DecodeJob {
    path: String,
    bpm: f32,
    points: usize,
    tx: mpsc::Sender<Option<DecodedWave>>,
}

struct ImportJob {
    path: String,
    bpm: f32,
    tx: mpsc::Sender<Option<ImportedWave>>,
}

enum Job {
    Decode(DecodeJob),
    Import(ImportJob),
}

fn sender() -> &'static mpsc::Sender<Job> {
    static SENDER: std::sync::OnceLock<mpsc::Sender<Job>> = std::sync::OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("ruana-decode".into())
            .spawn(move || {
                for job in rx {
                    run_job(job);
                }
            })
            .expect("falha ao criar worker de decode");
        tx
    })
}

fn run_job(job: Job) {
    match job {
        Job::Decode(job) => {
            let out = decode_wave(&job);
            let _ = job.tx.send(out);
        }
        Job::Import(job) => {
            let out = import_wave(&job);
            let _ = job.tx.send(out);
        }
    }
}

fn decode_wave(job: &DecodeJob) -> Option<DecodedWave> {
    let path = std::path::PathBuf::from(&job.path);
    let decoded = daw_media::waveform::decode_audio(&path).ok()?;

    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();

    Some(DecodedWave {
        name,
        path: job.path.clone(),
        duration_seconds: decoded.duration_seconds,
        format,
        interleaved: decoded.interleaved,
        channels: decoded.channels as usize,
        sample_rate: decoded.sample_rate,
        samples: decoded.samples,
    })
}

/// Import: decodifica, apara silêncio nas bordas e constrói LODs.
///
/// Espelha a lógica de `app::importer::import_audio_file` para que a
/// thread da UI faça só a mutação de documento.
fn import_wave(job: &ImportJob) -> Option<ImportedWave> {
    const THRESHOLD: f32 = 0.001;
    const MIN_TRIM_SECS: f32 = 0.05;

    let path = std::path::PathBuf::from(&job.path);
    let decoded = daw_media::waveform::decode_audio(&path).ok()?;
    let bpm = if job.bpm > 0.0 { job.bpm } else { 120.0 };

    // ---- aparar silêncio nas bordas (mesma regra do import síncrono)
    let ch = decoded.channels.max(1) as usize;
    let frames = decoded.frames();
    let audible = |f: usize| {
        let base = f * ch;
        (0..ch).any(|c| decoded.interleaved[base + c].abs() > THRESHOLD)
    };

    let (start, end, total) = if frames == 0 {
        return None;
    } else if let Some(first) = (0..frames).find(|&f| audible(f)) {
        let last = (0..frames).rev().find(|&f| audible(f)).unwrap_or(first);
        let lead = first;
        let trail = frames.saturating_sub(last + 1);
        let min_trim = (MIN_TRIM_SECS * decoded.sample_rate as f32) as usize;
        if lead < min_trim && trail < min_trim {
            (0usize, frames, frames)
        } else {
            let pad = ((0.002 * decoded.sample_rate as f32) as usize).max(1);
            let start = first.saturating_sub(pad);
            let end = (last + 1 + pad).min(frames);
            if end - start < 2 {
                (0usize, frames, frames)
            } else {
                (start, end, frames)
            }
        }
    } else {
        // Arquivo todo em silêncio: usa o arquivo inteiro.
        (0usize, frames, frames)
    };

    let trimmed = if end == total && start == 0 {
        decoded.samples.clone()
    } else {
        decoded.samples[start..end].to_vec()
    };
    let duration_seconds = if end > start {
        (end - start) as f32 / decoded.sample_rate as f32
    } else {
        decoded.duration_seconds
    };
    let length_beats = (duration_seconds * bpm / 60.0).max(0.01);
    let source_start_beats = {
        let secs = if end == total && start == 0 {
            0.0
        } else {
            start as f32 / decoded.sample_rate as f32
        };
        (secs * bpm / 60.0).max(0.0)
    };

    // Constrói LODs a partir das samples aparadas.
    let lods = build_lods(&trimmed);
    let frames = trimmed.len();

    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();

    Some(ImportedWave {
        name,
        length_beats,
        source_start_beats,
        lods,
        frames,
    })
}

// ============================================================
// HANDLE DA UI: fila de resultados + drainer único
// ============================================================

/// Um trabalho em trânsito, com o callback tipado que só roda na UI.
enum Pending {
    Decode {
        rx: mpsc::Receiver<Option<DecodedWave>>,
        on_ready: Box<dyn FnOnce(Option<DecodedWave>) + 'static>,
    },
    Import {
        rx: mpsc::Receiver<Option<ImportedWave>>,
        on_ready: Box<dyn FnOnce(Option<ImportedWave>) + 'static>,
    },
}

/// Handle da thread da UI para agendar e drenar trabalhos.
///
/// Clonável e barato; a fila + timer vivem num `Rc` que só a UI toca.
#[derive(Clone)]
pub struct DecodeWorker {
    queue: Rc<RefCell<VecDeque<Pending>>>,
    timer: Rc<RefCell<Option<slint::Timer>>>,
    active: Rc<Cell<bool>>,
}

impl Default for DecodeWorker {
    fn default() -> Self {
        Self {
            queue: Rc::new(RefCell::new(VecDeque::new())),
            timer: Rc::new(RefCell::new(None)),
            active: Rc::new(Cell::new(false)),
        }
    }
}

impl DecodeWorker {
    /// Agenda decode + waveform no fundo.
    ///
    /// `on_ready(Option<DecodedWave>)` roda NA THREAD DA UI.
    pub fn schedule_decode(
        &self,
        path: &Path,
        bpm: f32,
        points: usize,
        on_ready: impl FnOnce(Option<DecodedWave>) + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Option<DecodedWave>>();
        let _ = sender().send(Job::Decode(DecodeJob {
            path: path.to_string_lossy().to_string(),
            bpm,
            points,
            tx,
        }));
        self.queue.borrow_mut().push_back(Pending::Decode {
            rx,
            on_ready: Box::new(on_ready),
        });
        self.pump();
    }

    /// Agenda um import completo (decode + aparar silêncio + LODs).
    ///
    /// `on_ready(Option<ImportedWave>)` roda NA THREAD DA UI.
    pub fn schedule_import(
        &self,
        path: &Path,
        bpm: f32,
        on_ready: impl FnOnce(Option<ImportedWave>) + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Option<ImportedWave>>();
        let _ = sender().send(Job::Import(ImportJob {
            path: path.to_string_lossy().to_string(),
            bpm,
            tx,
        }));
        self.queue.borrow_mut().push_back(Pending::Import {
            rx,
            on_ready: Box::new(on_ready),
        });
        self.pump();
    }

    /// Garante o drainer único rodando (já rodando = só enfileira).
    fn pump(&self) {
        if self.active.get() {
            return;
        }
        self.active.set(true);

        let queue = self.queue.clone();
        let timer_holder = self.timer.clone();
        let timer_rc = self.timer.clone();
        let active = self.active.clone();

        let timer = slint::Timer::default();
        *timer_holder.borrow_mut() = Some(timer);
        if let Some(t) = timer_holder.borrow().as_ref() {
            t.start(slint::TimerMode::Repeated, DRAIN_MS, move || {
                loop {
                    let Some(mut pending) = queue.borrow_mut().pop_front() else {
                        break;
                    };
                    match &mut pending {
                        Pending::Decode { rx, on_ready } => match rx.try_recv() {
                            Ok(ready) => {
                                let f = std::mem::replace(on_ready, Box::new(|_| {}));
                                f(ready);
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                                queue.borrow_mut().push_front(pending);
                                break;
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {}
                        },
                        Pending::Import { rx, on_ready } => match rx.try_recv() {
                            Ok(ready) => {
                                let f = std::mem::replace(on_ready, Box::new(|_| {}));
                                f(ready);
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                                queue.borrow_mut().push_front(pending);
                                break;
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {}
                        },
                    }
                }
                if queue.borrow().is_empty() {
                    if let Some(t) = timer_rc.borrow().as_ref() {
                        t.stop();
                    }
                    active.set(false);
                }
            });
        }
    }
}