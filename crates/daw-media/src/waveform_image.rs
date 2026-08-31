//! Renderização da waveform em imagem RGBA em memória.
//!
//! Pipeline:
//!
//! ```text
//! áudio mono
//!     ↓
//! downsample preservando transientes
//!     ↓
//! display wave
//!     ↓
//! mipmap hierárquico MIN/MAX/RMS
//!     ↓
//! rasterização na resolução EXATA da UI
//! ```
//!
//! Características:
//!
//! - Envelope MIN/MAX real do áudio.
//! - Corpo RMS.
//! - Linhas retas, sem spline.
//! - Mipmap hierárquico para zoom.
//! - Transientes preservados no downsample.
//! - Osciloscópio em zoom extremo.
//! - Altura exatamente igual à solicitada pela UI.
//! - Fundo transparente.
//! - Waveform escura (#0A0A0A).
//!
//! API moderna:
//!
//! ```text
//! build_display_wave()
//! waveform_image_rgba_mip()
//! ```
//!
//! API legada mantida:
//!
//! ```text
//! build_waveform_lods()
//! pick_lod()
//! waveform_image_rgba()
//! waveform_image_rgba_at()
//! ```

use tiny_skia::*;

const LEVELS: usize = 6;

/// Largura usada pelas APIs antigas.
pub const WAVEFORM_IMG_WIDTH: usize = 2048;

/// Altura usada pelas APIs antigas.
pub const IMAGE_HEIGHT: u32 = 512;

/// Altura legada.
const H: u32 = IMAGE_HEIGHT;

/// Cor da waveform.
const FG: [u8; 4] = [0x0A, 0x0A, 0x0A, 255];

/// Peso visual do RMS dentro do envelope.
const RMS_WEIGHT: f32 = 0.35;

/// Espessura mínima visual do envelope.
const MIN_HALF: f32 = 0.75;

/// Abaixo disso entra no modo osciloscópio.
const SCOPE_SAMPLES_PER_PX: f64 = 2.0;

/// Número máximo de samples mantidos diretamente.
///
/// Aproximadamente 4 MB em f32.
pub const MONO_DISPLAY_CAP: usize = 1 << 20;

// ============================================================
// LOD LEGADO
// ============================================================

#[derive(Clone)]
pub struct WaveformLod {
    pub width: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone)]
pub struct WaveformLods {
    pub levels: Vec<WaveformLod>,
}

impl WaveformLods {
    /// Retorna o maior LOD que não ultrapassa a largura desejada.
    ///
    /// Se todos forem maiores que `target_width`, retorna o menor.
    pub fn best_level(&self, target_width: u32) -> Option<&WaveformLod> {
        let mut best: Option<&WaveformLod> = None;

        for level in &self.levels {
            if level.width <= target_width {
                if best
                    .map(|current| level.width > current.width)
                    .unwrap_or(true)
                {
                    best = Some(level);
                }
            }
        }

        if best.is_none() {
            best = self.levels.last();
        }

        best
    }
}

// ============================================================
// CANVAS
// ============================================================

fn new_canvas(w: u32, h: u32) -> Pixmap {
    Pixmap::new(w, h).expect("falha ao criar Pixmap")
}

// ============================================================
// ESTATÍSTICAS
// ============================================================

/// Retorna:
///
/// ```text
/// MIN
/// MAX
/// RMS
/// ```
///
/// O MIN/MAX sempre incluem o eixo zero.
/// Isso é proposital: uma waveform positiva não deve "flutuar"
/// para fora do centro.
#[inline]
fn stats(slice: &[f32]) -> (f32, f32, f32) {
    if slice.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut sum_sq = 0.0f32;

    for &sample in slice {
        let sample = sample.clamp(-1.0, 1.0);

        min = min.min(sample);
        max = max.max(sample);

        sum_sq += sample * sample;
    }

    let rms = (sum_sq / slice.len() as f32).sqrt();

    (min, max, rms)
}

// ============================================================
// ENVELOPE DIRETO
// ============================================================

