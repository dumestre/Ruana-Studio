//! Projeção inicial do core nos modelos Slint e sincronização
//! granular via eventos do documento.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ClipData;
use crate::MainWindow;
use crate::explorer::load_directory_entries;
use crate::mixer::mixer_inserts_state;
use crate::waveform::{WaveformCache, to_clip_data};
use daw_core::clip::Clip;
use daw_core::project::{ChangeKind, DocumentEvent, Project};
use daw_media::filesystem::default_samples_dir;
use slint::Model;
use std::path::PathBuf;

/// Compassos necessários para caber até `end_beat`: teto em
/// compassos + 8 de folga, mínimo 32. Cresce monotonicamente
/// (undo não encolhe a timeline — comportamento padrão de DAW).
pub fn bars_for_end_beat(end_beat: f64, beats_per_bar: i32) -> i32 {
    ((end_beat / beats_per_bar as f64).ceil() as i32 + 8).max(32)
}

/// Fim (em batidas) relevante do evento, para crescer total-bars.
/// Considera old E new — cobrir undo/redo sem ramificar.
fn event_max_end_beat(event: &DocumentEvent) -> Option<f64> {
    let end = |c: &Clip| c.end_beat() as f64;
    let both = |a: &Clip, b: &Clip| end(a).max(end(b));
    match event {
        DocumentEvent::ClipAdded(c) => Some(end(c)),
        DocumentEvent::ClipSliced { left, right, .. } => Some(both(left, right)),
        DocumentEvent::ClipMoved { old, new } => Some(both(old, new)),
        DocumentEvent::ClipsGlued {
            left_before, right, ..
        } => Some(both(left_before, right)),
        DocumentEvent::ClipResized { old, new } => Some(both(old, new)),
        _ => None,
    }
}

/// Preenche os modelos da UI a partir do estado inicial do core.
///
/// Retorna os `VecModel` compartilhados para que o assinante de
/// eventos possa atualizá-los granularmente em runtime.
pub fn initial_bind(
    ui: &MainWindow,
    project: &Project,
    waveforms: &WaveformCache,
) -> (
    Rc<slint::VecModel<slint::SharedString>>,
    Rc<slint::VecModel<ClipData>>,
    Rc<slint::VecModel<i32>>,
) {
    let track_names: Vec<slint::SharedString> = project
        .tracks
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    let tracks_model = Rc::new(slint::VecModel::from(track_names));
    ui.set_track_names(tracks_model.clone().into());

    // Cores das tracks (índice na paleta da UI).
    let track_colors: Vec<i32> = project.tracks.iter().map(|t| t.color as i32).collect();
    let track_colors_model = Rc::new(slint::VecModel::from(track_colors));
    ui.set_track_colors(track_colors_model.clone().into());

    let clips: Vec<ClipData> = project
        .clips
        .iter()
        .map(|c| to_clip_data(c, waveforms))
        .collect();
    let clips_model = Rc::new(slint::VecModel::from(clips));
    ui.set_clips(clips_model.clone().into());

    // Inserts iniciais (o estado de volume/pan/mute/etc. é aplicado
    // pelos MixerModels compartilhados em `main`).
    let (insert_counts, insert_names, insert_enabled, max_inserts) = mixer_inserts_state(project);
    ui.set_mixer_insert_counts(Rc::new(slint::VecModel::from(insert_counts)).into());
    ui.set_mixer_insert_names(Rc::new(slint::VecModel::from(insert_names)).into());
    ui.set_mixer_insert_enabled(Rc::new(slint::VecModel::from(insert_enabled)).into());
    ui.set_mixer_max_inserts(max_inserts);

    // Inicializa file browser na pasta de samples padrão. Se ela não
    // puder ser criada (disco cheio, perfil sem Documentos), cai para o
    // diretório de trabalho — abrir num explorador vazio parece defeito.
    let samples_dir = default_samples_dir();
    let start_dir = match std::fs::create_dir_all(&samples_dir) {
        Ok(()) => samples_dir,
        Err(err) => {
            eprintln!("[explorer] {} indisponível: {err}", samples_dir.display());
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }
    };
    let initial_path = start_dir.to_string_lossy().to_string();
    ui.set_explorer_current_path(initial_path.clone().into());
    ui.set_explorer_path_history(Rc::new(slint::VecModel::from(vec![initial_path.into()])).into());
    ui.set_explorer_history_index(0);
    ui.set_explorer_can_go_up(start_dir.parent().is_some_and(|p| p.is_dir()));
    let initial_entries = load_directory_entries(&start_dir, "");
    ui.set_explorer_entries(Rc::new(slint::VecModel::from(initial_entries)).into());

    (tracks_model, clips_model, track_colors_model)
}

