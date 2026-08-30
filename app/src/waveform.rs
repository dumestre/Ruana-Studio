// Cache e rasterização de waveform para a camada Slint.
//
// O cache guarda `(picos, imagem RGBA)` por clip; a imagem é
// gerada UMA vez por import e re-renderizada na resolução exata
// quando o zoom muda (`render_waveform_at`).
use crate::MainWindow;

use std::rc::Rc;

use crate::ClipData;
use daw_core::clip::Clip;

/// Dados de waveform em cache por clip.
#[derive(Clone)]
pub struct CachedWave {
    /// Picos agregados (fallback / compatibilidade).
    pub peaks: Vec<f32>,
    /// Downmix mono de EXIBIÇÃO (downsample com teto) — fonte do
    /// re-render por zoom e do modo osciloscópio.
    pub mono_ds: Vec<f32>,
    /// Pirâmide MIN/MAX/RMS sobre `mono_ds` — zoom sem re-varredura.
    pub mip: daw_media::waveform_image::Mipmap,
    /// Imagem pré-renderizada na resolução do último pedido.
    pub image: Option<Vec<u8>>,
}

/// Cache de waveform: clip_id → dados completos.
pub type WaveformCache = std::collections::HashMap<u64, CachedWave>;

/// Converte RGBA cru + dimensões num `slint::Image`.
fn image_from_rgba(rgba: &[u8], w: u32, h: u32) -> slint::Image {
    use slint::Rgba8Pixel;
    let buf = slint::SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, w, h);
    slint::Image::from_rgba8(buf)
}

/// Converte o RGBA cru do cache num `slint::Image`.
/// Largura 0 = sem imagem (fallback de retângulos na UI).
pub fn waveform_image_to_slint(rgba: Option<Vec<u8>>) -> slint::Image {
    use slint::Rgba8Pixel;
    match rgba {
        Some(data) => {
            let buf = slint::SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                &data,
                daw_media::waveform_image::WAVEFORM_IMG_WIDTH as u32,
                512,
            );
            slint::Image::from_rgba8(buf)
        }
        None => slint::Image::default(),
    }
}

/// Atualiza apenas os pontos/imagem de um ClipData existente a partir do cache.
pub fn to_clip_data_from_id(id: u64, base: &ClipData, waveforms: &WaveformCache) -> ClipData {
    let (peaks, img) = match waveforms.get(&id) {
        Some(c) => (c.peaks.clone(), c.image.clone()),
        None => (Vec::new(), None),
    };
    ClipData {
        id: base.id,
        name: base.name.clone(),
        track_index: base.track_index,
        beat: base.beat,
        len: base.len,
        is_midi: base.is_midi,
        color: base.color,
        waveform_points: Rc::new(slint::VecModel::from(peaks)).into(),
        waveform_image: waveform_image_to_slint(img),
        waveform_src_x: 0.0,
        waveform_src_w: 0.0,
        multi_selected: base.multi_selected,
    }
}

// Re-render da waveform na resolução exata de exibição.
//
// DEBOUNCE: escrever no modelo dentro do próprio `changed
// width` reconstrói a linha e redispara o handler (loop).
// Guardamos o pedido e aplicamos num timer de 40 ms; se um
// novo pedido chegar, reiniciamos o timer. Também ignoramos
// janelas iguais à última renderizada.
//
// JANELA (zoom fundo): o pedido vem com a janela visível do
// clip em px (x0/x1). Só esse trecho é rasterizado — no zoom
// fundo são poucos samples por pixel (poucas waves), desenhados
// sample a sample: nítido e leve, sem varrer o arquivo inteiro.
// Fora da viewport nada é renderizado (o row já culling faz o
// clip sumir) e a largura da imagem fica ≤ a viewport.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc;

use crate::TimelineMetrics;
use crate::models::clip_row;
use slint::{ComponentHandle, Model};