/// Desenha a waveform diretamente dos samples.
///
/// Usado:
///
/// - como fallback;
/// - pela API legada;
/// - quando não existe Mipmap.
fn draw_envelope_lane(
    pixmap: &mut Pixmap,
    samples: &[f32],
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) {
    let width = pixmap.width() as usize;

    if width == 0 || samples.is_empty() {
        return;
    }

    let start = view.0.min(samples.len());
    let end = view.1.min(samples.len());

    if end <= start {
        return;
    }

    let sample_count = end - start;

    let mut top = Vec::with_capacity(width);
    let mut bottom = Vec::with_capacity(width);

    for x in 0..width {
        let rel_start =
            ((x as f64 / width as f64) * sample_count as f64).floor() as usize;

        let rel_end =
            (((x + 1) as f64 / width as f64) * sample_count as f64).ceil() as usize;

        let rel_start = rel_start.min(sample_count.saturating_sub(1));

        let rel_end = rel_end
            .max(rel_start + 1)
            .min(sample_count);

        let s0 = start + rel_start;
        let s1 = start + rel_end;

        let (min, max, rms) = stats(&samples[s0..s1]);

        let positive_peak = max.max(0.0);
        let negative_peak = min.min(0.0);

        let rms_body = rms * RMS_WEIGHT;

        let upper = positive_peak
            .max(rms_body)
            .clamp(0.0, 1.0);

        let lower = negative_peak
            .min(-rms_body)
            .clamp(-1.0, 0.0);

        top.push(
            y_center
                - (upper * half_h).max(MIN_HALF),
        );

        bottom.push(
            y_center
                + (-lower * half_h).max(MIN_HALF),
        );
    }

    draw_filled_envelope(
        pixmap,
        &top,
        &bottom,
    );
}

// ============================================================
// ENVELOPE COMUM
// ============================================================

