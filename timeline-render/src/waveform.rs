//! Waveforms procedurais de teste (placeholder com aparência real).
//!
//! Gera MIN/MAX por coluna — o "osciloscópio" da timeline.
//! O shader liga as amostras com segmentos de reta.
//!
//! O buffer usa 2 floats por coluna:
//!   [L, R]
//!
//! Para waveform procedural, L e R recebem o mesmo valor.
//! Para áudio real, cada canal é calculado separadamente.

use crate::clips::ClipDemo;
use crate::fades::{
    eval_bezier_2d_in,
    eval_bezier_2d_out,
    ClipFade,
};
use daw_media::waveform::decode_audio;
use std::path::Path;

/// Número de colunas de waveform de um clip de áudio.
///
/// Deve bater com o mesmo cálculo usado na geração.
pub fn wave_cols(
    len: f32,
    px_per_beat: f32,
) -> u32 {
    (
        (
            len *
            px_per_beat *
            4.0
        )
        .ceil()
        as u32
    )
    .max(1)
    .min(16384)
}

/// Hash determinístico 64-bit, sem dependências.
fn hash01(mut x: u64) -> f32 {
    x = x.wrapping_mul(
        0x9E3779B97F4A7C15
    );

    x = (
        x ^
        (x >> 30)
    )
    .wrapping_mul(
        0xBF58476D1CE4E5B9
    );

    x ^= x >> 27;

    x = x.wrapping_mul(
        0x94D049BB133111EB
    );

    x ^= x >> 31;

    (x >> 33) as f32 /
    (1u64 << 31) as f32
}

/// Envelope de ganho do clip.
///
/// Nunca zera para evitar que a linha fique completamente morta
/// nas bordas e nas quedas.
fn gain(frac: f32) -> f32 {
    let fade_in =
        (frac / 0.06)
        .min(1.0);

    let fade_out =
        ((1.0 - frac) / 0.08)
        .min(1.0);

    0.35 +
    0.65 *
    fade_in *
    fade_out
}

/// Baixo:
/// duas oscilações lentas superpostas.
fn sample_bass(
    i: u32,
    n: u32,
    seed: u64,
) -> f32 {
    let frac =
        i as f32 /
        n as f32;

    let t =
        frac *
        std::f32::consts::TAU;

    let phase =
        hash01(seed);

    (
        (t * 2.0 + phase).sin() *
        0.85
    ) +
    (
        (t * 4.3 + phase * 2.0).sin() *
        0.15
    )
}

/// Bateria:
/// onda oscilante modulada em amplitude.
fn sample_drums(
    i: u32,
    n: u32,
    seed: u64,
) -> f32 {
    let frac =
        i as f32 /
        n as f32;

    let t =
        frac *
        std::f32::consts::TAU;

    let phase =
        hash01(
            seed.wrapping_mul(3)
        );

    let pulse =
        0.5 +
        0.5 *
        (
            t * 6.0 +
            phase
        )
        .sin();

    (
        t * 3.0 +
        phase
    )
    .sin()
    *
    (
        0.35 +
        0.65 * pulse
    )
}

/// Pad:
/// oscilação lenta com leve variação.
fn sample_pads(
    i: u32,
    n: u32,
    seed: u64,
) -> f32 {
    let frac =
        i as f32 /
        n as f32;

    let t =
        frac *
        std::f32::consts::TAU;

    let phase =
        hash01(
            seed.wrapping_add(9)
        );

    (
        t * 1.5 +
        phase
    )
    .sin()
    *
    0.7
    +
    (
        t * 4.0 +
        phase * 3.0
    )
    .sin()
    *
    0.3
}

/// Estilo da waveform por índice de clip de áudio.
fn sample_wave(
    style: usize,
    i: u32,
    n: u32,
    seed: u64,
) -> f32 {
    match style % 3 {
        0 => sample_bass(
            i,
            n,
            seed,
        ),

        1 => sample_drums(
            i,
            n,
            seed,
        ),

        _ => sample_pads(
            i,
            n,
            seed,
        ),
    }
}

