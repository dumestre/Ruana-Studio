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
    /// Downmix mono de EXIBIÇÃO (downsample com teto).
    pub mono_ds: Vec<f32>,
    /// Pirâmide MIN/MAX/RMS sobre `mono_ds` (render de zoom por mipmap).
    pub mip: daw_media::waveform_image::Mipmap,
    /// LODs pré-renderizados (pipeline antigo; mantido para migração e
    /// fallback — o render atual usa o mipmap).
    #[allow(dead_code)]
    pub lods: daw_media::waveform_image::WaveformLods,
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

/// Aplica ganho linear e fade in/out no RGBA da waveform.
fn apply_gain_and_fade_to_rgba(rgba: &mut [u8], w: u32, h: u32, gain_linear: f32, fade_in_px: f32, fade_out_px: f32) {
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
        fade_in: base.fade_in,
        fade_out: base.fade_out,
        gain_db: base.gain_db,
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

use std::cell::RefCell;

use crate::TimelineMetrics;
use crate::models::clip_row;
use slint::{ComponentHandle, Model};

pub fn wire_resolution_requests(
    ui: &MainWindow,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
) {
    // WAVEFORM MIPMAP: render via pirâmide MIN/MAX/RMS.
    // A UI pede a resolução (width_px = largura do corpo do clip em px).
    // Rasterizamos o clip INTEIRO (`mono_ds`) na resolução exata pedida,
    // escolhendo o bucket de mip adequado — nítido e sem varrer o arquivo.
    ui.global::<TimelineMetrics>()
        .on_waveform_resolution_requested(move |id, width_px, _wx0, _wx1, hh, gain_linear, fi, fo| {
            let id_u = id.max(0) as u64;
            let w = (width_px.round() as u32).clamp(16, 16384);
            let h = (hh.round() as u32).clamp(4, 2048);

            let img = waveforms.borrow().get(&id_u).and_then(|c| {
                let mono = &c.mono_ds;
                if mono.is_empty() {
                    return None;
                }
                let mut rgba =
                    daw_media::waveform_image::waveform_image_rgba_mip(
                        mono,
                        &c.mip,
                        w,
                        h,
                        (0, mono.len()),
                    )?;
                apply_gain_and_fade_to_rgba(&mut rgba, w, h, gain_linear, fi, fo);
                Some(image_from_rgba(&rgba, w, h))
            });

            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut data) = clips_model.row_data(i) {
                    data.waveform_image = img.unwrap_or_default();
                    data.waveform_src_x = 0.0;
                    data.waveform_src_w = w as f32;
                    clips_model.set_row_data(i, data);
                }
            }
        });
}


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

/// Divide a waveform cacheada de um clip em duas metades proporcionais
/// a `f` (fração de tempo que fica no clip da ESQUERDA após o corte).
///
/// A nossa invariante é "cache ⇔ janela de origem do clip": o `mono_ds`
/// cobre exatamente o trecho do arquivo que o clip enxerga. Cortar sem
/// dividir faz o lado esquerdo continuar mostrando o arquivo INTEIRO
/// espremido e o direito não mostrar nada. Aqui cada metade ganha sua
/// própria parte (picos + mono + mipmap reconstruído). `image` volta a
/// `None` — o pedido de resolução da UI re-rasteriza as metades.
pub fn split_cached_wave(cw: &CachedWave, f: f64) -> (CachedWave, CachedWave) {
    let f = f.clamp(0.0, 1.0);
    let split = |v: &[f32]| -> (Vec<f32>, Vec<f32>) {
        if v.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let k = (v.len() as f64 * f + 0.5) as usize;
        let k = k.clamp(1, v.len().saturating_sub(1));
        (v[..k].to_vec(), v[k..].to_vec())
    };
    let (l_peaks, r_peaks) = split(&cw.peaks);
    let (l_mono, r_mono) = split(&cw.mono_ds);
    let (l_mip, r_mip) = {
        let build = |mono: &[f32]| daw_media::waveform_image::build_mipmap(mono);
        (build(&l_mono), build(&r_mono))
    };
    let l_lods = daw_media::waveform_image::build_waveform_lods(&l_mono);
    let r_lods = daw_media::waveform_image::build_waveform_lods(&r_mono);
    (
        CachedWave {
            peaks: l_peaks,
            mono_ds: l_mono,
            mip: l_mip,
            lods: l_lods,
            image: None,
        },
        CachedWave {
            peaks: r_peaks,
            mono_ds: r_mono,
            mip: r_mip,
            lods: r_lods,
            image: None,
        },
    )
}

/// Reconstitue a waveform da esquerda unindo as duas metades de um
/// corte (glue): picos + mono concatenados e mipmap reconstruído.
/// `image` volta a `None` — a UI re-rasteriza no próximo pedido.
pub fn merge_cached_waves(a: &CachedWave, b: &CachedWave) -> CachedWave {
    let mut peaks = a.peaks.clone();
    peaks.extend(b.peaks.iter().copied());
    let mut mono = a.mono_ds.clone();
    mono.extend(b.mono_ds.iter().copied());
    let mip = daw_media::waveform_image::build_mipmap(&mono);
    let lods = daw_media::waveform_image::build_waveform_lods(&mono);
    CachedWave {
        peaks,
        mono_ds: mono,
        mip,
        lods,
        image: None,
    }
}

/// Corta a waveform cacheada ao trecho visível após um RESIZE de
/// borda — sem re-escalar o conteúdo.
///
/// `f0` é a fração (do OLD len) que o novo clip recebe no INÍCIO
/// (borda esquerda avança o source); `f1` é a fração do len novo.
/// O conteúdo permanece ANCORADO no grid: um trim de borda direita
/// só esconde/revela o trecho, nunca encolhe o wave. `image` volta
/// a `None` — a UI re-rasteriza com o trecho já aparado.
pub fn trim_cached_wave(cw: &CachedWave, f0: f64, f1: f64) -> CachedWave {
    let f0 = f0.clamp(0.0, 1.0);
    let f1 = f1.clamp(0.0, 1.0 - f0);

    let trim = |v: &[f32]| -> Vec<f32> {
        if v.is_empty() {
            return Vec::new();
        }
        let n = v.len();
        let lo = ((n as f64 * f0).floor() as usize).min(n - 1);
        let hi = ((n as f64 * (f0 + f1)).ceil() as usize)
            .max(lo + 1)
            .min(n);
        v[lo..hi].to_vec()
    };

    let peaks = trim(&cw.peaks);
    let mono_ds = trim(&cw.mono_ds);
    let mip = daw_media::waveform_image::build_mipmap(&mono_ds);
    let lods = daw_media::waveform_image::build_waveform_lods(&mono_ds);
    CachedWave {
        peaks,
        mono_ds,
        mip,
        lods,
        image: None,
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
        gain_db: clip.gain_db,
        waveform_points: Rc::new(slint::VecModel::from(peaks)).into(),
        waveform_image: waveform_image_to_slint(img),
        waveform_src_x: 0.0,
        waveform_src_w: 0.0,
        multi_selected: false,
        fade_in: clip.fade_in_beats,
        fade_out: clip.fade_out_beats,
    }
}
