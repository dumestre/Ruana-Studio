//! Time-stretch de clips de audio: a base do timewarp estilo Cubase.
//!
//! Envolve o crate `timestretch` (v0.15.0, robmorgan/timestretch-rs) com as
//! garantias que o motor e o worker precisam:
//!
//! - ratio validado/clampado na faixa suportada pelo crate (0.01..=100.0);
//! - entrada/saida SEMPRE no formato do motor: f32 intercalado, -1.0..1.0;
//! - ratio 1.0 = passthrough exato (o crate ja faz isso e nos evitamos ate
//!   a montagem dos parametros);
//! - comprimento previsivel: `round(frames * ratio)`, o que permite ao motor
//!   mapear posicao do projeto -> posicao do buffer esticado sem surpresa.
//!
//! A matematica de "quanto esticar" (mapa de tempo + marcadores de AudioWarp)
//! NAO vive aqui: ela e do documento (`daw-core`). Este modulo so executa.

use crate::buffer::AudioBuffer;
use crate::mapped_buffer::AudioBufferKind;
use timestretch::{StretchError, StretchParams};

/// Minimo/maximo de ratio aceito pelo `timestretch` (`stretch/params.rs`).
pub const RATIO_MIN: f64 = 0.01;
pub const RATIO_MAX: f64 = 100.0;

/// Taxa de amostragem aceita pelo crate.
pub const RATE_MIN: u32 = 8_000;
pub const RATE_MAX: u32 = 192_000;

/// Clampa um ratio para a faixa suportada.
///
/// `NaN`/infinito viram `1.0` (identidade): um ratio corrompido nunca pode
/// virar som esticado aleatorio, e sim o audio como ele e.
pub fn clamp_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() {
        ratio.clamp(RATIO_MIN, RATIO_MAX)
    } else {
        1.0
    }
}

/// O ratio e (numericamente) identidade?
pub fn is_identity(ratio: f64) -> bool {
    (ratio - 1.0).abs() <= f64::EPSILON
}

/// Frames (amostras por canal) de um buffer intercalado.
pub fn frames_of(samples: usize, channels: u16) -> usize {
    samples / channels.max(1) as usize
}

/// Estica `input` (f32 intercalado) por `ratio`.
///
/// - `ratio > 1.0` => saida MAIS LONGA (mais lenta);
/// - `ratio < 1.0` => saida mais curta (mais rapida);
/// - `ratio == 1.0` => copia exata da entrada.
///
/// O comprimento de saida e `round(frames * ratio)` (garantido pelo render
/// offline do crate), exceto no caminho de identidade, onde e o proprio
/// comprimento de entrada.
pub fn stretch_interleaved(
    input: &[f32],
    sample_rate: u32,
    channels: u16,
    ratio: f64,
) -> Result<Vec<f32>, StretchError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let ratio = clamp_ratio(ratio);

    if is_identity(ratio) {
        return Ok(input.to_vec());
    }

    let channels = channels.max(1);
    let params = StretchParams::new(ratio)
        .with_sample_rate(sample_rate.clamp(RATE_MIN, RATE_MAX))
        .with_channels(channels as u32)
        // Normalizar mudaria o ganho do clip: o ganho e do clip (`gain_db`)
        // e o motor aplica depois. Aqui o sinal tem de sair como entrou.
        .with_normalize(false);

    timestretch::stretch(input, &params)
}

// ============================================================================
// TIMEWARP: QUANTO O MOTOR LE (rate) E O QUE O WORKER RENDERIZA (target)
// ============================================================================

