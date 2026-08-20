slint::include_modules!();

mod cursors;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cursors::{init_cursor_manager, set_cursor_for_tool, set_cursor_window_handle, set_timeline_bounds, set_beat_bounds, set_drumstick_active, reset_cursor_to_default};
use daw_core::clip::{Clip, ClipKind};
use daw_core::mixer::MixerParam;
use daw_core::project::{ChangeKind, DocumentEvent, Project};
use daw_core::timeline::Grid;
use daw_core::track::TrackKind;
use daw_core::transport::Transport;
use daw_media::{filesystem::default_samples_dir, filesystem::list_directory};
use daw_media::waveform::{decode_audio, waveform_points};
use daw_midi::{Note, NoteEditor};
use slint::Model;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn next_random(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let r = (*seed >> 33) as f32 / u32::MAX as f32;
    r
}

/// Máximo de slots de insert renderizados por canal no mixer.
const MAX_INSERTS: usize = 8;

/// Cache de waveform: clip_id → picos normalizados.
type WaveformCache = std::collections::HashMap<u64, Vec<f32>>;

/// Mapeia um clip do core para o DTO da camada Slint.
fn to_clip_data(clip: &Clip, waveforms: &WaveformCache) -> ClipData {
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat as f32,
        len: clip.length_beats as f32,
        is_midi: clip.is_midi(),
        color: clip.color as i32,
        waveform_points: {
            let empty: Vec<f32> = Vec::new();
            let peaks = waveforms.get(&clip.id).unwrap_or(&empty);
            Rc::new(slint::VecModel::from(peaks.clone())).into()
        },
    }
}

/// Atualiza apenas os waveform_points de um ClipData existente a partir do cache.
fn to_clip_data_from_id(id: u64, base: &ClipData, waveforms: &WaveformCache) -> ClipData {
    let empty: Vec<f32> = Vec::new();
    let peaks = waveforms.get(&id).unwrap_or(&empty);
    ClipData {
        id: base.id,
        name: base.name.clone(),
        track_index: base.track_index,
        beat: base.beat,
        len: base.len,
        is_midi: base.is_midi,
        color: base.color,
        waveform_points: Rc::new(slint::VecModel::from(peaks.clone())).into(),
    }
}

