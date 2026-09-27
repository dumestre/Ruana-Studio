//! Testes do desenho de notas MIDI dentro do clip.
//!
//! Existe por causa de um bug concreto: arrastar o "BEAT SEQUENCER" do
//! Clip Pool criava um clip MIDI com `notes: []` (o pattern nascia com
//! a grade toda desligada, `crates/daw-midi/src/pattern.rs:80`), e um
//! clip MIDI sem notas era desenhado exatamente como um retângulo
//! chapado — indistinguível de um placeholder. O motor de áudio ainda
//! descartava o clip inteiro (`daw-audio/src/engine.rs:205`).

use timeline_render::clips::{
    build_clip_instances,
    ClipDemo,
    ClipInteraction,
    ClipMetrics,
    ClipNote,
};
use timeline_render::srgb_to_linear;

fn midi_clip(len: f32, notes: Vec<ClipNote>) -> ClipDemo {
    ClipDemo {
        id: 1,
        track_index: 0,
        beat: 0.0,
        len,
        is_midi: true,
        color: 3,
        name: Some("BEAT SEQUENCER".into()),
        seam_beat: None,
        pre_glue_len: None,
        pre_glue_color: None,
        wave: Vec::new(),
        notes,
        file_path: None,
        source_start_beats: 0.0,
        source_length_beats: len,
        channels: 1,
    }
}

fn metrics() -> ClipMetrics {
    ClipMetrics {
        px_per_beat: 60.0,
        wave_segments: Vec::new(),
        row_height: 64.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
        ruler_px: 24.0,
        theme: 0,
        max_track: 1,
    }
}

#[test]
fn notas_midi_geram_mais_instancias_que_clip_vazio() {
    let inter = ClipInteraction::new();
    let m = metrics();

    let vazio = build_clip_instances(&[midi_clip(4.0, Vec::new())], &inter, &m, 900.0);
    let com_notas = build_clip_instances(
        &[midi_clip(
            4.0,
            vec![
                ClipNote { pitch: 36, start: 0.0, length: 0.25, velocity: 110 },
                ClipNote { pitch: 36, start: 1.0, length: 0.25, velocity: 110 },
                ClipNote { pitch: 42, start: 2.0, length: 0.25, velocity: 90 },
            ],
        )],
        &inter,
        &m,
        900.0,
    );

    // Uma instância por nota é o mínimo: o clip sem notas precisa
    // continuar desenhando o corpo, o header e o divisor.
    assert!(
        com_notas.len() >= vazio.len() + 3,
        "esperava ao menos 3 instâncias a mais, veio {} vs {}",
        com_notas.len(),
        vazio.len()
    );
}

#[test]
fn nota_usa_a_cor_do_clip_nao_a_da_onda() {
    let inter = ClipInteraction::new();
    let m = metrics();

    // color = 0 -> usa CLIP_MIDI_DEFAULT[theme] = CLIP_MIDI_DEFAULT[0]
    // = (0x6F, 0xA8, 0x6B) = (111, 168, 107) em sRGB.
    let clip = ClipDemo {
        id: 1,
        track_index: 0,
        beat: 0.0,
        len: 4.0,
        is_midi: true,
        color: 0,
        name: Some("BEAT SEQUENCER".into()),
        seam_beat: None,
        pre_glue_len: None,
        pre_glue_color: None,
        wave: Vec::new(),
        notes: vec![ClipNote { pitch: 36, start: 0.0, length: 1.0, velocity: 120 }],
        file_path: None,
        source_start_beats: 0.0,
        source_length_beats: 4.0,
        channels: 1,
    };

    let inst = build_clip_instances(&[clip], &inter, &m, 900.0);

    let barras: Vec<_> = inst.iter().filter(|r| r.radius == 1.0).collect();
    assert_eq!(barras.len(), 1);

    let c = barras[0].color;
    // CLIP_MIDI_DEFAULT[0] = (0x6F, 0xA8, 0x6B) = (111, 168, 107) em sRGB.
    // Pipeline converte para linear antes de ir pra GPU.
    let expected_r = (srgb_to_linear(111.0 / 255.0) * 255.0).round() as u8;
    let expected_g = (srgb_to_linear(168.0 / 255.0) * 255.0).round() as u8;
    let expected_b = (srgb_to_linear(107.0 / 255.0) * 255.0).round() as u8;
    // Tolerância de ±1 por diferenças de arredondamento FP.
    let (dr, dg, db) = (c[0] as i16 - expected_r as i16, c[1] as i16 - expected_g as i16, c[2] as i16 - expected_b as i16);
    assert!(
        dr.abs() <= 1 && dg.abs() <= 1 && db.abs() <= 1,
        "a nota deve usar a cor do clip em linear, veio {c:?}, esperado ~({expected_r}, {expected_g}, {expected_b})"
    );
    assert_eq!(c[3], 255, "a nota é opaca");
}

#[test]
fn nota_que_atravessa_a_borda_da_janela_e_cortada() {    let inter = ClipInteraction::new();

    // Janela de 100px: o clip tem 4 beats a 60px/beat = 240px, então
    // a última nota vive inteira fora da tela.
    let m = metrics();
    let notas = vec![
        ClipNote { pitch: 36, start: 0.0, length: 0.25, velocity: 100 },
        ClipNote { pitch: 60, start: 3.5, length: 0.25, velocity: 100 },
    ];
    let inst = build_clip_instances(&[midi_clip(4.0, notas)], &inter, &m, 100.0);

    // Só as barras de nota: `radius: 1.0` é delas (corpo, header e wave
    // usam 3.0). As outras instâncias já saem estouradas da janela por
    // outro motivo — o corpo usa a largura inteira do clip, não a
    // visível — e isso é anterior a este desenho.
    let barras: Vec<_> = inst.iter().filter(|r| r.radius == 1.0).collect();

    assert_eq!(barras.len(), 1, "a nota dentro da tela é a única que deve sobrar");

    for r in barras {
        assert!(
            r.x + r.w <= 100.0,
            "nota vazou para fora da janela: x={} w={}",
            r.x,
            r.w
        );
    }
}
