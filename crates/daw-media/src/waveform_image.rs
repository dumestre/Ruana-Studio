//! Renderização da waveform em imagem (RGBA em memória).
//!
//! Design validado no WaveformLab e aprovado visualmente:
//!
//! - Envelope poligonal MIN/MAX real do áudio + corpo RMS
//!   (linhas retas — sem splines: interpolação inventa overshoot)
//! - Mapeamento por posição relativa (`x/w * n`), distribuição
//!   uniforme dos samples por coluna de pixel
//! - MODO OSCILOSCÓPIO automático: com menos de ~2 samples por
//!   pixel, desenha a linha real sample a sample em vez do
//!   envelope — os ciclos individuais ficam visíveis no zoom fundo
//! - Waveform sempre escura (#0A0A0A), padrão DAW, nos dois modos

use tiny_skia::*;

/// Largura do cache padrão (import). Re-renders de zoom usam a
/// largura exata pedida pela UI.
pub const WAVEFORM_IMG_WIDTH: usize = 2048;

/// Altura interna da imagem. A curva é simétrica, então a UI
/// pode esticar verticalmente sem distorcer a forma.
const H: u32 = 512;

/// Altura interna das imagens geradas.
pub const IMAGE_HEIGHT: u32 = H;

const FG: [u8; 4] = [0x0A, 0x0A, 0x0A, 255];
/// Corpo RMS dentro do envelope (0 = só pico, 1 = RMS domina).
const RMS_WEIGHT: f32 = 0.35;
/// Espessura mínima do traço em px — trechos baixos não somem.
const MIN_HALF: f32 = 0.75;
/// Limite para trocar envelope → osciloscópio (samples por pixel).
const SCOPE_SAMPLES_PER_PX: f64 = 2.0;

/// Estatísticas MIN/MAX/RMS de uma fatia de samples.
#[inline]
fn stats(slice: &[f32]) -> (f32, f32, f32) {
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut sum_sq = 0.0f32;
    for &s in slice {
        let s = s.clamp(-1.0, 1.0);
        min = min.min(s);
        max = max.max(s);
        sum_sq += s * s;
    }
    let rms = if slice.is_empty() {
        0.0
    } else {
        (sum_sq / slice.len() as f32).sqrt()
    };
    (min, max, rms)
}

fn new_canvas(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    // Fundo transparente: o clip já tem o próprio corpo colorido.
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 0);
    pm.fill_rect(
        Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    pm
}

