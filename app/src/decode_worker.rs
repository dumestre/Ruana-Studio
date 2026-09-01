//! Decode e rasterização de áudio em THREADS DE FUNDO.
//!
//! Reúne num só lugar todo o processamento pesado que NÃO pode rodar
//! na thread da UI nem na thread de áudio:
//!
//! - Decodificação (symphonia/hound) de arquivos de áudio.
//! - Construção da waveform (picos, display wave, mipmap, imagem RGBA).
//! - Rasterização de waveform na resolução exata pedida pela UI.
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
//! (`DecodedWave`, `Vec<u8>`) são puros e `Send`. Os callbacks capturam
//! `Rc`, mas só executam na thread da UI.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use daw_media::waveform_image::{Mipmap, WaveformLods};

/// Intervalo de drenagem dos resultados na event loop.
const DRAIN_MS: Duration = Duration::from_millis(16);

// ============================================================
// TIPOS DE RESULTADO (puros, Send)
// ============================================================

/// Áudio decodificado + waveform agregada.
pub struct DecodedWave {
    pub name: String,
    pub path: String,
    pub duration_seconds: f32,
    pub format: String,
    pub interleaved: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub peaks: Vec<f32>,
    pub mono_ds: Vec<f32>,
    pub mip: Mipmap,
    pub lods: WaveformLods,
    pub image_rgba: Option<Vec<u8>>,
}

/// Resultado do import: áudio decodificado + silêncio aparado + TODA a
/// waveform pronta (peaks, display, mipmap, lods, imagem). Computado na
/// worker; a UI só faz a mutação de documento (barata).
pub struct ImportedWave {
    pub name: String,
    pub length_beats: f32,
    pub source_start_beats: f32,
    pub peaks: Vec<f32>,
    pub mono_ds: Vec<f32>,
    pub mip: Mipmap,
    pub lods: WaveformLods,
    pub image_rgba: Option<Vec<u8>>,
}

// ============================================================
// FILA GLOBAL DE TRABALHOS (worker thread)
// ============================================================

struct DecodeJob {
    path: String,
    bpm: f32,
    points: usize,
    with_image: bool,
    tx: mpsc::Sender<Option<DecodedWave>>,
}

struct RasterJob {
    mono: Vec<f32>,
    mip: Mipmap,
    w: u32,
    h: u32,
    gain: f32,
    fi: f32,
    fo: f32,
    tx: mpsc::Sender<Option<Vec<u8>>>,
}

struct ImportJob {
    path: String,
    bpm: f32,
    tx: mpsc::Sender<Option<ImportedWave>>,
}

enum Job {
    Decode(DecodeJob),
    Raster(RasterJob),
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
        Job::Raster(job) => {
            let out = raster_wave(&job);
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

    let bpm = if job.bpm > 0.0 { job.bpm } else { 120.0 };
    let length_beats = (decoded.duration_seconds * bpm / 60.0).max(0.01);
    let points = if job.points > 0 {
        job.points
    } else {
        ((length_beats * 24.0) as usize).clamp(256, 2048)
    };

    let peaks = daw_media::waveform::waveform_points(&decoded, points);
    let (mono_ds, mip) = daw_media::waveform_image::build_display_wave(&decoded.samples);
    let lods = daw_media::waveform_image::build_waveform_lods(&mono_ds);
    let image_rgba = if job.with_image {
        daw_media::waveform_image::waveform_image_rgba(&peaks, 0, false)
            .filter(|v| !v.is_empty())
    } else {
        None
    };

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
        peaks,
        mono_ds,
        mip,
        lods,
        image_rgba,
    })
}

fn raster_wave(job: &RasterJob) -> Option<Vec<u8>> {
    let mut rgba = daw_media::waveform_image::waveform_image_rgba_mip(
        &job.mono,
        &job.mip,
        job.w,
        job.h,
        (0, job.mono.len()),
    )?;
    apply_gain_and_fade(&mut rgba, job.w, job.h, job.gain, job.fi, job.fo);
    Some(rgba)
}