/// Frames da fonte consumidos por frame de saida.
///
/// Esta e a UNICA conta que o timewarp acrescenta ao motor, e ela cobre os
/// tres estados possiveis sem ramificar o render:
///
/// - LINEAR (`musical == false`) -> 1.0, o comportamento historico: 1 frame
///   da fonte por frame de saida (tempo absoluto).
/// - MUSICAL tocando o buffer ORIGINAL (varispeed, enquanto o render nao
///   chegou) -> `1 / ratio_pedido`: o audio acompanha o tempo e o PITCH
///   acompanha junto (e o varispeed do Cubase durante o gesto).
/// - MUSICAL tocando um buffer JA ESTICADO por `rendered_ratio` ->
///   `rendered_ratio / ratio_pedido`. Se o tempo mudou depois do render,
///   isto reescala o buffer pronto sem esperar um novo render; se o tempo
///   e o mesmo, da exatamente 1.0 (leitura 1:1, sem reamostragem).
///
/// Qualquer valor degenerado (fonte desconhecida, span zero, ratio nao
/// finito) cai em 1.0: um bug de documento nunca pode virar som acelerado.
pub fn source_rate(
    musical: bool,
    source_seconds: f64,
    span_seconds: f64,
    rendered_ratio: f64,
) -> f64 {
    if !musical || source_seconds <= 0.0 || span_seconds <= 0.0 {
        return 1.0;
    }

    let needed = span_seconds / source_seconds;

    if !needed.is_finite() || needed <= 0.0 {
        return 1.0;
    }

    let rendered = if rendered_ratio.is_finite() && rendered_ratio > 0.0 {
        rendered_ratio
    } else {
        1.0
    };

    let rate = rendered / needed;

    if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        1.0
    }
}

/// O que esticar e para quanto: uma assinatura estavel por clip musical.
///
/// E o contrato entre TRES donos diferentes:
///
/// - o snapshot do motor, que pergunta ao cache "ja existe esse render?";
/// - o worker de background, que renderiza e insere no cache;
/// - a UI, que pode mostrar "processando".
///
/// A chave e derivada do alvo (arquivo + regiao + span + forma do warp),
/// entao os tres chegam a MESMA string sem estado compartilhado.
#[derive(Debug, Clone, PartialEq)]
pub struct StretchTarget {
    /// Arquivo de origem.
    pub path: String,
    /// Onde a regiao comeca no arquivo, em segundos.
    pub source_start_seconds: f64,
    /// Comprimento NATURAL da regiao (o denominador do ratio).
    pub source_seconds: f64,
    /// Span do clip no projeto, em segundos (pelo mapa de tempo).
    pub span_seconds: f64,
    /// Ratio global = span / fonte.
    pub ratio: f64,
    /// Plano por trechos (marcadores de AudioWarp; um trecho so no caso
    /// simples). E o que o worker executa.
    pub plan: daw_core::WarpPlan,
}

impl StretchTarget {
    /// Vale a pena renderizar? Diferencas abaixo de 1 ms por segundo sao
    /// inaudiveis e so gastariam CPU/RAM.
    pub fn needs_render(&self) -> bool {
        (self.ratio - 1.0).abs() > 1e-3
    }

    /// Frames de saida esperados na taxa dada.
    pub fn frames_out(&self, sample_rate: u32) -> usize {
        let frames = self.source_seconds * sample_rate.max(1) as f64 * self.ratio;

        if frames.is_finite() && frames > 0.0 {
            frames.round() as usize
        } else {
            0
        }
    }

    /// Chave do cache de renders.
    ///
    /// Arredonda em milissegundos de propositio: e o suficiente para dois
    /// pedidos "iguais" colidirem (o arrasto fica estavel) e para um tempo
    /// genuinamente diferente gerar um alvo novo. A forma do warp entra na
    /// chave (numero de trechos + ratio de cada um).
    pub fn key(&self) -> String {
        let mut key = format!(
            "{}#s{:.3}+{:.3}@{:.3}",
            self.path, self.source_start_seconds, self.source_seconds, self.span_seconds
        );

        for seg in &self.plan.segments {
            key.push_str(&format!(
                "|{:.3}:{:.4}",
                seg.source_offset_seconds, seg.ratio
            ));
        }

        key
    }
}

