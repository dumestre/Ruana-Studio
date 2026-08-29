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
// MIPMAP DE PICOS (MIN/MAX/RMS hierárquicos)
//
// Zoom sem re-varredura: o envelope em qualquer largura lê níveis
// pré-computados em vez de escanear os samples de novo. Construído
// UMA vez por sample (thread de fundo) e guardado no cache; o custo
// por re-render cai de O(samples) para O(pixels × poucos buckets).
// ============================================================

/// Estatísticas agregadas de um bucket de samples (sinal clampado).
#[derive(Clone, Copy, Debug)]
pub struct MipBucket {
    pub min: f32,
    pub max: f32,
    pub sum_sq: f32,
    pub count: u32,
}

impl MipBucket {
    fn new(slice: &[f32]) -> Self {
        let mut min = 0.0f32;
        let mut max = 0.0f32;
        let mut sum_sq = 0.0f32;
        for &s in slice {
            let s = s.clamp(-1.0, 1.0);
            min = min.min(s);
            max = max.max(s);
            sum_sq += s * s;
        }
        MipBucket {
            min,
            max,
            sum_sq,
            count: slice.len() as u32,
        }
    }

    fn merge(a: MipBucket, b: MipBucket) -> MipBucket {
        MipBucket {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
            sum_sq: a.sum_sq + b.sum_sq,
            count: a.count + b.count,
        }
    }
}

/// Pirâmide: `levels[i]` tem buckets de `1 << (i+1)` samples.
/// O nível 0 (bucket 2) em diante — escope fino lê o mono direto.
#[derive(Clone, Debug)]
pub struct Mipmap {
    pub levels: Vec<Vec<MipBucket>>,
}

impl Mipmap {
    /// Nº de samples de origem cobertos por bucket do nível `i`.
    pub fn bucket(&self, i: usize) -> usize {
        1usize << (i + 1)
    }

    /// Nível com o MAIOR bucket ≤ `spp` samples por pixel.
    pub fn level_for(&self, spp: f64) -> usize {
        if spp < 2.0 || self.levels.is_empty() {
            return 0;
        }
        let mut i = (spp.log2().floor() as usize).saturating_sub(1);
        i = i.min(self.levels.len() - 1);
        while i > 0 && self.bucket(i) > spp as usize {
            i -= 1;
        }
        i
    }
}

/// Constrói a pirâmide a partir do mono de exibição.
pub fn build_mipmap(mono: &[f32]) -> Mipmap {
    let mut levels = Vec::new();
    let mut cur: Vec<MipBucket> = mono.chunks(2).map(MipBucket::new).collect();
    while !cur.is_empty() {
        if cur.len() == 1 {
            levels.push(cur);
            break;
        }
        let mut next = Vec::with_capacity(cur.len().div_ceil(2));
        let mut it = cur.chunks(2);
        while let Some(pair) = it.next() {
            if pair.len() == 2 {
                next.push(MipBucket::merge(pair[0], pair[1]));
            } else {
                next.push(pair[0]);
            }
        }
        levels.push(cur);
        cur = next;
    }
    Mipmap { levels }
}

/// Teto de samples do mono de exibição guardado no cache (~4 MB).
pub const MONO_DISPLAY_CAP: usize = 1 << 20;

/// Downsample por stride (fiel, sem suavizar transientes) com teto;
/// arquivos dentro do teto voltam-se intactos.
pub fn downsample_mono(mono: &[f32]) -> Vec<f32> {
    if mono.len() <= MONO_DISPLAY_CAP {
        return mono.to_vec();
    }
    let step = (mono.len() as f64 / MONO_DISPLAY_CAP as f64).ceil() as usize;
    mono.iter().step_by(step).copied().collect()
}

/// Mono de exibição + mipmap prontos para o cache de UI.
pub fn build_display_wave(mono: &[f32]) -> (Vec<f32>, Mipmap) {
    let ds = downsample_mono(mono);
    let mip = build_mipmap(&ds);
    (ds, mip)
}