fn draw_filled_envelope(
    pixmap: &mut Pixmap,
    top: &[f32],
    bottom: &[f32],
) {
    if top.is_empty() || bottom.is_empty() {
        return;
    }

    let width = top.len().min(bottom.len());

    if width == 0 {
        return;
    }

    let mut paint = Paint::default();

    paint.set_color_rgba8(
        FG[0],
        FG[1],
        FG[2],
        FG[3],
    );

    paint.anti_alias = true;

    let mut path = PathBuilder::new();

    path.move_to(
        0.0,
        top[0],
    );

    for x in 1..width {
        path.line_to(
            x as f32,
            top[x],
        );
    }

    for x in (0..width).rev() {
        path.line_to(
            x as f32,
            bottom[x],
        );
    }

    path.close();

    if let Some(path) = path.finish() {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

// ============================================================
// OSCILOSCÓPIO
// ============================================================

/// Desenha o sinal real como uma linha.
///
/// Entra em ação quando existem menos de 2 samples por pixel.
fn draw_scope_lane(
    pixmap: &mut Pixmap,
    samples: &[f32],
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) {
    let width = pixmap.width() as usize;

    if width == 0 || samples.is_empty() {
        return;
    }

    let start = view.0.min(samples.len());
    let end = view.1.min(samples.len());

    if end <= start {
        return;
    }

    let sample_count = end - start;

    let mut paint = Paint::default();

    paint.set_color_rgba8(
        FG[0],
        FG[1],
        FG[2],
        FG[3],
    );

    paint.anti_alias = true;

    let mut path = PathBuilder::new();

    let step =
        sample_count as f64 / width as f64;

    for x in 0..width {
        let position = x as f64 * step;

        let index = position.floor() as usize;
        let fraction = position.fract() as f32;

        let i0 =
            start + index.min(sample_count.saturating_sub(1));

        let i1 =
            (i0 + 1).min(end.saturating_sub(1));

        let s0 =
            samples.get(i0).copied().unwrap_or(0.0);

        let s1 =
            samples.get(i1).copied().unwrap_or(s0);

        let sample =
            if i0 == i1 {
                s0
            } else {
                s0 + (s1 - s0) * fraction
            };

        let y =
            y_center
                - sample.clamp(-1.0, 1.0) * half_h;

        if x == 0 {
            path.move_to(
                x as f32,
                y,
            );
        } else {
            path.line_to(
                x as f32,
                y,
            );
        }
    }

    if let Some(path) = path.finish() {
        let mut stroke = Stroke::default();

        stroke.width = 1.25;

        pixmap.stroke_path(
            &path,
            &paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }

    // Eixo central.
    let mut axis = Paint::default();

    axis.set_color_rgba8(
        0x33,
        0x38,
        0x42,
        255,
    );

    if let Some(rect) = Rect::from_xywh(
        0.0,
        y_center - 0.5,
        width as f32,
        1.0,
    ) {
        pixmap.fill_rect(
            rect,
            &axis,
            Transform::identity(),
            None,
        );
    }
}

// ============================================================
// RENDER DIRETO
// ============================================================

/// Rasteriza L/R diretamente.
///
/// Esta API é mantida por compatibilidade.
///
/// A rota moderna deve usar:
///
/// ```text
/// build_display_wave()
/// waveform_image_rgba_mip()
/// ```
pub fn waveform_image_rgba_at_samples(
    left: &[f32],
    right: &[f32],
    w: u32,
    view: (usize, usize),
) -> Option<Vec<u8>> {
    if left.is_empty()
        || right.is_empty()
        || w == 0
    {
        return None;
    }

    let sample_count =
        left.len().min(right.len());

    if sample_count == 0 {
        return None;
    }

    let start =
        view.0.min(sample_count);

    let end =
        view.1.min(sample_count);

    if end <= start {
        return None;
    }

    // Compatibilidade:
    // cria mono apenas para visualização.
    let mono: Vec<f32> = left[..sample_count]
        .iter()
        .zip(right[..sample_count].iter())
        .map(|(&l, &r)| {
            (l + r) * 0.5
        })
        .collect();

    let mut pixmap =
        new_canvas(w, H);

    let center =
        H as f32 / 2.0;

    let half_h =
        (H as f32 / 2.0 - 4.0).max(1.0);

    let samples_per_pixel =
        (end - start) as f64 / w as f64;

    if samples_per_pixel
        < SCOPE_SAMPLES_PER_PX
    {
        draw_scope_lane(
            &mut pixmap,
            &mono,
            center,
            half_h,
            (start, end),
        );
    } else {
        draw_envelope_lane(
            &mut pixmap,
            &mono,
            center,
            half_h,
            (start, end),
        );
    }

    Some(
        pixmap.data().to_vec()
    )
}

// ============================================================
// MIPMAP
// ============================================================

/// Bucket estatístico.
///
/// Representa uma quantidade de samples:
///
/// ```text
/// min
/// max
/// soma dos quadrados
/// quantidade
/// ```
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

        for &sample in slice {
            let sample =
                sample.clamp(-1.0, 1.0);

            min = min.min(sample);
            max = max.max(sample);

            sum_sq += sample * sample;
        }

        Self {
            min,
            max,
            sum_sq,
            count: slice.len() as u32,
        }
    }

    #[inline]
    fn merge(
        a: Self,
        b: Self,
    ) -> Self {
        Self {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
            sum_sq: a.sum_sq + b.sum_sq,
            count: a.count + b.count,
        }
    }
}

/// Pirâmide hierárquica.
///
/// level 0:
///
/// ```text
/// 2 samples / bucket
/// ```
///
/// level 1:
///
/// ```text
/// 4 samples / bucket
/// ```
///
/// level 2:
///
/// ```text
/// 8 samples / bucket
/// ```
///
/// etc.
#[derive(Clone, Debug)]
pub struct Mipmap {
    pub levels: Vec<Vec<MipBucket>>,
}

impl Mipmap {
    /// Quantidade de samples representados por bucket.
    #[inline]
    pub fn bucket(
        &self,
        level: usize,
    ) -> usize {
        1usize << (level + 1)
    }

    /// Seleciona o maior bucket que cabe no
    /// número de samples por pixel.
    pub fn level_for(
        &self,
        samples_per_pixel: f64,
    ) -> usize {
        if self.levels.is_empty()
            || samples_per_pixel < 2.0
        {
            return 0;
        }

        let mut level =
            samples_per_pixel
                .log2()
                .floor()
                as usize;

        level =
            level.saturating_sub(1);

        level =
            level.min(
                self.levels.len()
                    .saturating_sub(1)
            );

        while level > 0
            && self.bucket(level) as f64
                > samples_per_pixel
        {
            level -= 1;
        }

        level
    }
}

/// Constrói a pirâmide Mipmap.
///
/// Custo total O(N).
///
/// Deve ser feito uma única vez e reutilizado
/// durante todos os zooms.
pub fn build_mipmap(
    mono: &[f32],
) -> Mipmap {
    if mono.is_empty() {
        return Mipmap {
            levels: Vec::new(),
        };
    }

    let mut levels =
        Vec::new();

    // --------------------------------------------------------
    // LEVEL 0
    // --------------------------------------------------------

    let mut current: Vec<MipBucket> =
        mono.chunks(2)
            .map(MipBucket::new)
            .collect();

    loop {
        levels.push(
            current.clone()
        );

        if current.len() <= 1 {
            break;
        }

        let mut next =
            Vec::with_capacity(
                current.len().div_ceil(2)
            );

        for pair in current.chunks(2) {
            match pair {
                [a, b] => {
                    next.push(
                        MipBucket::merge(
                            *a,
                            *b,
                        )
                    );
                }

                [a] => {
                    next.push(*a);
                }

                _ => {}
            }
        }

        current = next;
    }

    Mipmap {
        levels,
    }
}

// ============================================================
// DOWNSAMPLE
// ============================================================

/// Reduz uma waveform grande preservando transientes.
///
/// Cada bloco gera:
///
/// ```text
/// mínimo
/// máximo
/// ```
///
/// na ordem temporal aproximada.
///
/// Isso é muito melhor que simplesmente fazer:
///
/// ```text
/// step_by()
/// ```
///
/// porque `step_by()` pode simplesmente pular um transient.
pub fn downsample_mono(
    mono: &[f32],
) -> Vec<f32> {
    if mono.len()
        <= MONO_DISPLAY_CAP
    {
        return mono.to_vec();
    }

    let block_size =
        mono.len()
            .div_ceil(MONO_DISPLAY_CAP);

    let block_count =
        mono.len()
            .div_ceil(block_size);

    let mut output =
        Vec::with_capacity(
            block_count * 2
        );

    for chunk in
        mono.chunks(block_size)
    {
        if chunk.is_empty() {
            continue;
        }

        let mut min =
            f32::INFINITY;

        let mut max =
            f32::NEG_INFINITY;

        let mut min_pos =
            0usize;

        let mut max_pos =
            0usize;

        for (index, &value)
            in chunk.iter().enumerate()
        {
            let value =
                value.clamp(-1.0, 1.0);

            if value < min {
                min = value;
                min_pos = index;
            }

            if value > max {
                max = value;
                max_pos = index;
            }
        }

        // Mantém a ordem aproximada do sinal.
        if min_pos <= max_pos {
            output.push(min);
            output.push(max);
        } else {
            output.push(max);
            output.push(min);
        }
    }

    output
}

// ============================================================
// DISPLAY WAVE
// ============================================================

/// Constrói a representação usada exclusivamente
/// pela renderização.
///
/// Retorna:
///
/// ```text
/// display samples
/// mipmap
/// ```
///
/// Esse resultado deve ser armazenado em cache.
///
/// NÃO reconstrua isso a cada zoom.
pub fn build_display_wave(
    mono: &[f32],
) -> (Vec<f32>, Mipmap) {
    let display =
        downsample_mono(mono);

    let mip =
        build_mipmap(&display);

    (display, mip)
}

// ============================================================
// ENVELOPE VIA MIPMAP
// ============================================================

/// Calcula o envelope para cada coluna.
///
/// IMPORTANTE:
///
/// `view` está em coordenadas da `display wave`.
///
/// `bucket_size` também está em coordenadas
/// da `display wave`.
fn envelope_contours(
    pixmap: &mut Pixmap,
    buckets: &[MipBucket],
    bucket_size: usize,
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) -> (Vec<f32>, Vec<f32>) {
    let width =
        pixmap.width() as usize;

    if width == 0
        || buckets.is_empty()
    {
        return (
            Vec::new(),
            Vec::new(),
        );
    }

    let bucket_size =
        bucket_size.max(1);

    let start =
        view.0;

    let end =
        view.1;

    if end <= start {
        return (
            Vec::new(),
            Vec::new(),
        );
    }

    let sample_count =
        end - start;

    let mut top =
        Vec::with_capacity(width);

    let mut bottom =
        Vec::with_capacity(width);

    for x in 0..width {
        // Intervalo da coluna em samples.
        let rel_start =
            ((x as f64 / width as f64)
                * sample_count as f64)
                .floor()
                as usize;

        let rel_end =
            (((x + 1) as f64 / width as f64)
                * sample_count as f64)
                .ceil()
                as usize;

        let rel_start =
            rel_start.min(
                sample_count
                    .saturating_sub(1)
            );

        let rel_end =
            rel_end
                .max(rel_start + 1)
                .min(sample_count);

        let sample_start =
            start + rel_start;

        let sample_end =
            start + rel_end;

        // ----------------------------------------------------
        // Descobre exatamente quais buckets intersectam
        // a coluna.
        //
        // Bucket B cobre:
        //
        //     [B * bucket_size,
        //      (B + 1) * bucket_size)
        // ----------------------------------------------------

        let bucket_start =
            sample_start / bucket_size;

        let bucket_end =
            sample_end
                .div_ceil(bucket_size);

        let bucket_start =
            bucket_start.min(
                buckets.len()
            );

        let bucket_end =
            bucket_end.min(
                buckets.len()
            );

        let mut min =
            0.0f32;

        let mut max =
            0.0f32;

        let mut sum_sq =
            0.0f64;

        let mut count =
            0u64;

        if bucket_end
            > bucket_start
        {
            for bucket in
                &buckets[
                    bucket_start
                        ..bucket_end
                ]
            {
                min =
                    min.min(bucket.min);

                max =
                    max.max(bucket.max);

                sum_sq +=
                    bucket.sum_sq
                        as f64;

                count +=
                    bucket.count
                        as u64;
            }
        }

        let rms =
            if count > 0 {
                (
                    sum_sq
                        / count as f64
                )
                .sqrt()
                    as f32
            } else {
                0.0
            };

        let positive_peak =
            max.max(0.0);

        let negative_peak =
            min.min(0.0);

        let rms_body =
            rms * RMS_WEIGHT;

        let upper =
            positive_peak
                .max(rms_body)
                .clamp(0.0, 1.0);

        let lower =
            negative_peak
                .min(-rms_body)
                .clamp(-1.0, 0.0);

        top.push(
            y_center
                - (upper * half_h)
                    .max(MIN_HALF),
        );

        bottom.push(
            y_center
                + (-lower * half_h)
                    .max(MIN_HALF),
        );
    }

    (top, bottom)
}

// ============================================================
// DRAW ENVELOPE MIP
// ============================================================

fn draw_envelope_mip(
    pixmap: &mut Pixmap,
    buckets: &[MipBucket],
    bucket_size: usize,
    y_center: f32,
    half_h: f32,
    view: (usize, usize),
) {
    let (top, bottom) =
        envelope_contours(
            pixmap,
            buckets,
            bucket_size,
            y_center,
            half_h,
            view,
        );

    if top.is_empty()
        || bottom.is_empty()
    {
        return;
    }

    draw_filled_envelope(
        pixmap,
        &top,
        &bottom,
    );
}

// ============================================================
// RENDER PRINCIPAL
// ============================================================

/// Rasteriza a waveform na resolução EXATA pedida pela UI.
///
/// `w`:
///
/// ```text
/// largura final em pixels.
/// ```
///
/// `h`:
///
/// ```text
/// altura final em pixels.
/// ```
///
/// `view`:
///
/// ```text
/// região visível em samples da display wave.
/// ```
///
/// Exemplo:
///
/// ```text
/// waveform_image_rgba_mip(
///     &display,
///     &mip,
///     1200,
///     64,
///     (10000, 30000),
/// );
/// ```
///
/// O resultado sempre terá:
///
/// ```text
/// 1200 * 64 * 4
/// ```
///
/// bytes.
pub fn waveform_image_rgba_mip(
    mono: &[f32],
    mip: &Mipmap,
    w: u32,
    h: u32,
    view: (usize, usize),
) -> Option<Vec<u8>> {
    if mono.is_empty()
        || w == 0
        || h == 0
    {
        return None;
    }

    let start =
        view.0.min(mono.len());

    let end =
        view.1.min(mono.len());

    if end <= start {
        return None;
    }

    let mut pixmap =
        new_canvas(w, h);

    let center =
        h as f32 / 2.0;

    let half_h =
        (h as f32 / 2.0 - 4.0)
            .max(1.0);

    let sample_count =
        end - start;

    let samples_per_pixel =
        sample_count as f64
            / w as f64;

    // ========================================================
    // OSCILOSCÓPIO
    // ========================================================

    if samples_per_pixel
        < SCOPE_SAMPLES_PER_PX
    {
        draw_scope_lane(
            &mut pixmap,
            mono,
            center,
            half_h,
            (start, end),
        );

        return Some(
            pixmap.data().to_vec()
        );
    }

    // ========================================================
    // FALLBACK SEM MIPMAP
    // ========================================================

    if mip.levels.is_empty() {
        draw_envelope_lane(
            &mut pixmap,
            mono,
            center,
            half_h,
            (start, end),
        );

        return Some(
            pixmap.data().to_vec()
        );
    }

    // ========================================================
    // ESCOLHA DO MIP
    // ========================================================

    let level =
        mip.level_for(
            samples_per_pixel
        );

    let buckets =
        match mip.levels.get(level) {
            Some(level) => level,

            None => {
                draw_envelope_lane(
                    &mut pixmap,
                    mono,
                    center,
                    half_h,
                    (start, end),
                );

                return Some(
                    pixmap.data().to_vec()
                );
            }
        };

    let bucket_size =
        mip.bucket(level);

    // ========================================================
    // RENDER
    // ========================================================

    draw_envelope_mip(
        &mut pixmap,
        buckets,
        bucket_size,
        center,
        half_h,
        (start, end),
    );

    Some(
        pixmap.data().to_vec()
    )
}

// ============================================================
// API LEGADA
// ============================================================

/// Compatibilidade com a API antiga.
///
/// `peaks` são amplitudes positivas 0..1.
pub fn waveform_image_rgba_at(
    peaks: &[f32],
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    if peaks.is_empty()
        || w == 0
        || h == 0
    {
        return None;
    }

    let mut pixmap =
        new_canvas(w, h);

    let center =
        h as f32 / 2.0;

    let count =
        peaks.len();

    let mut paint =
        Paint::default();

    paint.set_color_rgba8(
        FG[0],
        FG[1],
        FG[2],
        FG[3],
    );

    paint.anti_alias = true;

    let amplitude_at =
        |x: u32| -> f32 {
            let start =
                ((x as f64
                    / w as f64)
                    * count as f64)
                    .floor()
                    as usize;

            let end =
                (((x + 1) as f64
                    / w as f64)
                    * count as f64)
                    .ceil()
                    as usize;

            if start >= peaks.len() {
                return 0.0;
            }

            let end =
                end.max(start + 1)
                    .min(peaks.len());

            peaks[start..end]
                .iter()
                .fold(
                    0.0f32,
                    |acc, &value| {
                        acc.max(value)
                    },
                )
        };

    let mut path =
        PathBuilder::new();

    for x in 0..w {
        let half =
            (
                amplitude_at(x)
                    .clamp(0.0, 1.0)
                    * center
            )
            .max(1.0);

        let y =
            center - half;

        if x == 0 {
            path.move_to(
                0.0,
                y,
            );
        } else {
            path.line_to(
                x as f32,
                y,
            );
        }
    }

    for x in (0..w).rev() {
        let half =
            (
                amplitude_at(x)
                    .clamp(0.0, 1.0)
                    * center
            )
            .max(1.0);

        path.line_to(
            x as f32,
            center + half,
        );
    }

    path.close();

    if let Some(path) =
        path.finish()
    {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    Some(
        pixmap.data().to_vec()
    )
}

/// Compatibilidade com API antiga.
pub fn waveform_image_rgba(
    peaks: &[f32],
    _color_index: i32,
    _is_midi: bool,
) -> Option<Vec<u8>> {
    waveform_image_rgba_at(
        peaks,
        WAVEFORM_IMG_WIDTH as u32,
        H,
    )
}

// ============================================================
// LOD LEGADO
// ============================================================

/// Gera imagens LOD.
///
/// ATENÇÃO:
///
/// Essa função existe somente para compatibilidade.
///
/// O pipeline moderno NÃO deve chamar isso a cada zoom.
///
/// Prefira:
///
/// ```text
/// let (display, mip) = build_display_wave(&mono);
/// ```
///
/// e depois:
///
/// ```text
/// waveform_image_rgba_mip(
///     &display,
///     &mip,
///     width,
///     height,
///     view,
/// );
/// ```
pub fn build_waveform_lods(
    mono: &[f32],
) -> WaveformLods {
    let base_width =
        WAVEFORM_IMG_WIDTH as u32;

    let sample_count =
        mono.len();

    let mut levels =
        Vec::with_capacity(
            LEVELS
        );

    let mut width =
        base_width;

    for _ in 0..LEVELS {
        if width < 32 {
            break;
        }

        let rgba =
            waveform_image_rgba_at_samples(
                mono,
                mono,
                width,
                (0, sample_count),
            )
            .unwrap_or_default();

        levels.push(
            WaveformLod {
                width,
                rgba,
            }
        );

        width /= 2;
    }

    WaveformLods {
        levels,
    }
}

/// Retorna o melhor LOD legado.
pub fn pick_lod(
    lods: &WaveformLods,
    target_width: u32,
) -> Option<&WaveformLod> {
    lods.best_level(
        target_width
    )
}

// ============================================================
// TESTES
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mipmap_levels_halve_and_merge() {
        let mono: Vec<f32> =
            (0..16)
                .map(|i| {
                    ((i as i32 % 5) - 2)
                        as f32
                        / 4.0
                })
                .collect();

        let mip =
            build_mipmap(&mono);

        assert!(
            !mip.levels.is_empty()
        );

        assert_eq!(
            mip.levels[0].len(),
            8
        );

        assert_eq!(
            mip.bucket(0),
            2
        );

        assert_eq!(
            mip.bucket(2),
            8
        );

        let top =
            mip.levels
                .last()
                .unwrap()[0];

        let expected_min =
            mono.iter()
                .copied()
                .fold(
                    f32::INFINITY,
                    f32::min,
                );

        let expected_max =
            mono.iter()
                .copied()
                .fold(
                    f32::NEG_INFINITY,
                    f32::max,
                );

        assert_eq!(
            top.min,
            expected_min
        );

        assert_eq!(
            top.max,
            expected_max
        );
    }

    #[test]
    fn mipmap_preserves_rms_information() {
        let mono =
            vec![1.0f32, -1.0, 0.5, -0.5];

        let mip =
            build_mipmap(&mono);

        let bucket =
            mip.levels[0][0];

        assert_eq!(
            bucket.count,
            2
        );

        assert_eq!(
            bucket.min,
            -1.0
        );

        assert_eq!(
            bucket.max,
            1.0
        );

        assert!(
            bucket.sum_sq > 1.9
                && bucket.sum_sq < 2.1
        );
    }

    #[test]
    fn level_for_picks_correct_bucket() {
        let mono =
            vec![0.0f32; 1000];

        let mip =
            build_mipmap(&mono);

        // < 2 spp sempre começa no level 0.
        assert_eq!(
            mip.level_for(1.5),
            0
        );

        // 3 spp → bucket de 2.
        assert_eq!(
            mip.bucket(
                mip.level_for(3.0)
            ),
            2
        );

        // 7.9 spp → bucket de 4.
        assert_eq!(
            mip.bucket(
                mip.level_for(7.9)
            ),
            4
        );

        // 8 spp → bucket de 8.
        assert_eq!(
            mip.bucket(
                mip.level_for(8.0)
            ),
            8
        );

        assert!(
            mip.bucket(
                mip.level_for(999.0)
            ) <= 1000
        );
    }

    #[test]
    fn scope_draws_single_line() {
        let n =
            256usize;

        let mono: Vec<f32> =
            (0..n)
                .map(|i| {
                    (i as f32 / 4.0)
                        .sin()
                        * 0.9
                })
                .collect();

        let mip =
            build_mipmap(&mono);

        let image =
            waveform_image_rgba_mip(
                &mono,
                &mip,
                512,
                64,
                (0, n),
            )
            .unwrap();

        let total =
            image.len() / 4;

        let opaque =
            image
                .chunks_exact(4)
                .filter(
                    |pixel| pixel[3] > 0
                )
                .count();

        assert!(
            opaque > 0,
            "osciloscópio precisa ser visível"
        );

        assert!(
            opaque < total / 3,
            "osciloscópio deve ser uma linha fina"
        );
    }

    #[test]
    fn downsample_preserves_transient() {
        let mut mono =
            vec![
                0.0f32;
                MONO_DISPLAY_CAP + 100
            ];

        let transient_index =
            MONO_DISPLAY_CAP / 2;

        mono[transient_index] =
            1.0;

        let display =
            downsample_mono(&mono);

        assert!(
            display
                .iter()
                .any(|&value| value >= 0.99),
            "transiente não foi preservado"
        );
    }

    #[test]
    fn downsample_keeps_normal_signal() {
        let mono =
            vec![
                -0.5,
                -0.2,
                0.1,
                0.8,
                -0.9,
                0.3,
            ];

        let display =
            downsample_mono(&mono);

        assert_eq!(
            display,
            mono
        );
    }

    #[test]
    fn display_wave_builds_mipmap() {
        let mono: Vec<f32> =
            (0..10000)
                .map(|i| {
                    (i as f32 * 0.01)
                        .sin()
                })
                .collect();

        let (display, mip) =
            build_display_wave(
                &mono
            );

        assert!(
            !display.is_empty()
        );

        assert!(
            !mip.levels.is_empty()
        );

        assert_eq!(
            mip.levels[0].len(),
            display.len().div_ceil(2)
        );
    }

    #[test]
    fn rasterizes_at_requested_height() {
        let mono: Vec<f32> =
            (0..1000)
                .map(|i| {
                    (i as f32 * 0.02)
                        .sin()
                })
                .collect();

        let mip =
            build_mipmap(&mono);

        let width =
            100u32;

        let height =
            37u32;

        let image =
            waveform_image_rgba_mip(
                &mono,
                &mip,
                width,
                height,
                (0, mono.len()),
            )
            .unwrap();

        assert_eq!(
            image.len(),
            (width * height * 4)
                as usize
        );
    }

    #[test]
    fn rasterizes_at_different_heights() {
        let mono: Vec<f32> =
            (0..2000)
                .map(|i| {
                    (i as f32 * 0.03)
                        .sin()
                })
                .collect();

        let mip =
            build_mipmap(&mono);

        for height in [
            16u32,
            32,
            48,
            64,
            80,
            128,
            256,
        ] {
            let image =
                waveform_image_rgba_mip(
                    &mono,
                    &mip,
                    400,
                    height,
                    (0, mono.len()),
                )
                .unwrap();

            assert_eq!(
                image.len(),
                (400 * height * 4)
                    as usize
            );
        }
    }

    #[test]
    fn rasterizes_different_views() {
        let mono: Vec<f32> =
            (0..10000)
                .map(|i| {
                    (i as f32 * 0.01)
                        .sin()
                })
                .collect();

        let mip =
            build_mipmap(&mono);

        let full =
            waveform_image_rgba_mip(
                &mono,
                &mip,
                512,
                64,
                (0, 10000),
            )
            .unwrap();

        let zoomed =
            waveform_image_rgba_mip(
                &mono,
                &mip,
                512,
                64,
                (4000, 5000),
            )
            .unwrap();

        assert_eq!(
            full.len(),
            zoomed.len()
        );

        assert_ne!(
            full,
            zoomed,
            "zoom deveria alterar o conteúdo renderizado"
        );
    }

    #[test]
    fn empty_audio_returns_none() {
        let mono =
            Vec::<f32>::new();

        let mip =
            build_mipmap(&mono);

        assert!(
            waveform_image_rgba_mip(
                &mono,
                &mip,
                100,
                64,
                (0, 0),
            )
            .is_none()
        );
    }

    #[test]
    fn invalid_view_returns_none() {
        let mono =
            vec![0.0f32; 100];

        let mip =
            build_mipmap(&mono);

        assert!(
            waveform_image_rgba_mip(
                &mono,
                &mip,
                100,
                64,
                (80, 20),
            )
            .is_none()
        );
    }

    #[test]
    fn zero_dimensions_return_none() {
        let mono =
            vec![0.0f32; 100];

        let mip =
            build_mipmap(&mono);

        assert!(
            waveform_image_rgba_mip(
                &mono,
                &mip,
                0,
                64,
                (0, 100),
            )
            .is_none()
        );

        assert!(
            waveform_image_rgba_mip(
                &mono,
                &mip,
                100,
                0,
                (0, 100),
            )
            .is_none()
        );
    }

    #[test]
    fn legacy_api_still_works() {
        let peaks =
            vec![
                0.1f32,
                0.3,
                0.8,
                0.2,
            ];

        let image =
            waveform_image_rgba_at(
                &peaks,
                200,
                64,
            )
            .unwrap();

        assert_eq!(
            image.len(),
            200 * 64 * 4
        );
    }

    #[test]
    fn legacy_lods_still_work() {
        let mono: Vec<f32> =
            (0..1000)
                .map(|i| {
                    (i as f32 * 0.01)
                        .sin()
                })
                .collect();

        let lods =
            build_waveform_lods(
                &mono
            );

        assert!(
            !lods.levels.is_empty()
        );

        let selected =
            pick_lod(
                &lods,
                500,
            );

        assert!(
            selected.is_some()
        );
    }
}