/// Monta o alvo de stretch de um clip a partir do documento.
///
/// `natural_seconds` e o comprimento do arquivo (ou da regiao) em segundos,
/// medido por quem tem o audio decodificado. Devolve `None` para MIDI, para
/// clip em modo LINEAR e para fonte degenerada - ou seja, so existe alvo
/// quando ha mesmo algo a esticar.
pub fn stretch_target_for(
    clip: &daw_core::Clip,
    tempo_map: &daw_core::TempoMap,
    natural_seconds: f64,
) -> Option<StretchTarget> {
    if clip.is_midi() || !clip.is_musical() {
        return None;
    }

    let path = clip.file_path.clone()?;

    if !natural_seconds.is_finite() || natural_seconds <= 1e-3 || clip.length_beats <= 0.0 {
        return None;
    }

    // O offset de origem esta em beats; converte-lo com o BPM do proprio
    // ponto do clip e o que mantem o trecho lido alinhado com o mapa de
    // tempo (o motor faz a mesma conta com o BPM vivo).
    let bpm_at_clip = tempo_map.bpm_at(clip.start_beat).max(1.0) as f64;
    let source_start_seconds =
        (clip.source_start_beats as f64 * 60.0 / bpm_at_clip).clamp(0.0, natural_seconds);

    let source_seconds = natural_seconds - source_start_seconds;

    if source_seconds <= 1e-3 {
        return None;
    }

    let span_seconds = tempo_map.beats_to_seconds(clip.start_beat + clip.length_beats)
        - tempo_map.beats_to_seconds(clip.start_beat);

    if !span_seconds.is_finite() || span_seconds <= 1e-3 {
        return None;
    }

    let plan = daw_core::plan_for_clip(clip, tempo_map, source_seconds);

    Some(StretchTarget {
        path,
        source_start_seconds,
        source_seconds,
        span_seconds,
        ratio: span_seconds / source_seconds,
        plan,
    })
}
// ============================================================================
// RENDER: ARQUIVO -> BUFFER ESTICADO (roda no worker, nunca na RT)
// ============================================================================

/// Frames de crossfade nas emendas entre trechos do warp (~2.7 ms a 48 kHz).
const SEGMENT_XFADE_FRAMES: usize = 128;

/// Le um trecho do buffer como intercalado L/R.
///
/// Usa o `read` interpolado do proprio buffer (mesma leitura da RT), entao o
/// trecho lido corresponde exatamente ao que o motor tocaria em modo linear.
/// Pedir alem do fim devolve zero — o buffer ja clampa.
fn read_interleaved(buffer: &AudioBufferKind, start_frame: f64, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);

    for i in 0..frames {
        let (l, r) = buffer.read(start_frame + i as f64);
        out.push(l);
        out.push(r);
    }

    out
}

/// Emenda `src` (intercalado L/R) no fim de `dst`, com crossfade equal-power.
///
/// O corte seco na emenda de dois trechos esticados independentemente e um
/// degrau de amplitude no meio do clip — o mesmo problema que a rampa
/// anti-clique resolve nas bordas. O crossfade sobrepoe as ultimas `xfade`
/// amostras do que ja estava com as primeiras do trecho novo.
fn append_with_crossfade(dst: &mut AudioBuffer, src: &[f32], xfade: usize) {
    let src_frames = src.len() / 2;
    let dst_frames = dst.left.len();

    if dst_frames == 0 || xfade == 0 || src_frames == 0 {
        for i in 0..src_frames {
            dst.left.push(src[i * 2]);
            dst.right.push(src[i * 2 + 1]);
        }

        return;
    }

    let fade = xfade.min(dst_frames).min(src_frames);

    for i in 0..fade {
        // Equal-power: somar linearmente deixaria um vale na emenda.
        let t = (i as f32 + 0.5) / fade as f32;
        let a = (t * std::f32::consts::FRAC_PI_2).cos();
        let b = (t * std::f32::consts::FRAC_PI_2).sin();

        let di = dst_frames - fade + i;

        dst.left[di] = dst.left[di] * a + src[i * 2] * b;
        dst.right[di] = dst.right[di] * a + src[i * 2 + 1] * b;
    }

    for i in fade..src_frames {
        dst.left.push(src[i * 2]);
        dst.right.push(src[i * 2 + 1]);
    }
}