/// Rasteriza o envelope MIN/MAX+RMS lendo os buckets pré-computados
/// do nível certo para a largura pedida — sem varrer os samples.
fn draw_envelope_mip(
    pixmap: &mut Pixmap,
    buckets: &[MipBucket],
    bucket_size: usize,
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) {
    let w = pixmap.width();
    let n = view.1.saturating_sub(view.0);
    let v0 = view.0;
    let bucket_size = bucket_size.max(1);

    let mut paint = Paint::default();
    paint.set_color_rgba8(FG[0], FG[1], FG[2], FG[3]);
    paint.anti_alias = true;

    let mut top = Vec::with_capacity(w as usize);
    let mut bottom = Vec::with_capacity(w as usize);

    for x in 0..w as usize {
        let s0 = v0 + (x as f64 / w as f64 * n as f64).floor() as usize;
        let s1 = v0
            + (((x + 1) as f64 / w as f64 * n as f64).ceil() as usize)
                .max(s0 + 1)
                .min(view.1);
        let s0 = s0.min(view.1);

        let i0 = s0 / bucket_size;
        let i1 = (s1 as f64 / bucket_size as f64).ceil() as usize;
        let i1 = i1.min(buckets.len());

        let mut min = 0.0f32;
        let mut max = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut count = 0u64;
        if i1 > i0 && i0 < buckets.len() {
            for b in &buckets[i0..i1] {
                min = min.min(b.min);
                max = max.max(b.max);
                sum_sq += b.sum_sq as f64;
                count += b.count as u64;
            }
        }
        let rms = if count > 0 {
            ((sum_sq / count as f64) as f32).sqrt()
        } else {
            0.0
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

/// Rasteriza o mono de exibição (downsample/mipmap) na largura pedida.
///
/// `view.0..view.1` é a fatia visível em samples (do mono de origem).
/// MODO OSCILOSCÓPIO quando o zoom está muito fundo (< 2 samples/px):
/// aí o nível 0 não basta — desenha o mono direto sample a sample.
pub fn waveform_image_rgba_mip(
    mono: &[f32],
    mip: &Mipmap,
    w: u32,
    view: (usize, usize),
) -> Option<Vec<u8>> {
    if mono.is_empty() || w == 0 {
        return None;
    }

    let h = H;
    let mut pm = new_canvas(w, h);
    let center = h as f32 / 2.0;
    let half_h = h as f32 / 2.0 - 4.0;

    let n = view.1.saturating_sub(view.0);
    let spp = n as f64 / w as f64;

    if spp < SCOPE_SAMPLES_PER_PX {
        draw_scope_lane(&mut pm, mono, center, half_h, view);
    } else {
        let lvl = mip.level_for(spp);
        match mip.levels.get(lvl) {
            Some(level) => {
                draw_envelope_mip(&mut pm, level, mip.bucket(lvl), center, half_h, view)
            }
            None => draw_scope_lane(&mut pm, mono, center, half_h, view),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mipmap_levels_halve_and_merge() {
        let mono: Vec<f32> = (0..16).map(|i| ((i as i32 % 5) - 2) as f32 / 4.0).collect();
        let mip = build_mipmap(&mono);
        // level0: bucket 2 (8 buckets), sobindo até 1 bucket lá em cima.
        assert!(!mip.levels.is_empty());
        assert_eq!(mip.levels[0].len(), 8);
        assert_eq!(mip.bucket(0), 2);
        assert_eq!(mip.bucket(2), 8);
        // O bucket do topo cobre a pirâmide inteira com min/max reais.
        let top = mip.levels.last().unwrap()[0];
        assert_eq!(
            top.min,
            mono.iter().copied().fold(f32::INFINITY, f32::min)
        );
        assert_eq!(
            top.max,
            mono.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        );
    }

    #[test]
    fn level_for_picks_fit_bucket() {
        let mono = vec![0.0f32; 1000];
        let mip = build_mipmap(&mono);
        // spp em [2,4) → bucket 2; spp 8 → bucket 8; nunca maior que spp.
        assert_eq!(mip.bucket(mip.level_for(3.0)), 2);
        assert_eq!(mip.bucket(mip.level_for(7.9)), 4);
        assert_eq!(mip.bucket(mip.level_for(8.0)), 8);
        assert!(mip.bucket(mip.level_for(999.0)) <= 1000);
    }

    #[test]
    fn mip_raster_matches_scan_at_column_alignment() {
        // Sinal senoidal; largura e tamanho poder-de-dois alinham as
        // colunas com os buckets do nível 0 → o raster mip tem que
        // ser IDÊNTICO ao scan direto dos samples.
        let n = 64usize;
        let mono: Vec<f32> = (0..n)
            .map(|i| ((i as f32 / 4.0).sin()) * 0.9)
            .collect();
        let mip = build_mipmap(&mono);
        let view = (0usize, n);
        let w = 16u32; // spp = 4 → nível bucket 4 alinhado (borda par da coluna)
        let scan = waveform_image_rgba_at_samples(&mono, &mono, w, view).unwrap();
        let mip_img = waveform_image_rgba_mip(&mono, &mip, w, view).unwrap();
        assert_eq!(scan.len(), mip_img.len());
        for (a, b) in scan.iter().zip(mip_img.iter()) {
            assert_eq!(a, b, "pixel divergente no envelope mip");
        }
    }
}