pub fn wire_resolution_requests(
    ui: &MainWindow,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
) {
    // Re-renderiza a waveform de um clip na resolução exata de
    // exibição (zoom/resize/scroll mudaram largura ou janela).
    //
    // Índice: (1) DEBOUNCE de 40ms no pedido; (2) o raster roda
    // numa THREAD DE FUNDO (a entrada mono_ds+mip é clonada antes
    // de atravessar — a UI nunca espera); (3) bytes voltam por um
    // canal e um timer curto (collector) aplica no modelo dentro
    // da event loop. O zoom deixa de congelar a UI.
    {
        let pending: Rc<RefCell<HashMap<i32, (u32, f32, f32)>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let done_window: Rc<RefCell<HashMap<u64, (u32, u32, u32)>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let debounce: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
        let collector_timer: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
        let in_flight: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        // (id, largura total, largura rasterizada, janela x0, x1, rgba)
        let (tx, rx0) = mpsc::channel::<(i32, u32, u32, f32, f32, Option<Vec<u8>>)>();
        let rx: Rc<RefCell<mpsc::Receiver<(i32, u32, u32, f32, f32, Option<Vec<u8>>)>>> =
            Rc::new(RefCell::new(rx0));

        /// Trabalho que vai para a thread de fundo.
        enum RenderJob {
            Mip {
                mono_ds: Vec<f32>,
                mip: daw_media::waveform_image::Mipmap,
            },
            Peaks { peaks: Vec<f32> },
        }

        ui.global::<TimelineMetrics>()
            .on_waveform_resolution_requested(move |id, width_px, wx0, wx1| {
                let id_u = id.max(0) as u64;
                let w = (width_px.round() as u32).clamp(16, 16384);
                let x0 = wx0.round() as u32;
                let x1 = wx1.round() as u32;

                // Janela já renderizada nessa resolução? Nada a fazer.
                if done_window.borrow().get(&id_u) == Some(&(w, x0, x1)) {
                    return;
                }
                pending.borrow_mut().insert(id, (w, wx0, wx1));

                // Reinicia o timer (debounce do gesto de zoom/scroll).
                let t = slint::Timer::default();
                *debounce.borrow_mut() = Some(t);
                if let Some(t) = debounce.borrow().as_ref() {
                    let pending = pending.clone();
                    let done_window = done_window.clone();
                    let waveforms = waveforms.clone();
                    let clips_model = clips_model.clone();
                    let rx = rx.clone();
                    let tx = tx.clone();
                    let collector_timer = collector_timer.clone();
                    let in_flight = in_flight.clone();
                    t.start(
                        slint::TimerMode::SingleShot,
                        std::time::Duration::from_millis(40),
                        move || {
                            let jobs: Vec<(i32, u32, f32, f32)> = pending
                                .borrow_mut()
                                .drain()
                                .map(|(id, (w_full, wx0, wx1))| (id, w_full, wx0, wx1))
                                .collect();
                            for (id, w_full, wx0, wx1) in jobs {
                                let id_u = id.max(0) as u64;
                                // Largura rasterizada = apenas a janela
                                // visível (≤ viewport). Clona a entrada
                                // ANTES da thread (RefCell não atravessa).
                                let w_px = ((wx1 - wx0).round() as u32).clamp(8, 16384);
                                let job = {
                                    let wf = waveforms.borrow();
                                    match wf.get(&id_u) {
                                        Some(c) if !c.mono_ds.is_empty() => {
                                            let n = c.mono_ds.len();
                                            let f = w_full as f64;
                                            let s0 = ((n as f64) * (wx0 as f64) / f).floor() as usize;
                                            let s1 = ((((n as f64) * (wx1 as f64) / f)
                                                .ceil()) as usize)
                                                .clamp(s0, n);
                                            if s1 > s0 {
                                                Some((
                                                    RenderJob::Mip {
                                                        mono_ds: c.mono_ds.clone(),
                                                        mip: c.mip.clone(),
                                                    },
                                                    (s0, s1),
                                                ))
                                            } else {
                                                None
                                            }
                                        }
                                        Some(c) if !c.peaks.is_empty() => {
                                            let n = c.peaks.len();
                                            let f = w_full as f64;
                                            let p0 = ((n as f64) * (wx0 as f64) / f).floor() as usize;
                                            let p1 = ((((n as f64) * (wx1 as f64) / f)
                                                .ceil()) as usize)
                                                .clamp(p0, n);
                                            if p1 > p0 {
                                                Some((
                                                    RenderJob::Peaks {
                                                        peaks: c.peaks[p0..p1].to_vec(),
                                                    },
                                                    (0, p1 - p0),
                                                ))
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                let Some((job, view)) = job else {
                                    continue;
                                };
                                in_flight.set(in_flight.get() + 1);
                                let w0 = w_px;
                                let tx = tx.clone();
                                std::thread::spawn(move || {
                                    // Raster na thread de fundo: pirâmide
                                    // min/max quando há mono de exibição,
                                    // fallback de picos caso contrário. A
                                    // fatia `view` cobre só a janela.
                                    let rgba = match job {
                                        RenderJob::Mip { mono_ds, mip } => {
                                            daw_media::waveform_image::waveform_image_rgba_mip(
                                                &mono_ds, &mip, w0, view,
                                            )
                                        }
                                        RenderJob::Peaks { peaks } => {
                                            daw_media::waveform_image::waveform_image_rgba_at(
                                                &peaks, w0,
                                            )
                                        }
                                    };
                                    let _ = tx.send((id, w_full, w0, wx0, wx1, rgba));
                                });
                            }

                            // Garante um collector aplicando os bytes na UI.
                            let ct = slint::Timer::default();
                            *collector_timer.borrow_mut() = Some(ct);
                            if let Some(c) = collector_timer.borrow().as_ref() {
                                let rx = rx.clone();
                                let done_window = done_window.clone();
                                let clips_model = clips_model.clone();
                                let in_flight = in_flight.clone();
                                let collector_timer = collector_timer.clone();
                                c.start(
                                    slint::TimerMode::Repeated,
                                    std::time::Duration::from_millis(16),
                                    move || {
                                        let mut acted = false;
                                        loop {
                                            let item = rx.borrow_mut().try_recv();
                                            let (id, w_full, w, wx0, wx1, rgba) = match item {
                                                Ok(v) => v,
                                                Err(_) => break,
                                            };
                                            acted = true;
                                            in_flight
                                                .set(in_flight.get().saturating_sub(1));
                                            let Some(rgba) = rgba else {
                                                continue;
                                            };
                                            let id_u = id.max(0) as u64;
                                            let img = image_from_rgba(
                                                &rgba,
                                                w,
                                                daw_media::waveform_image::IMAGE_HEIGHT,
                                            );
                                            if let Some(i) = clip_row(&clips_model, id) {
                                                if let Some(mut data) =
                                                    clips_model.row_data(i)
                                                {
                                                    data.waveform_image = img;
                                                    // Janela rasterizada: x0 no
                                                    // corpo, largura renderizada.
                                                    data.waveform_src_x = wx0;
                                                    data.waveform_src_w = w as f32;
                                                    clips_model.set_row_data(i, data);
                                                    done_window.borrow_mut().insert(
                                                        id_u,
                                                        (
                                                            w_full,
                                                            wx0.round() as u32,
                                                            wx1.round() as u32,
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                        // Só para quando não há mais trabalho —
                                        // não queima CPU a 60Hz à toa.
                                        if !acted && in_flight.get() == 0 {
                                            if let Some(t) =
                                                collector_timer.borrow().as_ref()
                                            {
                                                t.stop();
                                            }
                                        }
                                    },
                                );
                            }
                        },
                    );
                }
            });
    }
}

/// Reprojeta a linha de UM clip após o cache ganhar/alterar sua
/// entrada (duplicar, colar, re-render por zoom).
///
/// Arquitetura vetor-primeiro: `waveform_points` (dados) são a
/// representação primária; `waveform_image` é só o raster CACHE
/// na resolução do último pedido — pode ser regenerado a qualquer
/// momento a partir dos samples.
pub fn reproject_clip(
    clips_model: &Rc<slint::VecModel<ClipData>>,
    waveforms: &WaveformCache,
    clip_id: u64,
) {
    let i = match crate::models::clip_row(clips_model, clip_id as i32) {
        Some(i) => i,
        None => return,
    };
    let Some(mut data) = clips_model.row_data(i) else {
        return;
    };
    let updated = to_clip_data_from_id(clip_id, &data, waveforms);
    data.waveform_points = updated.waveform_points;
    data.waveform_image = updated.waveform_image;
    clips_model.set_row_data(i, data);
}

/// Mapeia um clip do core para o DTO da camada Slint.
pub fn to_clip_data(clip: &Clip, waveforms: &WaveformCache) -> ClipData {
    let (peaks, img) = match waveforms.get(&clip.id) {
        Some(c) => (c.peaks.clone(), c.image.clone()),
        None => (Vec::new(), None),
    };
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat,
        len: clip.length_beats,
        is_midi: clip.is_midi(),
        color: clip.color as i32,
        waveform_points: Rc::new(slint::VecModel::from(peaks)).into(),
        waveform_image: waveform_image_to_slint(img),
        waveform_src_x: 0.0,
        waveform_src_w: 0.0,
        multi_selected: false,
    }
}
