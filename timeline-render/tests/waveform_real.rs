//! Testes da waveform REAL (arquivo -> colunas L/R do buffer da timeline).
//!
//! Cobrem tres defeitos que faziam a onda desenhada nao ser a do arquivo:
//!
//! 1. `WaveformData::samples` e o DOWNMIX MONO; o layout intercalado vive
//!    em `interleaved`. Ler o downmix como se fosse intercalado cortava a
//!    duracao pela metade e lia amostras alternadas - a segunda metade do
//!    arquivo simplesmente nao aparecia.
//! 2. O buffer tem SEMPRE 2 floats por coluna (L/R). Arquivo mono emitia
//!    1 float por coluna e os consumidores liam par/impar: cada faixa
//!    mostrava metade das colunas (onda decimada).
//! 3. `clip.channels` era um palpite do chamador (sempre 2 para audio), o
//!    que desenhava duas faixas L/R ate para arquivo mono.

use std::path::PathBuf;

use timeline_render::clips::ClipDemo;
use timeline_render::waveform::build_wave_bars_and_segments;

/// px-per-beat de referencia (o mesmo `WAVE_PPB` da engine).
const PPB: f32 = 40.0;
const BPM: f32 = 120.0;
const RATE: u32 = 8_000;
/// 1 segundo em 16-bit PCM.
const FRAMES: usize = RATE as usize;
/// 1 segundo = 2 beats a 120 BPM.
const LEN_BEATS: f32 = 2.0;

fn scratch_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("waveform-tests");

    std::fs::create_dir_all(&dir).expect("criar diretorio de teste");

    dir
}

/// Escreve um WAV PCM 16-bit intercalado (sem dependencias externas).
fn write_wav(
    name: &str,
    channels: u16,
    frames: [&[f32]; 2],
) -> PathBuf {
    let path = scratch_dir().join(name);

    let n_frames = frames[0].len();
    let data_len = (n_frames * channels as usize * 2) as u32;

    let mut bytes: Vec<u8> =
        Vec::with_capacity(44 + data_len as usize);

    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");

    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // tamanho do bloco fmt
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * channels as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits por amostra

    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());

    for frame in 0..n_frames {
        for ch in 0..channels as usize {
            let v = frames[ch][frame];
            let q = (v.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            bytes.extend_from_slice(&q.to_le_bytes());
        }
    }

    std::fs::write(&path, &bytes).expect("escrever wav");

    path
}

fn clip_for(path: &PathBuf, len_beats: f32) -> ClipDemo {
    ClipDemo {
        id: 1,
        track_index: 0,
        beat: 0.0,
        len: len_beats,
        is_midi: false,
        color: 0,
        name: Some("teste".into()),
        seam_beat: None,
        pre_glue_len: None,
        pre_glue_color: None,
        wave: Vec::new(),
        notes: Vec::new(),
        file_path: Some(path.to_string_lossy().to_string()),
        source_start_beats: 0.0,
        source_length_beats: 0.0,
        // Palpite do chamador, como faz o espelho do app.
        channels: 2,
        region_seconds: 0.0,
        warp_markers: Vec::new(),
    }
}

fn max_abs(values: &[f32]) -> f32 {
    values
        .iter()
        .fold(0.0f32, |acc, v| acc.max(v.abs()))
}

#[test]
fn arquivo_estereo_cobre_o_arquivo_inteiro() {
    // Primeira metade MUDA, segunda metade em +-1. A leitura do downmix
    // mono como intercalado cortava a duracao pela metade e a onda
    // resultante era so a metade silenciosa (nivel 0).
    let n = FRAMES;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];

    for i in 0..n {
        if i >= n / 2 {
            let v = if i % 2 == 0 { 1.0 } else { -1.0 };
            l[i] = v;
            r[i] = -v;
        }
    }

    let path = write_wav("estereo_metade.wav", 2, [&l, &r]);

    let mut clips = vec![clip_for(&path, LEN_BEATS)];
    let (bars, segments) = build_wave_bars_and_segments(&mut clips, PPB, BPM);

    let clip = &clips[0];
    let cols = segments[0].1 as usize;

    assert_eq!(clip.wave.len(), cols * 2, "2 floats por coluna (L/R)");
    assert_eq!(bars.len(), cols * 2, "buffer com 2 floats por coluna");
    assert_eq!(clip.channels, 2, "numero real de canais do arquivo");

    // A segunda metade (que sumia) tem de aparecer.
    let level = max_abs(&clip.wave);
    assert!(
        level > 0.9,
        "a metade final do arquivo nao foi desenhada (nivel {level})"
    );

    // E o comeco continua mudo: a onda cobre o arquivo TODO, na ordem.
    let first = clip.wave[0].abs();
    assert!(
        first < 0.05,
        "a coluna inicial devia ser silencio, veio {first}"
    );

    // A ultima coluna e a regiao alta.
    let last = clip.wave[clip.wave.len() - 2].abs();
    assert!(last > 0.9, "a coluna final devia estar no trecho alto");
}