/// Import: decodifica, apara silêncio nas bordas e agrega TODA a
/// waveform (peaks, display wave, mipmap, lods, imagem) na worker.
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

    let points = ((length_beats * 24.0) as usize).clamp(256, 2048);

    // Construa uma WaveformData "aparada" para picos/mono corretos.
    let trimmed_data = daw_media::waveform::WaveformData {
        samples: trimmed,
        display_samples: Vec::new(),
        mipmap: daw_media::waveform_image::Mipmap { levels: Vec::new() },
        interleaved: if end == total && start == 0 {
            decoded.interleaved.clone()
        } else {
            decoded.interleaved[start * ch..end * ch].to_vec()
        },
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        duration_seconds,
    };
    let peaks = daw_media::waveform::waveform_points(&trimmed_data, points);
    let (mono_ds, mip) =
        daw_media::waveform_image::build_display_wave(&trimmed_data.samples);
    let lods = daw_media::waveform_image::build_waveform_lods(&mono_ds);
    let image_rgba = daw_media::waveform_image::waveform_image_rgba(&peaks, 0, false)
        .filter(|v| !v.is_empty());

    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();

    Some(ImportedWave {
        name,
        length_beats,
        source_start_beats,
        peaks,
        mono_ds,
        mip,
        lods,
        image_rgba,
    })
}

/// Aplica ganho linear e fades no RGBA (cópia pura da lógica da UI).
fn apply_gain_and_fade(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    gain_linear: f32,
    fade_in_px: f32,
    fade_out_px: f32,
) {
    let gain = gain_linear.clamp(0.0, 4.0);
    let w = w as usize;
    let h = h as usize;
    if w == 0 || h == 0 {
        return;
    }
    let fi = fade_in_px.clamp(0.0, w as f32) as usize;
    let fo = fade_out_px.clamp(0.0, w as f32) as usize;
    let fo_start = w.saturating_sub(fo);
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            if idx + 3 >= rgba.len() {
                continue;
            }
            let mut factor = 1.0;
            if fi > 0 && x < fi {
                factor *= (x as f32 / fi as f32).clamp(0.0, 1.0);
            }
            if fo > 0 && x >= fo_start {
                factor *= ((w - x) as f32 / fo as f32).clamp(0.0, 1.0);
            }
            let sample = factor * gain;
            rgba[idx] = (rgba[idx] as f32 * sample).clamp(0.0, 255.0) as u8;
            rgba[idx + 1] = (rgba[idx + 1] as f32 * sample).clamp(0.0, 255.0) as u8;
            rgba[idx + 2] = (rgba[idx + 2] as f32 * sample).clamp(0.0, 255.0) as u8;
        }
    }
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
    Raster {
        rx: mpsc::Receiver<Option<Vec<u8>>>,
        on_ready: Box<dyn FnOnce(Option<Vec<u8>>) + 'static>,
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
    /// Agenda decode + agregado de waveform no fundo.
    ///
    /// `on_ready(Option<DecodedWave>)` roda NA THREAD DA UI.
    pub fn schedule_decode(
        &self,
        path: &Path,
        bpm: f32,
        points: usize,
        with_image: bool,
        on_ready: impl FnOnce(Option<DecodedWave>) + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Option<DecodedWave>>();
        let _ = sender().send(Job::Decode(DecodeJob {
            path: path.to_string_lossy().to_string(),
            bpm,
            points,
            with_image,
            tx,
        }));
        self.queue.borrow_mut().push_back(Pending::Decode {
            rx,
            on_ready: Box::new(on_ready),
        });
        self.pump();
    }

    /// Agenda uma rasterização de waveform (resolução exata pedida).
    ///
    /// `on_ready(Option<Vec<u8>>)` roda NA THREAD DA UI.
    pub fn schedule_raster(
        &self,
        mono: Vec<f32>,
        mip: Mipmap,
        w: u32,
        h: u32,
        gain: f32,
        fi: f32,
        fo: f32,
        on_ready: impl FnOnce(Option<Vec<u8>>) + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
        let _ = sender().send(Job::Raster(RasterJob {
            mono,
            mip,
            w,
            h,
            gain,
            fi,
            fo,
            tx,
        }));
        self.queue.borrow_mut().push_back(Pending::Raster {
            rx,
            on_ready: Box::new(on_ready),
        });
        self.pump();
    }

    /// Agenda um import completo (decode + aparar silêncio + waveform).
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
                        Pending::Raster { rx, on_ready } => match rx.try_recv() {
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