fn load_directory_entries(path: &std::path::Path) -> Vec<BrowserFileEntry> {
    if let Ok(entries) = list_directory(path) {
        entries
            .iter()
            .map(|e| {
                let entry_type = if e.is_directory {
                    "Folder".to_string()
                } else {
                    let ext = e.path.extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("")
                        .to_uppercase();
                    if ext == "WAV" || ext == "MP3" {
                        ext
                    } else {
                        "File".to_string()
                    }
                };
                BrowserFileEntry {
                    name: e.name.clone().into(),
                    entry_type: entry_type.into(),
                    path: e.path.to_string_lossy().to_string().into(),
                    is_directory: e.is_directory,
                }
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Editor de notas do piano roll (sem suporte a clip ainda: lista global
/// da janela). A fonte da verdade vive no core (`daw_midi::NoteEditor`);
/// o modelo Slint é re-sincronizado a cada edição.

/// Converte uma nota do core para o DTO da camada Slint.
fn to_piano_note(note: &Note) -> PianoNote {
    PianoNote {
        id: note.id,
        note: note.pitch,
        start: note.start,
        length: note.length,
        velocity: note.velocity,
    }
}

/// Converte um tick do core para o DTO da camada Slint.
fn to_ruler_tick(tick: &daw_core::RulerTick) -> RulerTick {
    RulerTick {
        bar: tick.bar,
        pos: tick.pos,
        level: tick.level,
    }
}

/// Modelos derivados de configuração do piano roll: grids, oitavas, réguas
/// e ticks. Tudo calculado no core (`daw_core::timeline::Grid`) — nada de
/// arrays hardcoded na UI.

/// Re-sincroniza o modelo de notas no global da janela.
fn refresh_notes<C: slint::ComponentHandle>(ui: &C, editor: &RefCell<NoteEditor>)
where
    for<'a> PianoRollData<'a>: slint::Global<'a, C>,
{
    let notes: Vec<PianoNote> = editor.borrow().notes().iter().map(to_piano_note).collect();
    PianoRollData::get(ui).set_notes(Rc::new(slint::VecModel::from(notes)).into());
}

/// Semeia os globals do piano roll e assina as ações de edição de notas.
fn wire_piano_roll<C: slint::ComponentHandle + 'static>(ui: &C, editor: Rc<RefCell<NoteEditor>>)
where
    for<'a> PianoRollData<'a>: slint::Global<'a, C>,
    for<'a> PianoRollActions<'a>: slint::Global<'a, C>,
{
    let grid = Grid::new(0, 127, 64);
    let data = PianoRollData::get(ui);
    data.set_first_note(grid.first_note);
    data.set_last_note(grid.last_note);
    data.set_note_lines(Rc::new(slint::VecModel::from(grid.note_lines)).into());
    data.set_beat_lines(Rc::new(slint::VecModel::from(grid.beat_lines)).into());
    data.set_octave_range(Rc::new(slint::VecModel::from(grid.octave_range)).into());
    data.set_bar_range(Rc::new(slint::VecModel::from(grid.bar_range)).into());
    let ruler_ticks: Vec<RulerTick> = grid.ruler_ticks.iter().map(to_ruler_tick).collect();
    data.set_ruler_ticks(Rc::new(slint::VecModel::from(ruler_ticks)).into());
    data.set_notes(Rc::new(slint::VecModel::from(Vec::<PianoNote>::new())).into());
    data.set_selected_note(-1);
    data.set_selected_notes(Rc::new(slint::VecModel::from(Vec::<i32>::new())).into());
    data.set_ppq(960);
    // Playhead começa em 0 pulses. Para animar, modifique `position` de acordo com
    // o transporte global do sequenciador (PPQ avançando no tempo).
    data.set_position(0);

    let actions = PianoRollActions::get(ui);

    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_note_added(move |note, start, length, velocity| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            editor.borrow_mut().add(note, start, length, velocity);
            refresh_notes(&ui, &editor);
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_note_moved(move |id, start| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            editor.borrow_mut().move_note(id, start);
            refresh_notes(&ui, &editor);
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_note_resized(move |id, length| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            editor.borrow_mut().resize_note(id, length);
            refresh_notes(&ui, &editor);
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_note_deleted(move |id| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            editor.borrow_mut().delete(id);
            if PianoRollData::get(&ui).get_selected_note() == id {
                PianoRollData::get(&ui).set_selected_note(-1);
            }
            refresh_notes(&ui, &editor);
        });
    }
    {
        let ui_weak = ui.as_weak();
        actions.on_note_selected(move |id| {
            if let Some(ui) = ui_weak.upgrade() {
                PianoRollData::get(&ui).set_selected_note(id);
            }
        });
    }
    {
        actions.on_note_previewed(move |note| {
            println!("[piano-roll] note preview: {}", note);
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_note_velocity_changed(move |id, velocity| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            editor.borrow_mut().change_note_velocity(id, velocity);
            refresh_notes(&ui, &editor);
        });
    }
    {
        let ui_weak = ui.as_weak();
        actions.on_notes_selection_changed(move |ids| {
            if let Some(ui) = ui_weak.upgrade() {
                PianoRollData::get(&ui).set_selected_notes(ids);
            }
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_quantize_requested(move |denom| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let sel = PianoRollData::get(&ui).get_selected_notes();
            let ids: Vec<i32> = sel.iter().collect::<Vec<_>>();
            if ids.is_empty() {
                editor.borrow_mut().quantize(denom);
            } else {
                editor.borrow_mut().quantize_selected(denom, &ids);
            }
            refresh_notes(&ui, &editor);
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_delete_selection_requested(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let sel = PianoRollData::get(&ui).get_selected_notes();
            let ids: Vec<i32> = sel.iter().collect::<Vec<_>>();
            if !ids.is_empty() {
                editor.borrow_mut().delete_many(&ids);
                PianoRollData::get(&ui).set_selected_note(-1);
                PianoRollData::get(&ui)
                    .set_selected_notes(Rc::new(slint::VecModel::from(Vec::<i32>::new())).into());
                refresh_notes(&ui, &editor);
            } else {
                let single = PianoRollData::get(&ui).get_selected_note();
                if single >= 0 {
                    editor.borrow_mut().delete(single);
                    PianoRollData::get(&ui).set_selected_note(-1);
                    PianoRollData::get(&ui)
                        .set_selected_notes(Rc::new(slint::VecModel::from(Vec::<i32>::new())).into());
                    refresh_notes(&ui, &editor);
                }
            }
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_lasso_selection_requested(move |x1, y1, x2, y2, px_per_beat, note_height, last_note| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            // Hit-test de seleção: vive no core (testável), a UI só projeta.
            let ids = editor
                .borrow()
                .notes_in_rect(x1, y1, x2, y2, px_per_beat, note_height, last_note);
            let first = ids.first().copied().unwrap_or(-1);

            PianoRollData::get(&ui)
                .set_selected_notes(Rc::new(slint::VecModel::from(ids)).into());
            PianoRollData::get(&ui).set_selected_note(first);
        });
    }
}

/// Extrai o estado do mixer do projeto (trilhas + master) como arrays.
fn mixer_state(project: &Project) -> (Vec<bool>, Vec<bool>, Vec<bool>, Vec<f32>, Vec<f32>) {
    let n = project.tracks.len() + 1; // trilhas + master
    let mut mutes = Vec::with_capacity(n);
    let mut soloes = Vec::with_capacity(n);
    let mut arms = Vec::with_capacity(n);
    let mut volumes = Vec::with_capacity(n);
    let mut pans = Vec::with_capacity(n);

    for t in &project.tracks {
        mutes.push(t.mixer.muted);
        soloes.push(t.mixer.soloed);
        arms.push(t.mixer.armed);
        volumes.push(t.mixer.volume);
        pans.push(t.mixer.pan);
    }
    // Master
    mutes.push(project.master_mixer.muted);
    soloes.push(project.master_mixer.soloed);
    arms.push(project.master_mixer.armed);
    volumes.push(project.master_mixer.volume);
    pans.push(project.master_mixer.pan);

    (mutes, soloes, arms, volumes, pans)
}

/// Extrai o estado de inserts do mixer como arrays achatados.
///
/// Cada canal ocupa `MAX_INSERTS` entradas em `names`/`enabled`
/// (índice `canal * MAX_INSERTS + slot`). Retorna as contagens por
/// canal, nomes, estados e o maior nº de inserts entre todos os canais.
fn mixer_inserts_state(
    project: &Project,
) -> (Vec<i32>, Vec<slint::SharedString>, Vec<bool>, i32) {
    let n = project.tracks.len() + 1; // trilhas + master
    let mut counts = Vec::with_capacity(n);
    let mut names = vec![slint::SharedString::default(); n * MAX_INSERTS];
    let mut enabled = vec![false; n * MAX_INSERTS];
    let mut max_count = 0;
    for (channel, mixer) in project
        .tracks
        .iter()
        .map(|t| &t.mixer)
        .chain(std::iter::once(&project.master_mixer))
        .enumerate()
    {
        counts.push(mixer.inserts.len() as i32);
        max_count = max_count.max(mixer.inserts.len() as i32);
        for (slot, insert) in mixer.inserts.iter().enumerate() {
            let idx = channel * MAX_INSERTS + slot;
            names[idx] = insert.name.clone().into();
            enabled[idx] = insert.enabled;
        }
    }
    (counts, names, enabled, max_count)
}

/// VecModel do mixer COMPARTILHADOS entre a janela principal e as
/// flutuantes. Como todas as janelas apontam para os mesmos `Rc`,
/// um `set_row_data` por canal propaga para todas de uma vez — sem
/// recriar modelos a cada movimento de fader/knob (zero garbage).
struct MixerModels {
    mutes: Rc<slint::VecModel<bool>>,
    soloes: Rc<slint::VecModel<bool>>,
    arms: Rc<slint::VecModel<bool>>,
    volumes: Rc<slint::VecModel<f32>>,
    pans: Rc<slint::VecModel<f32>>,
}

impl MixerModels {
    fn from_project(project: &Project) -> Self {
        let (mutes, soloes, arms, volumes, pans) = mixer_state(project);
        Self {
            mutes: Rc::new(slint::VecModel::from(mutes)),
            soloes: Rc::new(slint::VecModel::from(soloes)),
            arms: Rc::new(slint::VecModel::from(arms)),
            volumes: Rc::new(slint::VecModel::from(volumes)),
            pans: Rc::new(slint::VecModel::from(pans)),
        }
    }

    /// Atualiza apenas o canal informado (propaga para todas as janelas).
    fn update_channel(&self, channel: usize, param: MixerParam, value: f32) {
        match param {
            MixerParam::Volume => self.volumes.set_row_data(channel, value.clamp(0.0, 1.0)),
            MixerParam::Pan => self.pans.set_row_data(channel, value.clamp(0.0, 1.0)),
            MixerParam::Muted => self.mutes.set_row_data(channel, value >= 0.5),
            MixerParam::Soloed => self.soloes.set_row_data(channel, value >= 0.5),
            MixerParam::Armed => self.arms.set_row_data(channel, value >= 0.5),
        }
    }
}

/// Aplica os modelos do mixer numa janela (principal ou flutuante).
macro_rules! apply_mixer_models {
    ($ui:expr, $m:expr) => {{
        $ui.set_mixer_mutes($m.mutes.clone().into());
        $ui.set_mixer_soloes($m.soloes.clone().into());
        $ui.set_mixer_arms($m.arms.clone().into());
        $ui.set_mixer_volumes($m.volumes.clone().into());
        $ui.set_mixer_pans($m.pans.clone().into());
    }};
}

/// Preenche os modelos da UI a partir do estado inicial do core.
///
/// Retorna os `VecModel` compartilhados para que o assinante de
/// eventos possa atualizá-los granularmente em runtime.
fn initial_bind(
    ui: &MainWindow,
    project: &Project,
    waveforms: &WaveformCache,
) -> (
    Rc<slint::VecModel<slint::SharedString>>,
    Rc<slint::VecModel<ClipData>>,
) {
    let track_names: Vec<slint::SharedString> = project
        .tracks
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    let tracks_model = Rc::new(slint::VecModel::from(track_names));
    ui.set_track_names(tracks_model.clone().into());

    let clips: Vec<ClipData> = project.clips.iter().map(|c| to_clip_data(c, waveforms)).collect();
    let clips_model = Rc::new(slint::VecModel::from(clips));
    ui.set_clips(clips_model.clone().into());

    // Inserts iniciais (o estado de volume/pan/mute/etc. é aplicado
    // pelos MixerModels compartilhados em `main`).
    let (insert_counts, insert_names, insert_enabled, max_inserts) = mixer_inserts_state(project);
    ui.set_mixer_insert_counts(Rc::new(slint::VecModel::from(insert_counts)).into());
    ui.set_mixer_insert_names(Rc::new(slint::VecModel::from(insert_names)).into());
    ui.set_mixer_insert_enabled(Rc::new(slint::VecModel::from(insert_enabled)).into());
    ui.set_mixer_max_inserts(max_inserts);

    // Inicializa file browser com pasta de samples padrão
    let samples_dir = default_samples_dir();
    std::fs::create_dir_all(&samples_dir).ok();
    let initial_path = samples_dir.to_string_lossy().to_string();
    ui.set_explorer_current_path(initial_path.clone().into());
    ui.set_explorer_path_history(Rc::new(slint::VecModel::from(vec![initial_path.into()])).into());
    ui.set_explorer_history_index(0);
    let initial_entries = load_directory_entries(&samples_dir);
    ui.set_explorer_entries(Rc::new(slint::VecModel::from(initial_entries)).into());

    (tracks_model, clips_model)
}


/// Índice da linha do modelo cujo clip tem o id informado.
fn clip_row(model: &slint::VecModel<ClipData>, id: i32) -> Option<usize> {
    (0..model.row_count()).find(|&i| model.row_data(i).map(|c| c.id == id).unwrap_or(false))
}

/// Aplica um `DocumentEvent` aos modelos da UI.
///
/// `undo = true` reverte a mudança (caso contrário aplica/refaz).
/// Nunca toca no `Project` — apenas nos modelos, pois é chamado
/// de dentro de uma mutação do core (reentrância proibida).
fn apply_change(
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
    }
}

/// Assina os eventos do documento e mantém os modelos da UI
/// sincronizados de forma granular.
fn bind_document_events(
    project: &Project,
    tracks_model: Rc<slint::VecModel<slint::SharedString>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    waveforms: Rc<RefCell<WaveformCache>>,
) {
    project.subscribe(move |event, kind| {
        let wf = waveforms.borrow();
        apply_change(&tracks_model, &clips_model, event, kind == ChangeKind::Undo, &wf);
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    init_cursor_manager().expect("Failed to initialize cursor manager");

    let window = ui.window();
    window.on_close_requested(move || {
        let _ = reset_cursor_to_default();
        slint::CloseRequestResponse::HideWindow
    });

// Set up cursor window handle after window is created
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        #[cfg(windows)]
        {
            if let Some(ui) = ui_weak.upgrade() {
                let window = ui.window();
                let handle = window.window_handle();
                if let Ok(raw_handle) = handle.window_handle() {
                    if let RawWindowHandle::Win32(h) = raw_handle.as_raw() {
                        set_cursor_window_handle(h.hwnd.get() as *mut std::ffi::c_void);
                        let _ = set_cursor_for_tool(0);
                    }
                }
            }
        }
    });

    // Estado autoritativo da DAW: vive no core, nunca na UI.
    let project = Rc::new(RefCell::new(Project::demo()));
    let transport = Arc::new(Mutex::new(Transport::default()));

    // Cache de waveforms: clip_id → picos normalizados.
    let waveforms: Rc<RefCell<WaveformCache>> = Rc::new(RefCell::new(HashMap::new()));

    let (tracks_model, clips_model) = initial_bind(&ui, &project.borrow(), &waveforms.borrow());
    bind_document_events(&project.borrow(), tracks_model, clips_model.clone(), waveforms.clone());

    // Mixer: modelos compartilhados — atualização por canal (set_row_data)
    // propaga para a janela principal e todas as flutuantes de uma vez.
    let mixer_models: Rc<RefCell<MixerModels>> =
        Rc::new(RefCell::new(MixerModels::from_project(&project.borrow())));
    apply_mixer_models!(&ui, &mixer_models.borrow());

    // Meters: modelo compartilhado + estado de decay (smoothing no Rust).
    let meter_model: Rc<slint::VecModel<f32>> =
        Rc::new(slint::VecModel::from(vec![0.0; project.borrow().tracks.len() + 1]));
    ui.set_mixer_meters(meter_model.clone().into());
    let meter_state: Rc<RefCell<Vec<f32>>> =
        Rc::new(RefCell::new(vec![0.0; project.borrow().tracks.len() + 1]));

    // Piano roll da janela principal (estado por janela; cada float tem o seu).
    wire_piano_roll(&ui, Rc::new(RefCell::new(NoteEditor::new())));

    // Slots das janelas flutuantes, um por tool (0=Mixer, 1=Sampler, 2=Piano Roll).
    let float_panels: Rc<RefCell<[Option<FloatPanel>; 3]>> = Rc::new(RefCell::new([None, None, None]));

    // Espelha quais janelas estão abertas na propriedade `floated` da UI.
    let sync_floated = {
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        move || {
            let p = panels.borrow();
            let arr: Vec<bool> = vec![p[0].is_some(), p[1].is_some(), p[2].is_some()];
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_floated(Rc::new(slint::VecModel::from(arr)).into());
            }
        }
    };

    // Reaplica o estado do mixer nas janelas (principal + todas as flutuantes)
    // a partir do core. USADO só em mudanças estruturais (tracks/inserts);
    // fader/knob usam atualização por canal (update_channel), sem rebuild.
    let apply_mixer_state = {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        let holder = mixer_models.clone();
        move || {
            let proj = project.borrow();
            *holder.borrow_mut() = MixerModels::from_project(&proj);
            let models = holder.borrow();
            let (insert_counts, insert_names, insert_enabled, max_inserts) = mixer_inserts_state(&proj);
            let insert_counts = Rc::new(slint::VecModel::from(insert_counts));
            let insert_names = Rc::new(slint::VecModel::from(insert_names));
            let insert_enabled = Rc::new(slint::VecModel::from(insert_enabled));
            if let Some(ui) = ui_weak.upgrade() {
                apply_mixer_models!(&ui, &models);
                ui.set_mixer_insert_counts(insert_counts.clone().into());
                ui.set_mixer_insert_names(insert_names.clone().into());
                ui.set_mixer_insert_enabled(insert_enabled.clone().into());
                ui.set_mixer_max_inserts(max_inserts);
            }
            let slots = panels.borrow();
            for slot in slots.iter().flatten() {
                apply_mixer_models!(slot, &models);
                slot.set_mixer_insert_counts(insert_counts.clone().into());
                slot.set_mixer_insert_names(insert_names.clone().into());
                slot.set_mixer_insert_enabled(insert_enabled.clone().into());
                slot.set_mixer_max_inserts(max_inserts);
            }
        }
    };

    // Mixer flutuante: a largura acompanha o nº de canais (só quando
    // o nº de canais muda — nunca durante drag de fader).
    let fit_float_mixer_windows = {
        let project = project.clone();
        let panels = float_panels.clone();
        move || {
            let slots = panels.borrow();
            for slot in slots.iter().flatten() {
                if slot.get_tool_tab() == 0 {
                    let proj = project.borrow();
                    let w = ((proj.tracks.len() + 1) as f32 * 105.0 * slot.window().scale_factor()) as u32;
                    let h = slot.window().size().height;
                    slot.window().set_size(slint::PhysicalSize::new(w, h));
                }
            }
        }
    };

    // Janelas flutuantes: cada tool desacopla para a própria janela, várias ao mesmo tempo.
    {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        let apply_mixer = apply_mixer_state.clone();
        let sync_floated = sync_floated.clone();
        // Clone para o float toggle sem consumir os originais
        let mixer_models_for_float = mixer_models.clone();
        let meter_model_for_float = meter_model.clone();
        ui.on_float_toggle(move |tab| {
            let _ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let idx = tab as usize;
            if idx > 2 {
                return;
            }

            // Se já existe, fecha; senão, cria.
            let should_open;
            {
                let mut slots = panels.borrow_mut();
                if let Some(p) = slots[idx].take() {
                    p.hide().ok();
                    should_open = false;
                } else {
                    should_open = true;
                }
            }
            sync_floated();
            if !should_open {
                return;
            }

            let p = match FloatPanel::new() {
                Ok(p) => p,
                Err(_) => return,
            };
            p.set_tool_tab(tab);
            // Meters compartilhados: o timer atualiza um único modelo
            // e todas as janelas recebem o mesmo sinal.
            p.set_mixer_meters(meter_model_for_float.clone().into());

            // Piano roll próprio da janela flutuante.
            wire_piano_roll(&p, Rc::new(RefCell::new(NoteEditor::new())));

            // Fecha via botão da própria janela flutuante.
            {
                let panels = panels.clone();
                let sync = sync_floated.clone();
                p.on_close_panel(move || {
                    if let Some(p) = panels.borrow_mut()[idx].take() {
                        p.hide().ok();
                    }
                    sync();
                });
            }

            // Fecha via botão X do sistema.
            {
                let panels = panels.clone();
                let sync = sync_floated.clone();
                p.window().on_close_requested(move || {
                    if let Some(p) = panels.borrow_mut()[idx].take() {
                        p.hide().ok();
                    }
                    sync();
                    slint::CloseRequestResponse::HideWindow
                });
            }

            // Ajustes feitos dentro da janela flutuante → core (atualiza só o canal).
            {
                let project = project.clone();
                let mixer_models = mixer_models_for_float.clone();
                p.on_mixer_param_changed(move |channel, param, value| {
                    let Some(param) = MixerParam::from_str(param.as_str()) else {
                        return;
                    };
                    let channel = channel as usize;
                    project.borrow_mut().set_mixer_param(channel, param, value);
                    mixer_models.borrow().update_channel(channel, param, value);
                });
            }

            // Ajusta a janela flutuante do mixer ao conteúdo: largura pelos
            // canais e altura pela cadeia de inserts (com teto).
            let fit_mixer_window = {
                let project = project.clone();
                let p_weak = p.as_weak();
                move || {
                    let Some(p) = p_weak.upgrade() else {
                        return;
                    };
                    if p.get_tool_tab() != 0 {
                        return;
                    }
                    let sf = p.window().scale_factor();
                    let proj = project.borrow();
                    let w = ((proj.tracks.len() + 1) as f32 * 105.0 * sf) as u32;
                    let h = (p.get_mixer_natural_height().clamp(240.0, 640.0) * sf) as u32;
                    p.window().set_size(slint::PhysicalSize::new(w, h));
                }
            };

            // Inserts do mixer na janela flutuante → core (reajusta a altura).
            {
                let project = project.clone();
                let apply_mixer = apply_mixer.clone();
                let fit = fit_mixer_window.clone();
                p.on_mixer_insert_added(move |channel| {
                    project.borrow_mut().add_mixer_insert(channel as usize);
                    apply_mixer();
                    fit();
                });
            }
            {
                let project = project.clone();
                let apply_mixer = apply_mixer.clone();
                let fit = fit_mixer_window.clone();
                p.on_mixer_insert_removed(move |channel, slot| {
                    project.borrow_mut().remove_mixer_insert(channel as usize, slot as usize);
                    apply_mixer();
                    fit();
                });
            }
            {
                let project = project.clone();
                let apply_mixer = apply_mixer.clone();
                p.on_mixer_insert_toggled(move |channel, slot| {
                    project.borrow_mut().toggle_mixer_insert(channel as usize, slot as usize);
                    apply_mixer();
                });
            }

            // Redimensionamento da janela flutuante pelo canto inferior direito.
            {
                let p_weak = p.as_weak();
                let state: Rc<RefCell<Option<(u32, u32, f32, f32)>>> = Rc::new(RefCell::new(None));
                p.on_resize_cancel({
                    let state = state.clone();
                    move || {
                        *state.borrow_mut() = None;
                    }
                });
                p.on_resize_end({
                    let state = state.clone();
                    move || {
                        *state.borrow_mut() = None;
                    }
                });
                p.on_resize_move({
                    let state = state.clone();
                    let p_weak = p_weak.clone();
                    move |x, y| {
                        let Some(panel) = p_weak.upgrade() else {
                            return;
                        };
                        let win = panel.window();
                        let sf = win.scale_factor();
                        let is_mixer = panel.get_tool_tab() == 0;
                        let mut st = state.borrow_mut();
                        // Primeiro move do drag → ancora o tamanho inicial e o ponteiro.
                        let Some((w0, h0, x0, y0)) = *st else {
                            let sz = win.size();
                            *st = Some((sz.width, sz.height, x, y));
                            return;
                        };
                        if is_mixer {
                            // Mixer: largura é automática (segue os canais); só a altura
                            // redimensiona, limitada entre o mínimo e o teto (640 lógicos).
                            let new_h = ((h0 as f32 + (y - y0) * sf).clamp(240.0 * sf, 640.0 * sf)) as u32;
                            win.set_size(slint::PhysicalSize::new(win.size().width, new_h));
                        } else {
                            let new_w = (w0 as f32 + (x - x0) * sf).max(360.0 * sf) as u32;
                            let new_h = (h0 as f32 + (y - y0) * sf).max(240.0 * sf) as u32;
                            win.set_size(slint::PhysicalSize::new(new_w, new_h));
                        }
                    }
                });
            }

            p.show().ok();
            {
                let mut slots = panels.borrow_mut();
                slots[idx] = Some(p);
            }
            sync_floated();
            apply_mixer();
            fit_mixer_window();
        });
    }

    // Drop de clip (ClipPool / Explorer) → core (evento reprojeta).
    {
        let project = project.clone();
        ui.on_clip_dropped(move |track, beat, name, is_midi| {
            let kind = if is_midi {
                ClipKind::Midi
            } else {
                ClipKind::Audio
            };
            let track_id = track.max(0) as u64;
            let start = beat.round().max(0.0) as u32;
            let mut proj = project.borrow_mut();
            if proj.track(track_id).is_some() {
                proj.add_clip(track_id, kind, name.as_str(), start, 4);
            }
        });
    }

    // Adiciona tracks via botão "+" da timeline → core.
    {
        let project = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        let fit_float_mixers = fit_float_mixer_windows.clone();
        ui.on_add_track_requested(move |kind| {
            let kind = match kind.as_str() {
                "AUDIO" => TrackKind::Audio,
                "INSTRUMENT" => TrackKind::Instrument,
                "DRUM" => TrackKind::Drum,
                "FX" => TrackKind::Fx,
                "BUS" => TrackKind::Bus,
                "MASTER" => TrackKind::Master,
                _ => TrackKind::Midi,
            };
            {
                let mut proj = project.borrow_mut();
                let count = proj.tracks.len() + 1;
                proj.add_track(kind, format!("{} {count}", kind.base_name()));
            }
            apply_mixer();
            fit_float_mixers();
        });
    }

    // Slice de clip (ferramenta de corte) → core.
    {
        let project = project.clone();
        ui.on_clip_slice_requested(move |id, position| {
            let id = id as u64;
            let mut proj = project.borrow_mut();
            let Some(clip) = proj.clip(id).cloned() else {
                return;
            };
            let at = clip.start_beat + position.round().max(0.0) as u32;
            proj.slice_clip(id, at);
        });
    }

    // Glue de clip (ferramenta cola) → core: funde com o vizinho à direita.
    {
        let project = project.clone();
        ui.on_clip_glue_requested(move |id| {
            project.borrow_mut().glue_clip(id as u64);
        });
    }

    // Hit-test da ferramenta cola: converte px→beat na UI e delega a
    // busca do clip ao core (`Project::clip_at_beat`), ou -1 se nenhum.
    {
        let project = project.clone();
        ui.on_glue_clip_at(move |row, x, scroll_x, px_per_beat| -> i32 {
            let project = project.borrow();
            let row = row.max(0.0) as i32;
            let ppb = px_per_beat as f32;
            if ppb <= 0.0 {
                return -1;
            }
            let beat = ((x as f32 + scroll_x as f32).max(0.0) / ppb) as u32;
            project
                .clip_at_beat(row as u64, beat)
                .map(|clip| clip.id as i32)
                .unwrap_or(-1)
        });
    }

    // Estado para armazenar posição original de clips durante move/resize.
    let move_originals: Rc<RefCell<HashMap<i32, (f32, i32)>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let move_track_deltas: Rc<RefCell<HashMap<i32, i32>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let resize_originals: Rc<RefCell<HashMap<i32, (f32, f32)>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // Mover clip via drag (ferramenta select) — armazena posição original.
    {
        let clips_model = clips_model.clone();
        let move_originals = move_originals.clone();
        ui.on_clip_move_started(move |id, _beat| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(clip) = clips_model.row_data(i) {
                    move_originals.borrow_mut().insert(id, (clip.beat, clip.track_index));
                }
            }
        });
    }

    // Mover clip: delta acumulado do início do drag → aplica sobre a posição original.
    // NOTA: track_index NÃO é alterado durante o drag (só beat).
    // O delta de track é guardado e aplicado apenas no finish.
    {
        let clips_model = clips_model.clone();
        let move_originals = move_originals.clone();
        let move_track_deltas = move_track_deltas.clone();
        let ui_weak = ui.as_weak();
        ui.on_clip_move_delta(move |id, delta_beats, delta_tracks| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut clip) = clips_model.row_data(i) {
                    let originals = move_originals.borrow();
                    if let Some(&(orig_beat, _orig_track)) = originals.get(&id) {
                        clip.beat = (orig_beat + delta_beats).max(0.0);
                        clips_model.set_row_data(i, clip);
                    }
                    drop(originals);
                    move_track_deltas.borrow_mut().insert(id, delta_tracks);
                }
            }
            // Visual feedback: move clip vertically via translate-y
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_drag_clip_id(id);
                ui.set_drag_track_offset(delta_tracks);
            }
        });
    }

    // Mover clip: finalizado → persiste no core e limpa estado.
    {
        let project = project.clone();
        let clips_model = clips_model.clone();
        let move_originals = move_originals.clone();
        let move_track_deltas = move_track_deltas.clone();
        let ui_weak = ui.as_weak();
        ui.on_clip_move_finished(move |id| {
            let orig = move_originals.borrow_mut().remove(&id);
            let delta_tracks = move_track_deltas.borrow_mut().remove(&id).unwrap_or(0);

            // Clear visual drag offset
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_drag_clip_id(-1);
                ui.set_drag_track_offset(0);
            }

            if let Some((_, orig_track)) = orig {
                if let Some(i) = clip_row(&clips_model, id) {
                    if let Some(mut clip) = clips_model.row_data(i) {
                        let num_tracks = project.borrow().tracks.len() as i32;
                        clip.track_index = (orig_track + delta_tracks).max(0).min(num_tracks - 1);
                        let new_beat = clip.beat.round().max(0.0) as u32;
                        let track_id = clip.track_index as u64;
                        clips_model.set_row_data(i, clip);
                        let mut proj = project.borrow_mut();
                        proj.move_clip(id as u64, track_id, new_beat);
                    }
                }
            }
        });
    }

    // Resize de clip (handles das bordas) → core.
    {
        let clips_model = clips_model.clone();
        let resize_originals = resize_originals.clone();
        ui.on_clip_resize_started(move |id, _side| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(clip) = clips_model.row_data(i) {
                    resize_originals.borrow_mut().insert(id, (clip.beat, clip.len));
                }
            }
        });
    }
    {
        // resize-delta: feedback visual imediato (atualiza beat + len no modelo).
        let clips_model = clips_model.clone();
        let project_snap = project.clone();
        ui.on_clip_resize_delta(move |id, side, delta| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut clip) = clips_model.row_data(i) {
                    let core_clip = project_snap
                        .borrow()
                        .clips
                        .iter()
                        .find(|c| c.id as i32 == id)
                        .cloned();
                    if let Some(core) = core_clip {
                        if side < 0 {
                            // borda esquerda: move start e reduz comprimento
                            let new_beat = (core.start_beat as f32 + delta).max(0.0);
                            let new_len = (core.length_beats as f32 - delta).max(0.5);
                            clip.beat = new_beat;
                            clip.len = new_len;
                        } else {
                            // borda direita: estende/encolhe comprimento
                            let new_len = (core.length_beats as f32 + delta).max(0.5);
                            clip.len = new_len;
                        }
                        clips_model.set_row_data(i, clip);
                    }
                }
            }
        });
    }
    {
        // resize-finished: persiste no core com valores do modelo e limpa estado.
        let project = project.clone();
        let clips_model = clips_model.clone();
        let resize_originals = resize_originals.clone();
        ui.on_clip_resize_finished(move |id| {
            resize_originals.borrow_mut().remove(&id);
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(clip) = clips_model.row_data(i) {
                    let new_start = clip.beat.round().max(0.0) as u32;
                    let new_len = clip.len.round().max(1.0) as u32;
                    project.borrow_mut().resize_clip(id as u64, new_start, new_len);
                }
            }
        });
    }

    // Delete de clip → core.
    {
        let project = project.clone();
        ui.on_clip_delete_requested(move |id| {
            project.borrow_mut().delete_clip(id as u64);
        });
    }

    // Rename de clip (double-click + edição inline) → core.
    {
        let project = project.clone();
        ui.on_clip_rename_requested(move |id, new_name| {
            project.borrow_mut().rename_clip(id as u64, new_name.as_str());
        });
    }

    // Ciclo de cor do clip (swatch click) → core (incrementa paleta mod 8).
    {
        let project = project.clone();
        ui.on_clip_color_cycle_requested(move |id| {
            let new_color = {
                let proj = project.borrow();
                let current = proj.clips.iter().find(|c| c.id as i32 == id).map(|c| c.color).unwrap_or(0);
                (current + 1) % 8
            };
            project.borrow_mut().set_clip_color(id as u64, new_color);
        });
    }

    // Undo / redo (Ctrl+Z / Ctrl+Y) → histórico do core.
    {
        let p = project.clone();
        ui.on_undo_requested(move || {
            p.borrow_mut().undo();
        });
        let p = project.clone();
        ui.on_redo_requested(move || {
            p.borrow_mut().redo();
        });
    }

    // Parâmetros do mixer → core (atualização por canal, sem rebuild).
    {
        let project = project.clone();
        let mixer_models = mixer_models.clone();
        ui.on_mixer_param_changed(move |channel, param, value| {
            let Some(param) = MixerParam::from_str(param.as_str()) else {
                return;
            };
            let channel = channel as usize;
            project.borrow_mut().set_mixer_param(channel, param, value);
            mixer_models.borrow().update_channel(channel, param, value);
        });
    }

    // Inserts do mixer (janela principal) → core.
    {
        let project_add = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        ui.on_mixer_insert_added(move |channel| {
            project_add.borrow_mut().add_mixer_insert(channel as usize);
            apply_mixer();
        });
        let project_rm = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        ui.on_mixer_insert_removed(move |channel, slot| {
            project_rm.borrow_mut().remove_mixer_insert(channel as usize, slot as usize);
            apply_mixer();
        });
        let project_tg = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        ui.on_mixer_insert_toggled(move |channel, slot| {
            project_tg.borrow_mut().toggle_mixer_insert(channel as usize, slot as usize);
            apply_mixer();
        });
    }

    // Filesystem: navegação de diretórios
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_path_changed(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                let path = PathBuf::from(path.as_str());
                if path.is_dir() {
                    // Atualiza histórico
                    let history: Vec<slint::SharedString> = ui.get_explorer_path_history().iter().collect();
                    let idx = ui.get_explorer_history_index();
                    let mut new_history = history[..=idx as usize].to_vec();
                    new_history.push(slint::SharedString::from(path.to_string_lossy().as_ref()));
                    ui.set_explorer_path_history(Rc::new(slint::VecModel::from(new_history)).into());
                    ui.set_explorer_history_index(idx + 1);
                    ui.set_explorer_current_path(slint::SharedString::from(path.to_string_lossy().as_ref()));

                    // Carrega entradas
                    // Carrega entradas
                    let file_entries = load_directory_entries(&path);
                    ui.set_explorer_entries(Rc::new(slint::VecModel::from(file_entries)).into());
                }
            }
        });
    }

    // Filesystem: entrar em diretório
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_directory_entered(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                let path_buf = PathBuf::from(path.as_str());
                if path_buf.is_dir() {
                    let history: Vec<slint::SharedString> = ui.get_explorer_path_history().iter().collect();
                    let idx = ui.get_explorer_history_index();
                    let mut new_history = history[..=idx as usize].to_vec();
                    new_history.push(slint::SharedString::from(path_buf.to_string_lossy().as_ref()));
                    ui.set_explorer_path_history(Rc::new(slint::VecModel::from(new_history)).into());
                    ui.set_explorer_history_index(idx + 1);
                    ui.set_explorer_current_path(slint::SharedString::from(path_buf.to_string_lossy().as_ref()));

                    let file_entries = load_directory_entries(&path_buf);
                    ui.set_explorer_entries(Rc::new(slint::VecModel::from(file_entries)).into());
                }
            }
        });
    }

    let mut seed = 0x1234_5678_9abc_def0u64;

    // Playback: o AudioEngine avança o Transport na thread de áudio.
    // O timer UI (16ms) apenas lê position_ticks e projeta na UI.
    // Mixer meters animados também rodam enquanto tocando.
    {
        let ui_weak = ui.as_weak();
        let transport_timer = Rc::new(RefCell::new(slint::Timer::default()));
        let transport_timer_ref = transport_timer.clone();
        let transport = transport.clone();
        let project = project.clone();
        let meter_timer = Rc::new(RefCell::new(slint::Timer::default()));
        let meter_timer_ref = meter_timer.clone();
        // AudioEngine: fica vivo enquanto estiver tocando
        let audio_engine: Rc<RefCell<Option<daw_audio::AudioEngine>>> =
            Rc::new(RefCell::new(None));
        let audio_snapshot: Rc<RefCell<Option<Arc<Mutex<daw_audio::ProjectSnapshot>>>>> =
            Rc::new(RefCell::new(None));
        let audio_buffers: Rc<RefCell<Option<Arc<Mutex<daw_audio::BufferCache>>>>> =
            Rc::new(RefCell::new(None));
        ui.on_play_state_changed(move |playing| {
            if playing {
                // Inicia o audio engine (abre stream CPAL)
                match daw_audio::AudioEngine::start(transport.clone(), 48000) {
                    Ok((engine, snapshot, buffers)) => {
                        // Atualiza o snapshot e pré-carrega clips
                        {
                            let proj = project.borrow();
                            let mut snap = snapshot.lock().unwrap();
                            snap.update_from(&proj);
                        }
                        engine.preload_clips(&project.borrow());
                        *audio_snapshot.borrow_mut() = Some(snapshot);
                        *audio_buffers.borrow_mut() = Some(buffers);
                        *audio_engine.borrow_mut() = Some(engine);
                    }
                    Err(e) => {
                        eprintln!("audio engine: {}", e);
                    }
                }
                transport.lock().unwrap().playing = true;

                // Transport timer: só LÊ a posição do transport (nunca avança).
                // O audio callback é quem avança.
                {
                    let ui_weak_t = ui_weak.clone();
                    let transport = transport.clone();
                    transport_timer_ref.borrow().start(
                        slint::TimerMode::Repeated,
                        Duration::from_millis(16),
                        move || {
                            if let Some(ui) = ui_weak_t.upgrade() {
                                let t = transport.lock().unwrap();
                                ui.set_transport_position(t.position_ticks as i32);
                            }
                        },
                    );
                }
                // Meter timer: VU fake com smoothing no Rust (attack
                // instantâneo, decay lento). SEM animação Slint — a
                // animação contínua satura o renderer e trava o playhead.
                let project = project.clone();
                let meter_model = meter_model.clone();
                let meter_state = meter_state.clone();
                meter_timer_ref.borrow().start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(80),
                    move || {
                        let count = project.borrow().tracks.len() + 1;
                        let mut levels = meter_state.borrow_mut();
                        if levels.len() != count {
                            levels.resize(count, 0.0);
                        }
                        for level in levels.iter_mut() {
                            let r = next_random(&mut seed);
                            let new = r * r;
                            *level = if new > *level { new } else { *level * 0.82 };
                        }
                        for (i, &v) in levels.iter().enumerate() {
                            meter_model.set_row_data(i, v);
                        }
                    },
                );
            } else {
                transport.lock().unwrap().playing = false;
                transport_timer_ref.borrow().stop();
                meter_timer_ref.borrow().stop();
                // Para o audio engine
                *audio_engine.borrow_mut() = None;
                *audio_snapshot.borrow_mut() = None;
                *audio_buffers.borrow_mut() = None;
            }
        });
    }

    // Seek-to: régua da timeline clica para posicionar playhead.
    {
        let transport = transport.clone();
        let ui_weak = ui.as_weak();
        ui.on_seek_to(move |beat| {
            let ticks = (beat * 960.0) as u64;
            transport.lock().unwrap().seek(ticks);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_transport_position(transport.lock().unwrap().position_ticks as i32);
            }
        });
    }

    ui.on_tool_changed(move |tool| {
        let _ = set_cursor_for_tool(tool);
    });

    ui.on_timeline_bounds_changed(move |x, y, w, h| {
        set_timeline_bounds(x, y, w, h);
    });

    ui.on_beat_bounds_changed(move |x, y, w, h| {
        set_beat_bounds(x, y, w, h);
    });

    ui.on_drumstick_changed(move |active| {
        set_drumstick_active(active);
    });

    ui.on_window_close(move || {
        std::process::exit(0);
    });

    let ui_weak_min = ui.as_weak();
    ui.on_minimize(move || {
        if let Some(ui) = ui_weak_min.upgrade() {
            let window = ui.window();
            window.set_minimized(true);
        }
    });

    // Suporte a arrastar e soltar arquivos do Sistema Operacional (OS Drag & Drop)
    {
        use i_slint_backend_winit::WinitWindowAccessor;
        let ui_weak = ui.as_weak();
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        
        // Rastreia a última posição física do cursor
        let last_phys_pos = Rc::new(RefCell::new((0.0, 0.0)));
        let pos_tracker = last_phys_pos.clone();
        
        ui.window().on_winit_window_event(move |_win, event| {
            use i_slint_backend_winit::winit::event::WindowEvent;
            match event {
                WindowEvent::CursorMoved { position, .. } => {
                    *pos_tracker.borrow_mut() = (position.x, position.y);
                }
                WindowEvent::DroppedFile(path) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        let ext = path.extension()
                            .and_then(|ext| ext.to_str())
                            .unwrap_or("")
                            .to_uppercase();
                        let supported = matches!(ext.as_str(), "WAV" | "MP3" | "FLAC" | "OGG" | "OGA");
                        if supported {
                            let (phys_x, phys_y) = *last_phys_pos.borrow();
                            let scale_factor = ui.window().scale_factor();
                            let logical_x = phys_x as f32 / scale_factor;
                            let logical_y = phys_y as f32 / scale_factor;

                            let coords = ui.invoke_map_window_coords_to_timeline(logical_x, logical_y);

                            let track_id = coords.track.max(0) as u64;
                            let start = coords.beat.round().max(0.0) as u32;
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Audio Clip");

                            // Decodifica ANTES de adicionar o clip.
                            let file_path = path.to_string_lossy().to_string();
                            let peaks: Vec<f32> = match decode_audio(&path) {
                                Ok(wav_data) => {
                                    waveform_points(&wav_data, 200)
                                }
                                Err(e) => {
                                    eprintln!("[waveform] failed to decode {}: {}", name, e);
                                    Vec::new()
                                }
                            };

                            let mut proj = project.borrow_mut();
                            if proj.track(track_id).is_some() {
                                let clip_id = proj.add_clip_with_path(
                                    track_id, ClipKind::Audio, name, start, 4,
                                    Some(file_path),
                                );
                                drop(proj);

                                // Após add_clip_with_path, o ClipAdded já disparou
                                // e to_clip_data leu o cache (ainda vazio).
                                // Agora inserimos no cache E atualizamos o modelo diretamente.
                                if let Some(id) = clip_id {
                                    if !peaks.is_empty() {
                                        waveforms.borrow_mut().insert(id, peaks.clone());

                                        // Atualiza o ClipData no modelo da UI com os picos.
                                        let wf = waveforms.borrow();
                                        for i in 0..clips_model.row_count() {
                                            if let Some(data) = clips_model.row_data(i) {
                                                if data.id == id as i32 {
                                                    clips_model.set_row_data(i, to_clip_data_from_id(id, &data, &wf));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            i_slint_backend_winit::EventResult::Propagate
        });
    }

    ui.run()
}