/// Renderiza o stretch de um alvo a partir de um buffer JA DECODIFICADO.
///
/// Trabalha sobre o buffer que o motor ja tem em memoria (L/R na taxa de
/// saida) em vez de reabrir o arquivo: o worker nao paga decode nem I/O, e o
/// resultado ja sai no formato que a RT le (`AudioBuffer` plano).
///
/// Caso simples (sem marcadores): um unico stretch do trecho. Com marcadores
/// de AudioWarp: cada trecho e esticado pelo seu proprio ratio e emendado
/// com crossfade.
pub fn render_stretch(
    buffer: &AudioBufferKind,
    target: &StretchTarget,
) -> Result<AudioBuffer, StretchError> {
    let rate = buffer.sample_rate().max(1);

    let mut out = AudioBuffer {
        left: Vec::new(),
        right: Vec::new(),
        sample_rate: rate,
    };

    for seg in &target.plan.segments {
        let frames = (seg.source_seconds * rate as f64).round().max(1.0) as usize;
        let start_frame = (target.source_start_seconds + seg.source_offset_seconds) * rate as f64;

        let interleaved = read_interleaved(buffer, start_frame, frames);

        if interleaved.is_empty() {
            continue;
        }

        let stretched = stretch_interleaved(&interleaved, rate, 2, seg.ratio)?;

        if stretched.is_empty() {
            continue;
        }

        append_with_crossfade(&mut out, &stretched, SEGMENT_XFADE_FRAMES);
    }

    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    /// Senoide de `seconds` segundos, `freq` Hz, mono.
    fn sine(freq: f64, seconds: f64) -> Vec<f32> {
        let n = (RATE as f64 * seconds) as usize;

        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / RATE as f64).sin() as f32)
            .collect()
    }

    /// Cruzamentos por zero por segundo: mede o PITCH, nao a duracao.
    fn zero_crossings_per_second(samples: &[f32]) -> f64 {
        if samples.len() < 2 {
            return 0.0;
        }

        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();

        let seconds = samples.len() as f64 / RATE as f64;

        if seconds <= 0.0 {
            return 0.0;
        }

        crossings as f64 / seconds
    }

    #[test]
    fn ratio_um_e_passthrough_exato() {
        let input = sine(440.0, 0.1);
        let out = stretch_interleaved(&input, RATE, 1, 1.0).unwrap();

        assert_eq!(out, input);
    }

    #[test]
    fn ratio_maior_alonga_e_preserva_o_pitch() {
        let input = sine(440.0, 0.25);
        let out = stretch_interleaved(&input, RATE, 1, 2.0).unwrap();

        // Comprimento ~2x (o render offline garante o comprimento por
        // construcao; 1% de folga cobre arredondamento de frames).
        let expected = input.len() as f64 * 2.0;
        let got = out.len() as f64;

        assert!(
            (got - expected).abs() / expected < 0.01,
            "comprimento {got} != esperado {expected}"
        );

        // O pitch NAO pode mudar: ~440 cruzamentos/s antes e depois.
        let before = zero_crossings_per_second(&input);
        let after = zero_crossings_per_second(&out);

        assert!(
            (after - before).abs() / before < 0.05,
            "pitch mudou: {before} -> {after} cruzamentos/s"
        );
    }

    #[test]
    fn ratio_menor_encurta_e_preserva_o_pitch() {
        let input = sine(440.0, 0.25);
        let out = stretch_interleaved(&input, RATE, 1, 0.5).unwrap();

        let expected = input.len() as f64 * 0.5;
        let got = out.len() as f64;

        assert!(
            (got - expected).abs() / expected < 0.01,
            "comprimento {got} != esperado {expected}"
        );

        let before = zero_crossings_per_second(&input);
        let after = zero_crossings_per_second(&out);

        assert!(
            (after - before).abs() / before < 0.05,
            "pitch mudou: {before} -> {after} cruzamentos/s"
        );
    }

    #[test]
    fn estereo_mantem_o_numero_de_canais() {
        // L = 440 Hz, R = 660 Hz: intercalado.
        let l = sine(440.0, 0.2);
        let r = sine(660.0, 0.2);

        let mut input = Vec::with_capacity(l.len() * 2);
        for i in 0..l.len() {
            input.push(l[i]);
            input.push(r[i]);
        }

        let out = stretch_interleaved(&input, RATE, 2, 1.5).unwrap();

        assert_eq!(out.len() % 2, 0, "saida estereo tem de ficar par");

        let frames_in = frames_of(input.len(), 2);
        let frames_out = frames_of(out.len(), 2);
        let expected = frames_in as f64 * 1.5;

        assert!(
            (frames_out as f64 - expected).abs() / expected < 0.01,
            "frames {frames_out} != esperado {expected}"
        );
    }

    #[test]
    fn ratio_invalido_e_clampado() {
        assert_eq!(clamp_ratio(f64::NAN), 1.0);
        assert_eq!(clamp_ratio(f64::INFINITY), 1.0);
        assert_eq!(clamp_ratio(0.0), RATIO_MIN);
        assert_eq!(clamp_ratio(-3.0), RATIO_MIN);
        assert_eq!(clamp_ratio(1_000.0), RATIO_MAX);
        assert_eq!(clamp_ratio(1.25), 1.25);
    }

    #[test]
    fn entrada_vazia_devolve_vazio() {
        let out = stretch_interleaved(&[], RATE, 2, 2.0).unwrap();

        assert!(out.is_empty());
    }

    #[test]
    fn taxa_fora_da_faixa_nao_quebra() {
        // O crate so aceita 8k..192k; taxas estranhas sao clampadas em vez
        // de virar erro no meio do render de um clip.
        let input = sine(440.0, 0.1);
        let out = stretch_interleaved(&input, 1_000, 1, 1.2).unwrap();

        assert!(!out.is_empty());

        let out = stretch_interleaved(&input, 999_999, 1, 1.2).unwrap();

        assert!(!out.is_empty());
    }
}
#[cfg(test)]
mod timewarp_tests {
    use super::*;
    use daw_core::{Clip, TempoMap, TimeMode, WarpMarker};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn linear_le_um_frame_por_frame() {
        assert!(close(source_rate(false, 2.0, 1.0, 1.0), 1.0));
        // Mesmo com numeros de musical, o modo linear manda.
        assert!(close(source_rate(false, 0.5, 2.0, 1.0), 1.0));
    }