/// Índice da linha do modelo cujo clip tem o id informado.
pub fn clip_row(model: &slint::VecModel<ClipData>, id: i32) -> Option<usize> {
    (0..model.row_count()).find(|&i| model.row_data(i).map(|c| c.id == id).unwrap_or(false))
}

/// Aplica um `DocumentEvent` aos modelos da UI.
///
/// `undo = true` reverte a mudança (caso contrário aplica/refaz).
/// Nunca toca no `Project` — apenas nos modelos, pois é chamado
/// de dentro de uma mutação do core (reentrância proibida).
pub fn apply_change(
    tracks_model: &slint::VecModel<slint::SharedString>,
    clips_model: &slint::VecModel<ClipData>,
    event: &DocumentEvent,
    undo: bool,
    waveforms: &WaveformCache,
) {
    match event {
        DocumentEvent::TrackAdded(track) => {
            if undo {
                let track_id = track.id as i32;
                let mut i = 0;
                while i < clips_model.row_count() {
                    if clips_model
                        .row_data(i)
                        .map(|c| c.track_index == track_id)
                        .unwrap_or(false)
                    {
                        clips_model.remove(i);
                    } else {
                        i += 1;
                    }
                }
                if tracks_model.row_count() > 0 {
                    tracks_model.remove(tracks_model.row_count() - 1);
                }
            } else {
                tracks_model.push(track.name.clone().into());
            }
        }
        DocumentEvent::ClipAdded(clip) => {
            if undo {
                if let Some(i) = clip_row(clips_model, clip.id as i32) {
                    clips_model.remove(i);
                }
            } else {
                clips_model.push(to_clip_data(clip, waveforms));
            }
        }
        DocumentEvent::ClipSliced {
            original,
            left,
            right,
        } => {
            if undo {
                if let Some(i) = clip_row(clips_model, right.id as i32) {
                    clips_model.remove(i);
                }
                if let Some(i) = clip_row(clips_model, original.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(original, waveforms));
                }
            } else {
                if let Some(i) = clip_row(clips_model, left.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left, waveforms));
                }
                clips_model.push(to_clip_data(right, waveforms));
            }
        }
        DocumentEvent::ClipMoved { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipDeleted(clip) => {
            if undo {
                clips_model.push(to_clip_data(clip, waveforms));
            } else if let Some(i) = clip_row(clips_model, clip.id as i32) {
                clips_model.remove(i);
            }
        }
        DocumentEvent::ClipsGlued {
            left_before,
            left,
            right,
        } => {
            if undo {
                if let Some(i) = clip_row(clips_model, left_before.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left_before, waveforms));
                }
                clips_model.push(to_clip_data(right, waveforms));
            } else {
                if let Some(i) = clip_row(clips_model, left.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left, waveforms));
                }
                if let Some(i) = clip_row(clips_model, right.id as i32) {
                    clips_model.remove(i);
                }
            }
        }
        DocumentEvent::ClipResized { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipRenamed { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipColorChanged { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipFadesChanged { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipGainChanged { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
        DocumentEvent::ClipNotesChanged { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data, waveforms));
            }
        }
    }
}

/// Assina os eventos do documento e mantém os modelos da UI
/// sincronizados de forma granular. `ui` é usado para crescer
/// `total-bars` quando um clip passa do fim atual da timeline.
pub fn bind_document_events(
    project: &Project,
    tracks_model: Rc<slint::VecModel<slint::SharedString>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    ui: slint::Weak<MainWindow>,
) {
    project.subscribe(move |event, kind| {
        let wf = waveforms.borrow();
        apply_change(
            &tracks_model,
            &clips_model,
            event,
            kind == ChangeKind::Undo,
            &wf,
        );
        drop(wf);
        // Cresce a timeline se o clip passou do fim atual.
        if let Some(end) = event_max_end_beat(event) {
            if let Some(ui) = ui.upgrade() {
                let needed = bars_for_end_beat(end, ui.get_beats_per_bar());
                if needed > ui.get_total_bars() {
                    ui.set_total_bars(needed);
                }
            }
        }
    });
}
