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
fn nota_usa_a_cor_escura_da_waveform() {
    let inter = ClipInteraction::new();
    let m = metrics();

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
    assert_eq!(
        barras[0].color,
        [5, 5, 5, 255],
        "a nota usa a mesma cor escura da waveform"
    );
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

#[test]
fn selecao_de_clip_usa_contorno_e_sombra_compativeis_com_o_tema() {
    let mut inter = ClipInteraction::new();
    inter.selected = vec![true];
    let clip = midi_clip(4.0, Vec::new());

    let mut paper = metrics();
    paper.theme = 6;
    let paper_instances = build_clip_instances(&[clip.clone()], &inter, &paper, 900.0);
    assert!(paper_instances.iter().any(|rect| rect.color == [0x26, 0x26, 0x26, 255]));
    assert!(!paper_instances.iter().any(|rect| rect.color == [0x00, 0xC8, 0x53, 88]));

    let mut dark = metrics();
    dark.theme = 0;
    let dark_instances = build_clip_instances(&[clip], &inter, &dark, 900.0);
    assert!(dark_instances.iter().any(|rect| rect.color == [0x00, 0xC8, 0x53, 255]));
    assert_eq!(
        dark_instances
            .iter()
            .filter(|rect| rect.color == [0x00, 0xC8, 0x53, 88])
            .count(),
        4,
        "temas escuros usam sombra externa com a cor accent"
    );
}