    #[test]
    fn musical_sem_render_toca_varispeed() {
        // Fonte de 0.5 s num span de 1.0 s: precisa esticar 2x, e o buffer
        // original (rendered 1.0) e lido na METADE da velocidade.
        let rate = source_rate(true, 0.5, 1.0, 1.0);

        assert!(close(rate, 0.5), "rate {rate}");
    }

    #[test]
    fn musical_com_render_pronto_le_um_para_um() {
        // Buffer ja renderizado com o ratio pedido: leitura 1:1.
        let rate = source_rate(true, 0.5, 1.0, 2.0);

        assert!(close(rate, 1.0), "rate {rate}");
    }

    #[test]
    fn tempo_mudou_depois_do_render_reescala_o_buffer_pronto() {
        // Fonte de 0.5 s esticada 2x => o buffer tem 1.0 s de material.
        // O tempo dobrou, entao o span do clip caiu de 1.0 s para 0.5 s:
        // esse 1.0 s de material tem de caber em 0.5 s, ou seja lido a 2x
        // (compressao liquida de 0.5x em relacao a fonte). Sem esperar um
        // novo render o audio ja casa com a grade.
        let rate = source_rate(true, 0.5, 0.5, 2.0);

        assert!(close(rate, 2.0), "rate {rate}");
    }

    #[test]
    fn numeros_degenerados_caem_em_um() {
        for (src, span, rendered) in [
            (0.0, 1.0, 1.0),
            (-1.0, 1.0, 1.0),
            (f64::NAN, 1.0, 1.0),
            (1.0, 0.0, 1.0),
            (1.0, f64::INFINITY, 1.0),
            (1.0, 1.0, f64::NAN),
            (1.0, 1.0, 0.0),
        ] {
            let rate = source_rate(true, src, span, rendered);

            assert!(
                close(rate, 1.0),
                "src {src} span {span} rendered {rendered} -> {rate}"
            );
        }
    }

    fn musical_clip(len_beats: f32, start: f32) -> Clip {
        let mut clip = Clip::audio(7, 0, "loop", start, len_beats);
        clip.file_path = Some("C:/samples/loop.wav".into());
        clip.time_mode = TimeMode::Musical;
        clip
    }

