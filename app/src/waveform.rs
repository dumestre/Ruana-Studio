// Cache e rasterização de waveform para a camada Slint.
//
// O cache guarda `(picos, imagem RGBA)` por clip; a imagem é
// gerada UMA vez por import e re-renderizada na resolução exata
// quando o zoom muda (`render_waveform_at`).
use crate::MainWindow;

use std::rc::Rc;

use daw_core::clip::Clip;
use crate::ClipData;

/// Dados de waveform em cache por clip.
pub struct CachedWave {
    /// Picos agregados (fallback / compatibilidade).
    pub peaks: Vec<f32>,
    /// Downmix mono dos samples (fonte do re-render por zoom,
    /// incluindo o modo osciloscópio).
    pub mono: Vec<f32>,
    /// Imagem pré-renderizada na resolução do último pedido.
    pub image: Option<Vec<u8>>,
}

/// Cache de waveform: clip_id → dados completos.
pub type WaveformCache = std::collections::HashMap<u64, CachedWave>;

/// Renderiza uma waveform no tamanho EXATO pedido (px de tela).
///
/// Rota nova: rasteriza direto dos samples mono com a fatia
/// `view` visível — o modo osciloscópio entra sozinho quando o
/// zoom está muito fundo (< 2 samples/px). Igual ao lab.
pub fn render_waveform_at_samples(
    mono: &[f32],
    width_px: u32,
    view: (usize, usize),
) -> Option<slint::Image> {
    let rgba =
        daw_media::waveform_image::waveform_image_rgba_at_samples(mono, mono, width_px, view)?;
    Some(image_from_rgba(&rgba, width_px, daw_media::waveform_image::IMAGE_HEIGHT))
}

/// Fallback pela rota antiga (picos agregados).
fn render_waveform_at(peaks: &[f32], width_px: u32) -> Option<slint::Image> {
    use daw_media::waveform_image::waveform_image_rgba_at;
    let rgba = waveform_image_rgba_at(peaks, width_px)?;
    Some(waveform_image_to_slint(Some(rgba)))
}

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
    }
}

// Re-render da waveform na resolução exata de exibição.
//
// DEBOUNCE: escrever no modelo dentro do próprio `changed
// width` reconstrói a linha e redispara o handler (loop).
// Guardamos o pedido e aplicamos num timer de 120 ms; se um
// novo pedido chegar, reiniciamos o timer. Também ignoramos
// larguras iguais à última renderizada.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::models::clip_row;
use crate::TimelineMetrics;
use slint::{ComponentHandle, Model};

pub fn wire_resolution_requests(
    ui: &MainWindow,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
) {
        // Re-renderiza a waveform de um clip na resolução exata de
        // exibição (zoom/resize mudaram a largura em px).
        //
        // DEBOUNCE: escrever no modelo dentro do próprio `changed
        // width` reconstrói a linha e redispara o handler (loop).
        // Guardamos o pedido e aplicamos num timer de 120 ms; se um
        // novo pedido chegar, reiniciamos o timer. Também ignoramos
        // larguras iguais à última renderizada.
        {
                        let waveforms = waveforms.clone();
            let clips_model = clips_model.clone();
            let pending: Rc<RefCell<HashMap<i32, u32>>> = Rc::new(RefCell::new(HashMap::new()));
            let done_width: Rc<RefCell<HashMap<u64, u32>>> = Rc::new(RefCell::new(HashMap::new()));
            let timer: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));

            ui.global::<TimelineMetrics>().on_waveform_resolution_requested(move |id, width_px| {
                let id_u = id.max(0) as u64;
                let w = (width_px.round() as u32).clamp(16, 8192);

                // Já está nessa resolução? Nada a fazer.
                if done_width.borrow().get(&id_u) == Some(&w) {
                    return;
                }
                pending.borrow_mut().insert(id, w);

                // Reinicia o timer (debounce do gesto de zoom).
                let t = slint::Timer::default();
                *timer.borrow_mut() = Some(t);
                if let Some(t) = timer.borrow().as_ref() {
                    let pending = pending.clone();
                    let done_width = done_width.clone();
                    let waveforms = waveforms.clone();
                    let clips_model = clips_model.clone();
                    t.start(
                        slint::TimerMode::SingleShot,
                        std::time::Duration::from_millis(120),
                        move || {
                            let jobs: Vec<(i32, u32)> = pending.borrow_mut().drain().collect();
                            for (id, w) in jobs {
                                let id_u = id.max(0) as u64;
                                let img_new = {
                                    let wf = waveforms.borrow();
                                    match wf.get(&id_u) {
                                        // Rota nova: samples mono direto —
                                        // osciloscópio automático no zoom fundo.
                                        Some(c) if !c.mono.is_empty() => {
                                            let view = (0usize, c.mono.len());
                                            render_waveform_at_samples(&c.mono, w, view)
                                        }
                                        Some(c) if !c.peaks.is_empty() => {
                                            render_waveform_at(&c.peaks, w)
                                        }
                                        _ => None,
                                    }
                                };
                                if let Some(img) = img_new {
                                    if let Some(i) = clip_row(&clips_model, id) {
                                        if let Some(mut data) = clips_model.row_data(i) {
                                            data.waveform_image = img;
                                            clips_model.set_row_data(i, data);
                                            done_width.borrow_mut().insert(id_u, w);
                                        }
                                    }
                                }
                            }
                        },
                    );
                }
            });
        }
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
    }
}