#[test]
fn cada_canal_tem_o_seu_conteudo() {
    // L constante +0.5, R constante -0.25. O downmix mono e
    // (0.5 + -0.25)/2 = 0.125: se os canais viessem do downmix, os dois
    // valeriam 0.125.
    let l = vec![0.5f32; FRAMES];
    let r = vec![-0.25f32; FRAMES];
    let path = write_wav("estereo_canais.wav", 2, [&l, &r]);

    let mut clips = vec![clip_for(&path, LEN_BEATS)];
    let (_, segments) = build_wave_bars_and_segments(&mut clips, PPB, BPM);
    let cols = segments[0].1 as usize;
    let wave = &clips[0].wave;

    for col in 0..cols {
        let left = wave[col * 2];
        let right = wave[col * 2 + 1];

        assert!(
            (left - 0.5).abs() < 0.01,
            "coluna {col}: L devia ser 0.5, veio {left}"
        );
        assert!(
            (right + 0.25).abs() < 0.01,
            "coluna {col}: R devia ser -0.25, veio {right}"
        );
    }
}

#[test]
fn arquivo_mono_mantem_par_lr_e_um_canal() {
    let m = vec![0.4f32; FRAMES];
    let path = write_wav("mono.wav", 1, [&m, &m]);

    // O chamador PALPITA 2 canais (como o espelho do app fazia).
    let mut clips = vec![clip_for(&path, LEN_BEATS)];
    let (_, segments) = build_wave_bars_and_segments(&mut clips, PPB, BPM);

    let clip = &clips[0];
    let cols = segments[0].1 as usize;

    assert_eq!(
        clip.wave.len(),
        cols * 2,
        "mono tambem ocupa o par L/R (senao cada faixa le metade das colunas)"
    );

    assert_eq!(clip.channels, 1, "mono tem de ser desenhado como mono");

    for col in 0..cols {
        let left = clip.wave[col * 2];
        let right = clip.wave[col * 2 + 1];

        assert!(
            (left - 0.4).abs() < 0.01,
            "coluna {col}: L devia ser 0.4, veio {left}"
        );
        assert!(
            (right - left).abs() < 1e-6,
            "mono deve repetir o valor em L e R"
        );
    }
}

#[test]
fn nivel_do_envelope_nao_cai_com_a_resolucao() {
    // Sinal sempre em +-1: o envelope (pico por coluna) tem de ficar no
    // fundo de escala em QUALQUER resolucao. Era isto que nao podia
    // variar com o zoom - a resolucao nao pode mudar o NIVEL desenhado.
    let n = FRAMES;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];

    for i in 0..n {
        let v = if i % 2 == 0 { 1.0 } else { -1.0 };
        l[i] = v;
        r[i] = -v;
    }

    let path = write_wav("estereo_nivel.wav", 2, [&l, &r]);

    for ppb in [8.0f32, 40.0, 160.0, 640.0] {
        let mut clips = vec![clip_for(&path, LEN_BEATS)];
        let (_, segments) =
            build_wave_bars_and_segments(&mut clips, ppb, BPM);

        assert_eq!(segments[0].1 as usize * 2, clips[0].wave.len());

        let level = max_abs(&clips[0].wave);
        assert!(
            level > 0.99,
            "ppb {ppb}: nivel do envelope caiu para {level}"
        );
    }
}
#[test]
fn regiao_musical_vai_do_trim_ate_o_fim_do_arquivo() {
    // Clip MUSICAL: `source_length_beats = 0` com offset > 0 pede a fonte
    // INTEIRA a partir do trim (e o que sera esticado para o span).
    //
    // O arquivo tem a primeira metade muda e depois um trecho em +-1 que
    // comeca EM 5000 (1000 frames depois da metade). O offset do clip e 1
    // beat = 4000 frames. Sem a regra "ate o fim", a regiao degenerava para
    // 1 frame e a unica amostra lida era a do frame 4000 - silencio, ou
    // seja a onda sumia.
    let n = FRAMES;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];

    for i in 0..n {
        if i >= n / 2 + 1000 {
            let v = if i % 2 == 0 { 1.0 } else { -1.0 };
            l[i] = v;
            r[i] = -v;
        }
    }

    let path = write_wav("musical_ate_o_fim.wav", 2, [&l, &r]);

    let mut clip = clip_for(&path, LEN_BEATS);
    // 1 beat a 120 BPM = 0.5 s = 4000 frames.
    clip.source_start_beats = 1.0;
    clip.source_length_beats = 0.0;

    let mut clips = vec![clip];
    let (_, segments) = build_wave_bars_and_segments(&mut clips, PPB, BPM);

    let cols = segments[0].1 as usize;
    let wave = &clips[0].wave;

    assert_eq!(wave.len(), cols * 2, "2 floats por coluna (L/R)");

    let level = max_abs(wave);
    assert!(
        level > 0.9,
        "a onda do clip musical nao apareceu (nivel {level})"
    );

    let first = wave[0].abs();
    assert!(
        first < 0.05 || cols < 8,
        "a coluna inicial deveria ser o silencio logo apos o trim, veio {first}"
    );

    let last = wave[wave.len() - 2].abs();
    assert!(last > 0.9, "a coluna final deveria estar no trecho que soa");
}