    #[test]
    fn alvo_so_existe_para_clip_musical_de_audio() {
        let map = TempoMap::from_bpm(120.0);

        let mut linear = musical_clip(2.0, 0.0);
        linear.time_mode = TimeMode::Linear;

        assert!(stretch_target_for(&linear, &map, 1.0).is_none());

        let mut midi = Clip::midi(1, 0, "padrao", 0.0, 2.0);
        midi.time_mode = TimeMode::Musical;

        assert!(stretch_target_for(&midi, &map, 1.0).is_none());

        assert!(stretch_target_for(&musical_clip(2.0, 0.0), &map, 0.0).is_none());
    }

    #[test]
    fn alvo_musical_calcula_ratio_e_frames() {
        // 2 beats a 120 BPM = 1.0 s de span; fonte de 0.5 s -> 2x.
        let map = TempoMap::from_bpm(120.0);
        let clip = musical_clip(2.0, 0.0);
        let target = stretch_target_for(&clip, &map, 0.5).expect("alvo");

        assert!(
            close(target.span_seconds, 1.0),
            "span {}",
            target.span_seconds
        );
        assert!(close(target.source_seconds, 0.5));
        assert!(close(target.ratio, 2.0));
        assert!(target.needs_render());
        assert_eq!(target.frames_out(48_000), 48_000);
    }

    #[test]
    fn alvo_desconta_o_offset_de_origem_da_fonte() {
        // Clip que comeca 1 beat dentro do arquivo (0.5 s a 120 BPM), num
        // arquivo de 2.0 s: a regiao tem 1.5 s.
        let map = TempoMap::from_bpm(120.0);
        let mut clip = musical_clip(2.0, 4.0);
        clip.source_start_beats = 1.0;

        let target = stretch_target_for(&clip, &map, 2.0).expect("alvo");

        assert!(close(target.source_start_seconds, 0.5));
        assert!(close(target.source_seconds, 1.5));
        // span de 2 beats = 1.0 s / 1.5 s de fonte.
        assert!(close(target.ratio, 1.0 / 1.5));
    }