/// Constrói todas as amplitudes L/R dos clips de áudio.
///
/// O buffer final contém:
///
/// [L0, R0, L1, R1, L2, R2, ...]
///
/// Os offsets são armazenados em unidades de COLUNA,
/// e não em unidades de float.
pub fn build_wave_bars_and_segments(
    clips: &mut [ClipDemo],
    px_per_beat: f32,
    bpm: f32,
) -> (
    Vec<f32>,
    Vec<(u32, u32)>
) {
    let mut bars =
        Vec::new();

    let mut segments =
        Vec::with_capacity(
            clips.len()
        );

    let mut audio_idx =
        0u64;

    for clip in
        clips.iter_mut()
    {
        // MIDI não possui waveform.
        if clip.is_midi {
            segments.push(
                (0u32, 0u32)
            );

            clip.wave =
                Vec::new();

            continue;
        }

        let n_cols =
            wave_cols(
                clip.len,
                px_per_beat,
            );

        // Nº real de canais lido do arquivo (quando há decode real).
        let mut file_channels: Option<u16> =
            None;

        let wave: Vec<f32> =
            if let Some(ref path) =
                clip.file_path
            {
                match decode_real_wave(
                    path,
                    n_cols,
                    clip.source_start_beats,
                    clip.source_length_beats,
                    bpm,
                ) {
                    Ok((
                        w,
                        channels,
                    )) => {
                        file_channels =
                            Some(channels);

                        w
                    }

                    Err(err) => {
                        warn_procedural_fallback(
                            path,
                            &*err,
                        );

                        generate_procedural_wave(
                            clip,
                            n_cols,
                            audio_idx,
                        )
                    }
                }
            } else {
                let style =
                    (audio_idx % 3)
                    as usize;

                let seed =
                    audio_idx
                    .wrapping_mul(
                        0x9E3779B97F4A7C15
                    )
                    ^
                    (clip.beat as u32 as u64)
                        .wrapping_mul(11)
                    ^
                    (clip.track_index as u64)
                        .wrapping_mul(
                            0xD1B54A32D192ED03
                        );

                let mut w =
                    Vec::with_capacity(
                        n_cols as usize * 2
                    );

                for col in
                    0..n_cols
                {
                    let frac =
                        col as f32 /
                        n_cols as f32;

                    let v =
                        sample_wave(
                            style,
                            col,
                            n_cols,
                            seed,
                        )
                        *
                        gain(frac);

                    // L
                    w.push(v);

                    // R
                    w.push(v);
                }

                w
            };

        // Offset em COLUNAS.
        let offset_cols =
            bars.len() as u32 / 2;

        bars.extend_from_slice(
            &wave
        );

        segments.push(
            (
                offset_cols,
                n_cols,
            )
        );

        // O nº de canais REAL do arquivo substitui o palpite do
        // chamador: o espelho do app marcava TODO clip de áudio como
        // estéreo, o que desenhava DUAS faixas (L/R) para um arquivo
        // mono — cada faixa lendo metade das colunas (par/ímpar), ou
        // seja, uma versão decimada da onda do arquivo.
        if let Some(channels) =
            file_channels
        {
            clip.channels =
                channels;
        }

        // A wave armazenada no clip é a fonte da verdade.
        clip.wave =
            wave;

        audio_idx += 1;
    }

    (
        bars,
        segments
    )
}

/// Decodifica um arquivo de áudio real.
///
/// Mono:
///   1 float por coluna.
///
/// Stereo:
///   2 floats por coluna: L/R.
///
/// O valor usado é a média com sinal do bloco correspondente.
pub(crate) fn decode_real_wave(
    path: &str,
    n_cols: u32,
    source_start_beats: f32,
    source_length_beats: f32,
    bpm: f32,
) -> Result<
    (
        Vec<f32>,
        u16,
    ),
    Box<dyn std::error::Error>,
