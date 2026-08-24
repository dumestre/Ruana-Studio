//! Renderização profissional de waveform em imagem RGBA.
//!
//! A waveform é rasterizada no Rust e entregue ao Slint como
//! uma única Image.
//!
//! Cada coluna de pixel representa um intervalo do áudio e
//! preserva:
//!
//! - pico positivo
//! - pico negativo
//! - RMS
//!
//! O envelope principal usa MIN/MAX real do áudio.
//! O RMS é usado para dar corpo visual à waveform sem
//! destruir os transientes.
//!
//! A resolução é adaptada à largura real da timeline,
//! evitando que a UI tenha que desenhar milhares de objetos.

use tiny_skia::*;

/// Largura usada no primeiro cache.
pub const WAVEFORM_IMG_WIDTH: usize = 2048;

/// Altura interna da waveform.
///
/// 512px oferece bastante resolução vertical e permite
/// reduzir depois sem destruir os detalhes.
const H: u32 = 512;

/// Intensidade do RMS dentro do envelope visual.
///
/// 0.0 = somente pico.
/// 1.0 = RMS domina completamente.
///
/// 0.35 costuma produzir um desenho parecido com DAWs:
/// transientes continuam definidos, mas o corpo do áudio
/// permanece visível.
const RMS_WEIGHT: f32 = 0.35;

/// Espessura mínima do desenho em pixels.
///
/// Evita que trechos extremamente baixos desapareçam
/// completamente devido ao rasterizador.
const MIN_HALF_HEIGHT: f32 = 0.75;