    #[test]
    fn chave_muda_com_o_tempo_e_com_o_warp() {
        let map = TempoMap::from_bpm(120.0);
        let clip = musical_clip(2.0, 0.0);
        let base = stretch_target_for(&clip, &map, 0.5).expect("alvo").key();

        let mut faster = TempoMap::from_bpm(140.0);
        faster.set_change(0.0, 140.0);
        let other = stretch_target_for(&clip, &faster, 0.5).expect("alvo").key();

        assert_ne!(base, other, "tempo diferente tem de gerar alvo novo");

        let mut warped = clip.clone();
        warped.warp_markers = vec![WarpMarker::new(0.25, 0.5)];
        let with_warp = stretch_target_for(&warped, &map, 0.5).expect("alvo").key();

        assert_ne!(base, with_warp, "warp diferente tem de gerar alvo novo");

        // Mesmos parametros -> mesma chave (o arrasto nao pode gerar
        // renders infinitos).
        let again = stretch_target_for(&clip, &map, 0.5).expect("alvo").key();

        assert_eq!(base, again);
    }
}
#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::buffer::AudioBuffer;
    use crate::mapped_buffer::AudioBufferKind;
    use daw_core::{Clip, TempoMap, TimeMode};
    use std::sync::Arc;

    const RATE: u32 = 48_000;

    fn sine(freq: f64, seconds: f64) -> Vec<f32> {
        let n = (RATE as f64 * seconds) as usize;

        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / RATE as f64).sin() as f32)
            .collect()
    }

    fn buffer_from_lr(left: Vec<f32>, right: Vec<f32>) -> AudioBufferKind {
        AudioBufferKind::Memory(Arc::new(AudioBuffer {
            left,
            right,
            sample_rate: RATE,
        }))
    }

    fn musical_target(natural_seconds: f64, len_beats: f32) -> StretchTarget {
        let mut clip = Clip::audio(1, 0, "loop", 0.0, len_beats);
        clip.file_path = Some("C:/samples/loop.wav".into());
        clip.time_mode = TimeMode::Musical;

        let map = TempoMap::from_bpm(120.0);

        stretch_target_for(&clip, &map, natural_seconds).expect("alvo")
    }

    #[test]
    fn render_alonga_para_o_span_do_clip() {
        // Fonte de 1.0 s; clip de 4 beats a 120 BPM = 2.0 s de span -> 2x.
        let buffer = buffer_from_lr(sine(440.0, 1.0), sine(660.0, 1.0));
        let target = musical_target(1.0, 4.0);

        assert!((target.ratio - 2.0).abs() < 1e-6, "ratio {}", target.ratio);

        let out = render_stretch(&buffer, &target).expect("render");

        assert_eq!(out.sample_rate, RATE);

        let expected = target.frames_out(RATE) as f64;
        let got = out.left.len() as f64;

        assert!(
            (got - expected).abs() / expected < 0.02,
            "frames {got} != esperado {expected}"
        );

        assert_eq!(out.left.len(), out.right.len());
        assert!(out.left.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn render_le_a_regiao_certa_do_arquivo() {
        // Arquivo com a PRIMEIRA metade muda e a segunda em senoide. O
        // clip comeca em 0.5 s (offset de origem), entao a saida tem de
        // ser a parte que soa - nao o silencio.
        let n = (RATE as f64 * 1.0) as usize;
        let mut left = vec![0.0f32; n];
        let loud = sine(440.0, 0.5);

        for (i, s) in loud.iter().enumerate() {
            left[n / 2 + i] = *s;
        }

        let buffer = buffer_from_lr(left, vec![0.0; n]);

        let mut clip = Clip::audio(1, 0, "loop", 0.0, 4.0);
        clip.file_path = Some("C:/samples/loop.wav".into());
        clip.time_mode = TimeMode::Musical;
        // 1 beat a 120 BPM = 0.5 s de offset de origem.
        clip.source_start_beats = 1.0;

        let map = TempoMap::from_bpm(120.0);
        let target = stretch_target_for(&clip, &map, 1.0).expect("alvo");

        let out = render_stretch(&buffer, &target).expect("render");

        let peak = out.left.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));

        assert!(peak > 0.5, "a regiao lida nao foi a que soa (pico {peak})");
    }

    #[test]
    fn render_com_marcadores_mantem_o_span_e_sem_cliques() {
        // Dois trechos: 0.5 s -> 1.0 s (2x) e 0.5 s -> 1.5 s (3x), num clip
        // de 4 beats (2.0 s de span). O total tem de continuar batendo com o
        // span (e nao com a soma ingenua dos trechos) porque o ratio de cada
        // trecho ja foi calculado para isso.
        let buffer = buffer_from_lr(sine(440.0, 1.0), sine(440.0, 1.0));

        let mut clip = Clip::audio(1, 0, "loop", 0.0, 4.0);
        clip.file_path = Some("C:/samples/loop.wav".into());
        clip.time_mode = TimeMode::Musical;
        clip.warp_markers = vec![daw_core::WarpMarker::new(0.5, 1.0)];

        let map = TempoMap::from_bpm(120.0);
        let target = stretch_target_for(&clip, &map, 1.0).expect("alvo");

        assert_eq!(target.plan.segments.len(), 2, "{:?}", target.plan);

        let out = render_stretch(&buffer, &target).expect("render");

        let expected = target.frames_out(RATE) as f64;
        let got = out.left.len() as f64;

        assert!(
            (got - expected).abs() / expected < 0.05,
            "frames {got} != esperado {expected}"
        );

        // A emenda tem de ser continua: um degrau grande no meio seria
        // clique. Mede o maior salto entre amostras vizinhas na regiao da
        // emenda e compara com o salto tipico de uma senoide de 440 Hz.
        let join = (0.5 * RATE as f64 * 2.0) as usize;
        let lo = join.saturating_sub(200);
        let hi = (join + 200).min(out.left.len().saturating_sub(1));

        let mut max_jump = 0.0f32;
        for i in lo..hi {
            max_jump = max_jump.max((out.left[i + 1] - out.left[i]).abs());
        }

        assert!(
            max_jump < 0.8,
            "salto de {max_jump} na emenda (corte seco, seria clique)"
        );
    }
}
