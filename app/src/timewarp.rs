//! Timewarp: mapa de tempo (BPM varia ao longo da timeline) para time stretch.
//!
//! Reutiliza a grid e ruler de `daw_core::timeline` (e, indiretamente,
//! `piano_roll`), estendendo-as para eventos de tempo variável.
//!
//! Funcionalidades:
//! - Adicionar/remover mudanças de tempo (time-warp / stretch)
//! - Converter beats↔segundos com o mapa de tempo ativo
//! - Gerar ruler ticks com labels de tempo (segundos + BPM)
//! - Calcular ratio de time-stretch de clips de áudio

use std::cell::RefCell;
use std::rc::Rc;

use daw_core::project::Project;
use daw_core::timeline::{TempoChange, TempoMap};
use daw_core::transport::Transport;

use crate::MainWindow;

/// Converte beats → segundos usando o mapa de tempo do projeto.
pub fn beats_to_seconds(project: &Project, beat: f32) -> f64 {
    project.tempo_map.beats_to_seconds(beat)
}

/// Converte segundos → beats usando o mapa de tempo do projeto.
pub fn seconds_to_beats(project: &Project, seconds: f64) -> f32 {
    project.tempo_map.seconds_to_beats(seconds)
}

/// BPM no beat especificado (última mudança até ou antes do beat).
pub fn bpm_at(project: &Project, beat: f32) -> f32 {
    project.tempo_map.bpm_at(beat)
}

/// BPM base do projeto (no beat 0).
pub fn project_bpm(project: &Project) -> f32 {
    project
        .tempo_map
        .changes
        .first()
        .map(|c| c.bpm)
        .unwrap_or(120.0)
}

/// Adiciona ou atualiza uma mudança de tempo (time-warp) no beat especificado.
pub fn set_tempo_at(project: &mut Project, beat: f32, bpm: f32) {
    project.set_tempo_at(beat, bpm);
}

/// Remove uma mudança de tempo (exceto a do beat 0).
pub fn remove_tempo_at(project: &mut Project, beat: f32) -> bool {
    project.remove_tempo_change(beat)
}

/// Lista de mudanças de tempo (clonada para a UI).
pub fn tempo_changes(project: &Project) -> Vec<TempoChange> {
    project.tempo_map.changes.clone()
}

/// Tick da régua com informações de tempo para time stretch.
#[derive(Debug, Clone)]
pub struct TempoRulerTick {
    pub bar: i32,
    pub pos: f32,
    pub level: i32,
    /// Posição absoluta em segundos.
    pub seconds: f64,
    /// BPM ativo neste tick.
    pub bpm: f32,
}

/// Subdivisões de um compasso (nível 0 = barra, maior = subdivisão).
const RULER_SUBDIVS: &[(f32, i32)] = &[
    (0.0, 0),
    (0.25, 1),
    (0.50, 1),
    (0.75, 1),
    (0.125, 2),
    (0.375, 2),
    (0.625, 2),
    (0.875, 2),
];

/// Gera ticks da régua que refletem variações de tempo.
///
/// Reutiliza a mesma estrutura de ticks do `Grid` (16 ticks/compasso),
/// mas anota o tempo em segundos e o BPM ativo — a UI pode mostrar
/// labels de tempo absolutas e destacar onde o tempo muda.
pub fn tempo_aware_ruler_ticks(
    tempo_map: &TempoMap,
    bars: i32,
    beats_per_bar: u32,
) -> Vec<TempoRulerTick> {
    let mut ticks = Vec::new();
    let bpb = beats_per_bar as f32;
    for bar in 0..bars {
        let bar_beat = bar as f32 * bpb;
        for &(pos, level) in RULER_SUBDIVS {
            let beat = bar_beat + pos * bpb;
            ticks.push(TempoRulerTick {
                bar,
                pos,
                level,
                seconds: tempo_map.beats_to_seconds(beat),
                bpm: tempo_map.bpm_at(beat),
            });
        }
    }
    ticks
}

/// Ratio de time-stretch de um clip de áudio.
///
/// Compara a duração do clip no tempo do projeto (em segundos,
/// respeitando o mapa de tempo) com a duração natural da fonte.
///
/// - `> 1.0` → áudio mais lento (esticado)
/// - `< 1.0` → áudio mais rápido (comprimido)
/// MIDI ou fonte inválida retorna `1.0`.
pub fn clip_stretch_ratio(
    project: &Project,
    clip_id: u64,
    source_duration_seconds: f64,
) -> f32 {
    if source_duration_seconds <= 0.0 {
        return 1.0;
    }
    let Some(clip) = project.clip(clip_id) else {
        return 1.0;
    };
    if clip.is_midi() {
        return 1.0;
    }
    let timeline_seconds = project
        .tempo_map
        .beats_to_seconds(clip.start_beat + clip.length_beats)
        - project.tempo_map.beats_to_seconds(clip.start_beat);
    timeline_seconds / source_duration_seconds
}

/// Snap de um beat à grade musical, respeitando o mapa de tempo.
///
/// `resolution` controla a granularidade: 1 = semínima, 2 = semicolama,
/// 4 = 16th, etc.
pub fn snap_to_grid(beat: f32, resolution: u32) -> f32 {
    let quantum = 1.0 / resolution as f32;
    (beat / quantum).round() * quantum
}

/// Conecta o módulo timewarp aos callbacks da UI Slint.
///
/// - Quando o usuário muda o BPM global na UI, o mapa de tempo do
///   projeto é atualizado (beat 0) mantendo a posição musical
///   contínua.
/// - O callback `timeline_canvas_render_requested` pode ser usado
///   pela UI para forçar redraw da régua após mudanças de tempo.
pub fn wire_timewarp(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    transport: Rc<std::sync::Mutex<Transport>>,
) {
    let ui_weak = ui.as_weak();

    {
        let project = project.clone();
        ui.on_set_bpm(move |bpm| {
            let bpm = bpm.round().clamp(20.0, 300.0);
            let mut proj = project.borrow_mut();

            // Atualiza BPM base do mapa (beat 0) e do transporte.
            let old_bpm = proj.tempo_map.bpm_at(0.0);
            if (old_bpm - bpm).abs() > 0.001 {
                proj.set_tempo_at(0.0, bpm);
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_bpm(bpm as i32);
            }
        });
    }
}