/// Desenha um lane de envelope (MIN/MAX + corpo RMS) centrado
/// em `y_center`, usando a fatia `view` dos samples.
fn draw_envelope_lane(
    pixmap: &mut Pixmap,
    samples: &[f32],
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) {
    let w = pixmap.width();
    let n = view.1.saturating_sub(view.0);

    let mut paint = Paint::default();
    paint.set_color_rgba8(FG[0], FG[1], FG[2], FG[3]);
    paint.anti_alias = true;

    let mut top = Vec::with_capacity(w as usize);
    let mut bottom = Vec::with_capacity(w as usize);

    for x in 0..w as usize {
        let s0 = view.0 + (x as f64 / w as f64 * n as f64).floor() as usize;
        let s1 = view.0
            + (((x + 1) as f64 / w as f64 * n as f64).ceil() as usize)
                .max(s0 + 1)
                .min(view.1);
        let s0 = s0.min(view.1);

        let (min, max, rms) = if s1 > s0 {
            stats(&samples[s0..s1])
        } else {
            (0.0, 0.0, 0.0)
        };

        let pos_peak = max.max(0.0);
        let neg_peak = min.min(0.0);
        let rms_body = rms * RMS_WEIGHT;

        let upper = pos_peak.max(rms_body).clamp(0.0, 1.0);
        let lower = neg_peak.min(-rms_body).clamp(-1.0, 0.0);

        top.push(y_center - (upper * half_h).max(MIN_HALF));
        bottom.push(y_center + (-lower * half_h).max(MIN_HALF));
    }

    let mut pb = PathBuilder::new();
    pb.move_to(0.0, top[0]);
    for x in 1..w as usize {
        pb.line_to(x as f32, top[x]);
    }
    for x in (0..w as usize).rev() {
        pb.line_to(x as f32, bottom[x]);
    }
    pb.close();

    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// MODO OSCILOSCÓPIO: linha fina sample a sample (sem preenchimento).
fn draw_scope_lane(pixmap: &mut Pixmap, samples: &[f32], y_center: f32, half_h: f32, view: (usize, usize)) {
    use tiny_skia::Stroke;

    let w = pixmap.width();
    let n = view.1.saturating_sub(view.0);

    let mut paint = Paint::default();
    paint.set_color_rgba8(FG[0], FG[1], FG[2], FG[3]);
    paint.anti_alias = true;

    let mut pb = PathBuilder::new();
    let step = n as f64 / w as f64;
    for x in 0..w as usize {
        let idx = view.0 + ((x as f64 * step) as usize).min(n.saturating_sub(1));
        let s = samples.get(idx).copied().unwrap_or(0.0);
        let y = y_center - s.clamp(-1.0, 1.0) * half_h;
        if x == 0 {
            pb.move_to(x as f32, y);
        } else {
            pb.line_to(x as f32, y);
        }
    }

    if let Some(path) = pb.finish() {
        let mut stroke = Stroke::default();
        stroke.width = 1.25;
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    // Eixo central sutil.
    let mut axis = Paint::default();
    axis.set_color_rgba8(0x33, 0x38, 0x42, 255);
    if let Some(r) = Rect::from_xywh(0.0, y_center - 0.5, w as f32, 1.0) {
        pixmap.fill_rect(r, &axis, Transform::identity(), None);
    }
}

/// Rasteriza a waveform MONO (downmix L+R/2) na largura pedida.
///
/// `view.0..view.1` é a fatia visível em samples. O modo
/// osciloscópio entra sozinho quando o zoom está muito fundo.
pub fn waveform_image_rgba_at_samples(
    left: &[f32],
    right: &[f32],
    w: u32,
    view: (usize, usize),
) -> Option<Vec<u8>> {
    let mono: Vec<f32> = left.iter().zip(right.iter()).map(|(&l, &r)| (l + r) * 0.5).collect();
    if mono.is_empty() || w == 0 {
        return None;
    }

    let h = H;
    let mut pm = new_canvas(w, h);
    let center = h as f32 / 2.0;
    let half_h = h as f32 / 2.0 - 4.0;

    let n = view.1.saturating_sub(view.0);
    let scope = (n as f64 / w as f64) < SCOPE_SAMPLES_PER_PX;

    if scope {
        draw_scope_lane(&mut pm, &mono, center, half_h, view);
    } else {
        draw_envelope_lane(&mut pm, &mono, center, half_h, view);
    }

    Some(pm.data().to_vec())
}

// ============================================================
// COMPATIBILIDADE com a API atual (picos agregados).
//
// A rota ideal é `waveform_image_rgba_at_samples` direto dos
// samples; esta mantém o pipeline existente funcionando enquanto
// a integração avança.
// ============================================================

/// Rasteriza os picos num envelope contínuo, largura `w` px.
///
/// Os valores já chegam como amplitude real 0..1 (RMS + crista,
/// sem normalização por arquivo): áudio baixo desenha baixo.
pub fn waveform_image_rgba_at(peaks: &[f32], w: u32) -> Option<Vec<u8>> {
    if peaks.is_empty() || w == 0 {
        return None;
    }
    let h = H;

    let mut pixmap = Pixmap::new(w, h)?;
    let center = h as f32 / 2.0;
    let n = peaks.len() as f32;

    let mut paint = Paint::default();
    paint.set_color_rgba8(FG[0], FG[1], FG[2], FG[3]);
    paint.anti_alias = true;

    // Amplitude máxima das colunas [i0..i1) de picos que caem no pixel x.
    let amp_at = |x: u32| -> f32 {
        let i0 = ((x as f64 / w as f64) * n as f64).floor() as usize;
        let i1 = ((((x + 1) as f64 / w as f64) * n as f64).ceil() as usize).max(i0 + 1);
        peaks[i0.min(peaks.len() - 1)..i1.min(peaks.len())]
            .iter()
            .fold(0.0f32, |a, &p| a.max(p))
    };

    let mut pb = PathBuilder::new();

    // Contorno superior, esquerda → direita.
    for x in 0..w {
        let half = (amp_at(x).clamp(0.0, 1.0) * center).max(1.0);
        if x == 0 {
            pb.move_to(0.0, center - half);
        } else {
            pb.line_to(x as f32, center - half);
        }
    }
    // Contorno inferior espelhado, direita → esquerda.
    for x in (0..w).rev() {
        let half = (amp_at(x).clamp(0.0, 1.0) * center).max(1.0);
        pb.line_to(x as f32, center + half);
    }
    pb.close();

    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    Some(pixmap.data().to_vec())
}

/// Compatibilidade: rasteriza na largura padrão.
pub fn waveform_image_rgba(peaks: &[f32], _color_index: i32, _is_midi: bool) -> Option<Vec<u8>> {
    waveform_image_rgba_at(peaks, WAVEFORM_IMG_WIDTH as u32)
}