> {
    let data =
        decode_audio(
            Path::new(path)
        )?;

    // ================================================================
    // LAYOUT DAS AMOSTRAS
    //
    // `WaveformData::samples` é o DOWNMIX MONO do arquivo; o layout
    // ORIGINAL intercalado (com `channels` canais) vive em
    // `interleaved` (daw-media/src/waveform.rs:32-58).
    //
    // Ler o downmix como se fosse intercalado cortava a duração pela
    // METADE (`total_frames = len / channels`) e fazia cada canal ler
    // amostras alternadas do mono: a onda desenhada não era a do
    // arquivo — ficava mais curta e decimada em relação à original.
    // ================================================================

    let samples =
        &data.interleaved;

    if samples.is_empty() ||
       n_cols == 0
    {
        // SEMPRE 2 floats por coluna (L/R): é o layout que
        // `concat_wave_data`, `apply_fade_envelope` e o par lido pelo
        // shader assumem em todo o resto do pipeline.
        return Ok((
            vec![
                0.0;
                n_cols as usize * 2
            ],
            data.channels.max(1),
        ));
    }

    let channels =
        data.channels
            .max(1)
            as usize;

    let mut wave =
        Vec::with_capacity(
            n_cols as usize * 2
        );

    let total_frames =
        samples.len() /
        channels;

    let cols =
        n_cols as usize;

    // ==================================================================
    // REGIÃO AUDÍVEL do arquivo
    //
    // Clips importados passam por `trim_silence` (importer.rs): a wave do
    // clip NÃO é o arquivo inteiro — começa em `source_start_beats` e dura
    // `source_length_beats`. Se a décode usasse o arquivo inteiro, o silêncio
    // do topo apareceria à esquerda do clip e o áudio (que pula esse silêncio)
    // soaria à frente da waveform. Aqui mapeamos as colunas para a MESMA
    // região temporal que o motor de áudio lê (`source_start*fpb + offset`).
    //
    // `source_start_beats == 0` e `source_length_beats == 0` (clips criados
    // sem import, ex.: `add_clip_with_path`) = sem trim → arquivo inteiro.
    // ==================================================================
    let frames_per_beat =
        data.sample_rate.max(1) as f64 *
        60.0 /
        bpm.max(1.0) as f64;

    let (start_frame, region_len) =
        if source_start_beats > 0.0
           || source_length_beats > 0.0
        {
            let start = (
                source_start_beats as f64 *
                frames_per_beat
            )
            .round() as i64;

            let len = (
                source_length_beats
                    .max(0.0) as f64 *
                frames_per_beat
            )
            .round() as i64;

            let start =
                start.clamp(
                    0,
                    total_frames as i64 - 1,
                );

            let region =
                len.max(1).min(
                    total_frames as i64
                    - start,
                );

            if region >= 1 {
                (start as usize, region as usize)
            } else {
                (0, total_frames)
            }
        } else {
            (0, total_frames)
        };

    let region_end =
        start_frame +
        region_len;

    for col in
        0..cols
    {
        let frame_start =
            (
                start_frame +
                (
                    col *
                    region_len /
                    cols
                )
            )
            .min(
                total_frames
                    .saturating_sub(1)
            );

        let frame_end =
            (
                start_frame +
                (
                    (col + 1) *
                    region_len /
                    cols
                )
            )
            .min(
                region_end
            );

        if frame_start >=
           frame_end
        {
            // L + R (2 floats por coluna).
            for _ in
                0..2
            {
                wave.push(0.0);
            }

            continue;
        }

        // ================================================================
        // MONO
        // ================================================================

        if channels == 1 {
            let start_sample =
                frame_start;

            let end_sample =
                frame_end;

            let peak =
                samples[
                    start_sample
                    ..
                    end_sample
                ]
                .iter()
                .copied()
                .fold(0.0f32, |acc, s| {
                    if s.abs() > acc.abs() {
                        s
                    } else {
                        acc
                    }
                });

            // Mono ocupa o MESMO par L/R da coluna. Sem repetir o
            // valor o buffer ficaria com 1 float por coluna e todos os
            // consumidores leriam par/ímpar — cada faixa passaria a
            // mostrar metade das colunas, isto é, uma versão decimada
            // da onda do arquivo.
            let v =
                if peak.is_finite() {
                    peak
                } else {
                    0.0
                };

            wave.push(v);
            wave.push(v);

            continue;
        }

        // ================================================================
        // STEREO / MULTICANAL
        //
        // O buffer tem SEMPRE 2 floats por coluna (L/R). Arquivos com mais
        // de 2 canais contribuem apenas com os dois primeiros — o `step`
        // continua sendo o número REAL de canais, senão os canais seriam
        // lidos fora de fase.
        // ================================================================

        for ch in
            0..channels.min(2)
        {
            let start_sample =
                frame_start *
                channels +
                ch;

            let end_sample =
                frame_end *
                channels +
                ch;

            let step =
                channels;

            let mut peak =
                0.0f32;

            let mut i =
                start_sample;

            while i <
                  end_sample &&
                  i <
                  samples.len()
            {
                if samples[i].abs() >
                   peak.abs()
                {
                    peak =
                        samples[i];
                }

                i +=
                    step;
            }

            wave.push(
                if peak.is_finite() {
                    peak
                } else {
                    0.0
                }
            );
        }
    }

    // Devolve também o nº REAL de canais do arquivo: é o que decide
    // entre a faixa única (mono, `clip_w = -6`) e as duas faixas L/R no
    // renderer (`clips/render.rs`), e o que garante que o par L/R lido
    // pelo shader corresponde ao conteúdo.
    Ok((
        wave,
        channels as u16,
    ))
}

/// Avisa UMA vez por arquivo que a waveform real não pôde ser lida e o
/// desenho caiu no gerador procedural.
///
/// O fallback era silencioso: um clip com `file_path` presente porém
/// inválido (arquivo movido/renomeado, extensão não suportada) ficava com
/// a mesma aparência de placeholder de um clip sem áudio nenhum, sem
/// nenhuma pista no console.
fn warn_procedural_fallback(
    path: &str,
    err: &dyn std::error::Error,
) {
    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local! {
        static WARNED:
            RefCell<HashSet<String>> =
            RefCell::new(HashSet::new());
    }

    WARNED.with(|warned| {
        if warned
            .borrow_mut()
            .insert(path.to_string())
        {
            eprintln!(
                "[waveform] wave real indisponivel para {path}: {err} \
                 — desenhando placeholder procedural"
            );
        }
    });
}

