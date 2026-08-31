//! Renderizador do laboratório
//!
//! Duas rotas, ambas rasterizando com tiny-skia:
//!
//! - `render_mono`: downmix L+R/2 → envelope MIN/MAX centralizado
//!   + corpo RMS. Mostra o problema clássico do downmix.
//! - `render_stereo`: L e R em lanes separados (metade da altura
//!   cada), como uma DAW de verdade.
//!
//! Mapeamento por POSIÇÃO RELATIVA (x/width * sample_count) —
//! distribuição uniforme dos samples por coluna de pixel,
//! sem o último trecho torto dos chunks fixos.

use tiny_skia::*;

const H_LANE: u32 = 256; // altura interna por lane
const BG: [u8; 4] = [0x1c, 0x1f, 0x24, 255];
const FG: [u8; 4] = [0x0A, 0x0A, 0x0A, 255];
const RMS_WEIGHT: f32 = 0.35;
const MIN_HALF: f32 = 0.75;

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
    let rms = if slice.is_empty() { 0.0 } else { (sum_sq / slice.len() as f32).sqrt() };
    (min, max, rms)
}

/// Desenha um lane (envelope MIN/MAX + corpo RMS) dentro do pixmap,
/// entre y_center ± h_half.
fn draw_lane(pixmap: &mut Pixmap, samples: &[f32], y_center: f32, half_h: f32, view: (usize, usize)) {
    let w = pixmap.width();
    let n = view.1.saturating_sub(view.0);

    let mut paint = Paint::default();
    paint.set_color_rgba8(FG[0], FG[1], FG[2], FG[3]);
    paint.anti_alias = true;

    let mut top = Vec::with_capacity(w as usize);
    let mut bottom = Vec::with_capacity(w as usize);

    for x in 0..w as usize {
        // Coluna → faixa de samples DENTRO do viewport.
        let s0 = view.0 + (x as f64 / w as f64 * n as f64).floor() as usize;
        let s1 = view.0 + (((x + 1) as f64 / w as f64 * n as f64).ceil() as usize)
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

    // =====================================================
    // ENVELOPE POLIGONAL (padrão de DAW)
    //
    // Linhas retas entre os valores MIN/MAX reais. Sem interpolação:
    // qualquer spline aqui inventa overshoot/ondulação que não existe
    // no áudio. A suavidade visual vem do zoom (modo osciloscópio),
    // não de curvas artificiais.
    // =====================================================

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

// Linha central (eixo / ponto zero).
    let mut axis = Paint::default();
    axis.set_color_rgba8(0x4a, 0x50, 0x5c, 255);
    if let Some(r) = Rect::from_xywh(0.0, y_center - 0.5, w as f32, 1.0) {
        pixmap.fill_rect(r, &axis, Transform::identity(), None);
    }
}

fn new_canvas(w: u32, h: u32) -> Pixmap {
    let pm = Pixmap::new(w, h).unwrap();
    // fundo
    let mut pm = pm;
    let mut bgp = Paint::default();
    bgp.set_color_rgba8(BG[0], BG[1], BG[2], BG[3]);
    pm.fill_rect(
        Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
        &bgp,
        Transform::identity(),
        None,
    );
    pm
}

/// MONO: downmix (L+R)/2, envelope único centralizado no canvas inteiro.
///
/// É a rota "clássica" — e é ela que some com L=+1,R=-1.
pub fn render_mono(left: &[f32], right: &[f32], w: u32, view: (usize, usize)) -> Option<Vec<u8>> {
    let mono: Vec<f32> = left
        .iter()
        .zip(right.iter())
        .map(|(&l, &r)| (l + r) * 0.5)
        .collect();
    if mono.is_empty() || w == 0 {
        return None;
    }
    let mut pm = new_canvas(w, H_LANE * 2);
    draw_lane(&mut pm, &mono, H_LANE as f32, H_LANE as f32 - 4.0, view);
    Some(pm.data().to_vec())
}

/// ESTÉREO: L em cima, R embaixo, cada um com metade do canvas.
pub fn render_stereo(left: &[f32], right: &[f32], w: u32, view: (usize, usize)) -> Option<Vec<u8>> {
    if left.is_empty() || w == 0 {
        return None;
    }
    let mut pm = new_canvas(w, H_LANE * 2);
    let quarter = H_LANE as f32 / 2.0;
    draw_lane(&mut pm, left, quarter, quarter - 3.0, view);          // L
    draw_lane(&mut pm, right, H_LANE as f32 + quarter, quarter - 3.0, view); // R

    // Divisória entre canais.
    let mut dp = Paint::default();
    dp.set_color_rgba8(0x33, 0x38, 0x42, 255);
    pm.fill_rect(
        Rect::from_xywh(0.0, H_LANE as f32 - 1.0, w as f32, 2.0).unwrap(),
        &dp,
        Transform::identity(),
        None,
    );
    Some(pm.data().to_vec())
}

/// MODO OSCILOSCÓPIO: linha fina branca sample a sample.
///
/// Usado quando o zoom está tão fundo que cada pixel cobre
/// menos de ~2 samples — desenhar envelope aqui esconde os
/// ciclos reais da onda. É o visual da referência: linha
/// contínua mostrando os ciclos individuais, sem preenchimento.
pub fn render_oscilloscope(
    samples: &[f32],
    w: u32,
    view: (usize, usize),
    color: [u8; 4],
) -> Option<Vec<u8>> {
    if samples.is_empty() || w == 0 {
        return None;
    }
    let h = H_LANE * 2;
    let mut pm = new_canvas(w, h);
    let center = h as f32 / 2.0;
    let half_h = h as f32 / 2.0 - 6.0;
    let n = view.1.saturating_sub(view.0);

    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
    
    let mut pb = PathBuilder::new();
    let mut started = false;

    // Passo por pixel: quantos samples avança cada coluna.
    let step = n as f64 / w as f64;

    for x in 0..w as usize {
        // Sample central desta coluna.
        let idx = view.0 + ((x as f64 * step) as usize).min(n.saturating_sub(1));
        let s = samples.get(idx).copied().unwrap_or(0.0);
        let y = center - s.clamp(-1.0, 1.0) * half_h;
        if !started {
            pb.move_to(x as f32, y);
            started = true;
        } else {
            pb.line_to(x as f32, y);
        }
    }

    // Traço (stroke), não preenchimento!
    if started {
        if let Some(path) = pb.finish() {
            pixmap_stroke(&mut pm, &path, &paint);
        }
    }

    // Eixo central.
    let mut axis = Paint::default();
    axis.set_color_rgba8(0x4a, 0x50, 0x5c, 255);
    if let Some(r) = Rect::from_xywh(0.0, center - 0.5, w as f32, 1.0) {
        pm.fill_rect(r, &axis, Transform::identity(), None);
    }

    Some(pm.data().to_vec())
}

fn pixmap_stroke(pm: &mut Pixmap, path: &Path, paint: &Paint) {
    let mut stroke = Stroke::default();
    stroke.width = 1.25;
    pm.stroke_path(path, paint, &stroke, Transform::identity(), None);
}