/// Limita uma amplitude para o domínio válido.
#[inline]
fn clamp_sample(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

/// Estatísticas de um trecho de áudio.
#[derive(Clone, Copy, Debug)]
struct ColumnStats {
    min: f32,
    max: f32,
    rms: f32,
}

/// Calcula MIN/MAX/RMS de um intervalo de samples.
#[inline]
fn column_stats(samples: &[f32], start: usize, end: usize) -> ColumnStats {
    if start >= samples.len() || start >= end {
        return ColumnStats {
            min: 0.0,
            max: 0.0,
            rms: 0.0,
        };
    }

    let end = end.min(samples.len());
    let slice = &samples[start..end];

    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut sum_sq = 0.0f32;

    for &sample in slice {
        let s = clamp_sample(sample);

        min = min.min(s);
        max = max.max(s);

        sum_sq += s * s;
    }

    let rms = (sum_sq / slice.len() as f32).sqrt();

    ColumnStats { min, max, rms }
}

/// Rasteriza uma waveform diretamente a partir dos samples.
///
/// Esta é a versão ideal quando o áudio original ainda está
/// disponível.
///
/// Cada pixel horizontal representa um intervalo real do áudio.
pub fn waveform_image_from_samples(
    samples: &[f32],
    w: u32,
) -> Option<Vec<u8>> {
    if samples.is_empty() || w == 0 {
        return None;
    }

    let h = H;

    let mut pixmap = Pixmap::new(w, h)?;

    let center = h as f32 * 0.5;
    let width = w as usize;
    let sample_count = samples.len();

    let mut paint = Paint::default();

    paint.set_color_rgba8(
        0x0A,
        0x0A,
        0x0A,
        255,
    );

    paint.anti_alias = true;

    let mut top: Vec<f32> = Vec::with_capacity(width);
    let mut bottom: Vec<f32> = Vec::with_capacity(width);

    for x in 0..width {
        let start = (
            x as f32
            / width as f32
            * sample_count as f32
        )
            .floor() as usize;

        let end = (
            (x + 1) as f32
            / width as f32
            * sample_count as f32
        )
            .ceil()
            as usize;

        let end = end.max(start + 1).min(sample_count);

        let stats = column_stats(
            samples,
            start,
            end,
        );

        //
        // Envelope físico.
        //
        // O pico positivo determina a parte superior.
        // O pico negativo determina a parte inferior.
        //
        let positive_peak = stats.max.max(0.0);
        let negative_peak = stats.min.min(0.0);

        //
        // Corpo RMS.
        //
        // Mantemos o sinal do envelope original e usamos
        // RMS apenas para preencher o corpo.
        //
        let rms = stats.rms * RMS_WEIGHT;

        let upper = positive_peak.max(rms);
        let lower = negative_peak.min(-rms);

        let upper = upper.clamp(0.0, 1.0);
        let lower = lower.clamp(-1.0, 0.0);

        let half_top = (
            upper * center
        )
            .max(MIN_HALF_HEIGHT);

        let half_bottom = (
            -lower * center
        )
            .max(MIN_HALF_HEIGHT);

        top.push(
            center - half_top
        );

        bottom.push(
            center + half_bottom
        );
    }

    //
    // Suavização horizontal mínima.
    //
    // Não fazemos blur.
    // Apenas conectamos os pontos para formar um envelope
    // contínuo.
    //
    let mut path = PathBuilder::new();

    if width == 1 {
        let y_top = top[0];
        let y_bottom = bottom[0];

        path.move_to(
            0.0,
            y_top,
        );

        path.line_to(
            0.0,
            y_bottom,
        );

        path.close();
    } else {
        //
        // SUPERIOR
        //
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

        //
        // INFERIOR
        //
        for x in (0..width).rev() {
            path.line_to(
                x as f32,
                bottom[x],
            );
        }

        path.close();
    }

    if let Some(path) = path.finish() {
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

/// Rasteriza uma waveform a partir dos pontos de amplitude.
///
/// Mantida para compatibilidade com o sistema atual.
///
/// Os pontos representam amplitude simétrica 0..1.
/// Para máxima fidelidade, prefira
/// `waveform_image_from_samples()`.
pub fn waveform_image_rgba_at(
    peaks: &[f32],
    w: u32,
) -> Option<Vec<u8>> {
    if peaks.is_empty() || w == 0 {
        return None;
    }

    let h = H;

    let mut pixmap = Pixmap::new(
        w,
        h,
    )?;

    let center = h as f32 * 0.5;

    let width = w as usize;
    let point_count = peaks.len();

    let mut paint = Paint::default();

    paint.set_color_rgba8(
        0x0A,
        0x0A,
        0x0A,
        255,
    );

    paint.anti_alias = true;

    let mut top = Vec::with_capacity(width);
    let mut bottom = Vec::with_capacity(width);

    for x in 0..width {
        let start = (
            x as f32
            / width as f32
            * point_count as f32
        )
            .floor() as usize;

        let end = (
            (x + 1) as f32
            / width as f32
            * point_count as f32
        )
            .ceil()
            as usize;

        let end = end
            .max(start + 1)
            .min(point_count);

        let amplitude = peaks[start.min(point_count - 1)..end]
            .iter()
            .fold(
                0.0f32,
                |max, &value| {
                    max.max(
                        value.abs()
                    )
                },
            )
            .clamp(0.0, 1.0);

        let half = (
            amplitude * center
        )
            .max(MIN_HALF_HEIGHT);

        top.push(
            center - half
        );

        bottom.push(
            center + half
        );
    }

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

    Some(
        pixmap.data().to_vec()
    )
}

/// Compatibilidade com o código atual.
///
/// Renderiza usando a resolução padrão.
pub fn waveform_image_rgba(
    peaks: &[f32],
    _color_index: i32,
    _is_midi: bool,
) -> Option<Vec<u8>> {
    waveform_image_rgba_at(
        peaks,
        WAVEFORM_IMG_WIDTH as u32,
    )
}
Porém tem uma decisão arquitetural importante

O seu decoder já guarda:

pub samples: Vec<f32>,
pub interleaved: Vec<f32>,

e deixa claro que samples é o downmix mono usado somente para waveform, enquanto interleaved preserva o áudio original para reprodução.

Isso é exatamente o que precisamos.

Então, em vez de:

samples
 ↓
waveform_points()
 ↓
Vec<f32>
 ↓
waveform_image_rgba_at()

eu faria:

samples
 ↓
waveform_image_from_samples()
 ↓
RGBA
 ↓
Slint Image

O waveform_points() pode continuar existindo para compatibilidade, testes ou fallback, mas não precisa ser o caminho principal.

E tem outra coisa que eu corrigiria

Seu waveform_points() atualmente usa chunks calculados assim:

let chunk_size =
    (data.samples.len() as f32 / points as f32)
        .max(1.0) as usize;

Isso pode deixar o último trecho mal distribuído e, em determinadas relações entre número de samples e pontos, a resolução espacial não fica exatamente uniforme.

Para DAW profissional, prefiro:

pixel 0 → samples [0 .......... 123]
pixel 1 → samples [124 ........ 247]
pixel 2 → samples [248 ........ 371]
...

com início/fim calculados pela posição relativa, como no renderer acima.

Stereo

Aqui tem uma questão que eu considero obrigatória para a Ruana.

Seu decoder atualmente transforma stereo em mono para a waveform:

.map(|c| if c.len() == 2 {
    (c[0] + c[1]) * 0.5
})

Isso é aceitável para uma waveform simples, mas não é o ideal para uma DAW profissional.

Imagine:

L = +1
R = -1

O downmix dá:

(+1 + -1) / 2 = 0

Visualmente:

silêncio

quando na realidade temos dois canais completamente ativos.

E seu próprio teste demonstra exatamente esse caso: o downmix resulta em zero enquanto os canais originais continuam intactos.

O renderer definitivo deveria ser stereo

Para um clip estéreo:

┌──────────────────────────────────────────────┐
│              LEFT CHANNEL                    │
│       ╭──╮      ╭────╮       ╭──╮            │
│───────╯  ╰──────╯    ╰───────╯  ╰──────       │
├──────────────────────────────────────────────┤
│              RIGHT CHANNEL                   │
│────╮      ╭──────╮       ╭────────╮           │
│    ╰──────╯      ╰───────╯        ╰────       │
└──────────────────────────────────────────────┘

Esse é o tipo de representação que eu colocaria na Ruana Studio, não apenas um mono centralizado.

E você já tem channel(index) preparado para extrair L/R do buffer intercalado.

Minha recomendação para a Ruana

Eu estruturaria o sistema em quatro níveis:

AudioDecoder
     │
     ├── interleaved ───────► Audio Engine
     │
     └── waveform data
             │
             ├── Mono
             │
             └── Stereo
                    │
                    ▼
             Waveform Renderer
                    │
                    ▼
              tiny-skia RGBA
                    │
                    ▼
                Slint Image

E depois:

Zoom 25%
    ↓
render 300 px

Zoom 50%
    ↓
render 600 px

Zoom 100%
    ↓
render 1200 px

Zoom 200%
    ↓
render 2400 px

Ou seja: não esticar uma imagem de 2048 px indefinidamente.

Seu código atual já tem a ideia certa de renderizar novamente conforme a largura. O que eu mudaria agora é a qualidade do envelope e, principalmente, preservar L/R no desenho.

Isso deixa a arquitetura muito mais próxima de uma DAW de verdade, em vez de apenas "uma waveform bonita".