/// Gera waveform procedural para um clip.
///
/// Retorna:
///
/// [L0, R0, L1, R1, ...]
pub(crate) fn generate_procedural_wave(
    clip: &ClipDemo,
    n_cols: u32,
    seed: u64,
) -> Vec<f32> {
    let style =
        (seed % 3) as usize;

    let seed =
        seed.wrapping_mul(
            0x9E3779B97F4A7C15
        )
        ^
        (clip.beat as u32 as u64)
            .wrapping_mul(11)
        ^
        (clip.track_index as u64)
            .wrapping_mul(
                0xD1B54A32D192ED03
            );

    let mut wave =
        Vec::with_capacity(
            n_cols as usize * 2
        );

    for col in
        0..n_cols
    {
        let frac =
            col as f32 /
            n_cols as f32;

        let v =
            sample_wave(
                style,
                col,
                n_cols,
                seed,
            )
            *
            gain(frac);

        // L
        wave.push(v);

        // R
        wave.push(v);
    }

    wave
}

/// Concatena as ondas armazenadas nos clips.
///
/// Não regenera as waves.
///
/// Retorna:
///   (buffer, segments)
///
/// `segments` usa:
///   (offset_em_colunas, quantidade_de_colunas)
pub fn concat_wave_data(
    clips: &[ClipDemo],
) -> (
    Vec<f32>,
    Vec<(u32, u32)>
) {
    let mut bars =
        Vec::new();

    let mut segments =
        Vec::with_capacity(
            clips.len()
        );

    let mut col_offset =
        0u32;

    for clip in
        clips
    {
        if clip.is_midi ||
           clip.wave.is_empty()
        {
            segments.push(
                (0u32, 0u32)
            );

            continue;
        }

        let n_cols =
            (clip.wave.len() / 2)
                as u32;

        segments.push(
            (
                col_offset,
                n_cols,
            )
        );

        bars.extend_from_slice(
            &clip.wave
        );

        col_offset +=
            n_cols;
    }

    (
        bars,
        segments
    )
}

/// Aplica envelope de fade in/out e ganho às amostras.
///
/// Cada coluna ocupa:
///   L, R
///
/// O envelope é aplicado aos dois canais.
pub fn apply_fade_envelope(
    bars: Vec<f32>,
    segments: &[(u32, u32)],
    clips: &[ClipDemo],
    fades: &[ClipFade],
) -> Vec<f32> {
    let mut out =
        bars;

    for (i, clip) in
        clips.iter().enumerate()
    {
        if clip.is_midi ||
           clip.wave.is_empty()
        {
            continue;
        }

        if i >=
           fades.len()
        {
            continue;
        }

        let fade =
            &fades[i];

        let (
            offset_cols,
            count_cols,
        ) =
            segments[i];

        let n_cols =
            count_cols as usize;

        if n_cols == 0 {
            continue;
        }

        // 2 floats por coluna.
        let clip_len =
            clip.wave.len()
                as f32 /
            160.0;

        if clip_len <= 0.0 {
            continue;
        }

        let gain_linear =
            fade.gain_linear();

        for j in
            0..n_cols
        {
            let frac =
                if n_cols > 1 {
                    j as f32 /
                    (n_cols - 1)
                        as f32
                } else {
                    0.0
                };

            let beat_pos =
                frac *
                clip_len;

            // ============================================================
            // FADE IN
            // ============================================================

            let env_in =
                if fade.in_len > 0.0 &&
                   beat_pos < fade.in_len
                {
                    let t =
                        (
                            beat_pos /
                            fade.in_len
                        )
                        .clamp(
                            0.0,
                            1.0
                        );

                    eval_bezier_2d_in(
                        t,
                        fade.in_curve_x,
                        fade.in_curve_y,
                    )
                } else {
                    1.0
                };

            // ============================================================
            // FADE OUT
            // ============================================================

            let env_out =
                if fade.out_len > 0.0 &&
                   beat_pos >
                       clip_len -
                       fade.out_len
                {
                    let t =
                        (
                            (
                                beat_pos -
                                (
                                    clip_len -
                                    fade.out_len
                                )
                            )
                            /
                            fade.out_len
                        )
                        .clamp(
                            0.0,
                            1.0
                        );

                    eval_bezier_2d_out(
                        t,
                        fade.out_curve_x,
                        fade.out_curve_y,
                    )
                } else {
                    1.0
                };

            let envelope =
                env_in *
                env_out *
                gain_linear;

            // L/R
            let idx =
                (
                    offset_cols
                    as usize +
                    j
                )
                * 2;

            if idx + 1 <
               out.len()
            {
                out[idx] *=
                    envelope;

                out[idx + 1] *=
                    envelope;
            }
        }
    }

    out
}