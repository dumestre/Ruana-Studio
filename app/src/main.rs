slint::include_modules!();

mod cursors;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::cursors::{init_cursor_manager, set_cursor_for_tool, set_cursor_window_handle, reset_cursor_to_default};
use daw_core::clip::{Clip, ClipKind};
use daw_core::mixer::MixerParam;
use daw_core::project::{ChangeKind, DocumentEvent, Project};
use daw_core::track::TrackKind;
use daw_core::transport::Transport;
use daw_media::{filesystem::default_samples_dir, filesystem::list_directory};
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

/// Mapeia um clip do core para o DTO da camada Slint.
fn to_clip_data(clip: &Clip) -> ClipData {
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat as f32,
        len: clip.length_beats as f32,
        is_midi: clip.is_midi(),
    }
}

/// Editor de notas do piano roll (sem suporte a clip ainda: lista global
/// da janela). O Rust é a fonte da verdade; o modelo Slint é re-sincronizado
/// a cada edição.
struct PianoRollEditor {
    notes: Vec<PianoNote>,
    next_id: i64,
}

impl PianoRollEditor {
    fn new() -> Self {
        Self {
            notes: Vec::new(),
            next_id: 1,
        }
    }

    fn add(&mut self, note: i32, start: f32, length: f32, velocity: i32) {
        self.notes.push(PianoNote {
            id: self.next_id as i32,
            note,
            start,
            length,
            velocity,
        });
        self.next_id += 1;
    }

    fn move_note(&mut self, id: i32, start: f32) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.start = start;
        }
    }

    fn resize_note(&mut self, id: i32, length: f32) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.length = length;
        }
    }

    fn delete(&mut self, id: i32) {
        self.notes.retain(|n| n.id != id);
    }
}

/// Modelos derivados de configuração do piano roll: grids, oitavas, réguas
/// e ticks. Tudo calculado no Rust — nada de arrays hardcoded na UI.
fn piano_roll_models(
    first: i32,
    last: i32,
    bars: i32,
) -> (Vec<i32>, Vec<i32>, Vec<i32>, Vec<i32>, Vec<RulerTick>) {
    let note_lines: Vec<i32> = (0..=(last - first)).collect();
    let beat_lines: Vec<i32> = (0..(bars * 16)).collect();
    let octave_range: Vec<i32> = (first / 12..=last / 12).rev().collect();
    let bar_range: Vec<i32> = (0..bars).collect();

    let mut ticks = Vec::new();
    for bar in 0..bars {
        ticks.push(RulerTick {
            bar,
            pos: 0.0,
            level: 0,
        });
        for (pos, level) in [
            (0.25, 1),
            (0.50, 1),
            (0.75, 1),
            (0.125, 2),
            (0.375, 2),
            (0.625, 2),
            (0.875, 2),
            (0.0625, 3),
            (0.1875, 3),
            (0.3125, 3),
            (0.4375, 3),
            (0.5625, 3),
            (0.6875, 3),
            (0.8125, 3),
            (0.9375, 3),
        ] {
            ticks.push(RulerTick { bar, pos, level });
        }
    }
    (note_lines, beat_lines, octave_range, bar_range, ticks)
}

/// Re-sincroniza o modelo de notas no global da janela.
fn refresh_notes<C: slint::ComponentHandle>(ui: &C, editor: &RefCell<PianoRollEditor>)
where
    for<'a> PianoRollData<'a>: slint::Global<'a, C>,
{
    let notes = editor.borrow().notes.clone();
    PianoRollData::get(ui).set_notes(Rc::new(slint::VecModel::from(notes)).into());
}

/// Semeia os globals do piano roll e assina as ações de edição de notas.
fn wire_piano_roll<C: slint::ComponentHandle + 'static>(ui: &C, editor: Rc<RefCell<PianoRollEditor>>)
where
    for<'a> PianoRollData<'a>: slint::Global<'a, C>,
    for<'a> PianoRollActions<'a>: slint::Global<'a, C>,
{
    let (note_lines, beat_lines, octave_range, bar_range, ticks) = piano_roll_models(0, 127, 64);
    let data = PianoRollData::get(ui);
    data.set_first_note(0);
    data.set_last_note(127);
    data.set_note_lines(Rc::new(slint::VecModel::from(note_lines)).into());
    data.set_beat_lines(Rc::new(slint::VecModel::from(beat_lines)).into());
    data.set_octave_range(Rc::new(slint::VecModel::from(octave_range)).into());
    data.set_bar_range(Rc::new(slint::VecModel::from(bar_range)).into());
    data.set_ruler_ticks(Rc::new(slint::VecModel::from(ticks)).into());
    data.set_notes(Rc::new(slint::VecModel::from(Vec::<PianoNote>::new())).into());
    data.set_selected_note(-1);

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

/// Preenche os modelos da UI a partir do estado inicial do core.
///
/// Retorna os `VecModel` compartilhados para que o assinante de
/// eventos possa atualizá-los granularmente em runtime.
fn initial_bind(
    ui: &MainWindow,
    project: &Project,
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

    let clips: Vec<ClipData> = project.clips.iter().map(to_clip_data).collect();
    let clips_model = Rc::new(slint::VecModel::from(clips));
    ui.set_clips(clips_model.clone().into());

    // Mixer inicial
    let (mutes, soloes, arms, volumes, pans) = mixer_state(project);
    ui.set_mixer_mutes(Rc::new(slint::VecModel::from(mutes)).into());
    ui.set_mixer_soloes(Rc::new(slint::VecModel::from(soloes)).into());
    ui.set_mixer_arms(Rc::new(slint::VecModel::from(arms)).into());
    ui.set_mixer_volumes(Rc::new(slint::VecModel::from(volumes)).into());
    ui.set_mixer_pans(Rc::new(slint::VecModel::from(pans)).into());

    // Inserts iniciais
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

    (tracks_model, clips_model)
}

/// Carrega o diretório e atualiza o FileBrowser na UI.
fn load_directory(ui: &MainWindow, path: &Path) {
    match list_directory(path) {
        Ok(entries) => {
            let file_entries: Vec<slint::SharedString> = entries
                .iter()
                .map(|e| {
                    let entry_type = if e.is_directory { "Folder" } else { "File" };
                    format!("{}\t{}\t{}\t{}", e.name, entry_type, e.path.display(), e.is_directory)
                })
                .map(|s| s.into())
                .collect();
            // TODO: Atualizar o FileBrowser com entries
            // Por enquanto logamos
            println!("Loaded directory: {} ({} entries)", path.display(), entries.len());
        }
        Err(e) => {
            eprintln!("Failed to load directory {}: {}", path.display(), e);
        }
    }
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
                clips_model.push(to_clip_data(clip));
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
                    clips_model.set_row_data(i, to_clip_data(original));
                }
            } else {
                if let Some(i) = clip_row(clips_model, left.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left));
                }
                clips_model.push(to_clip_data(right));
            }
        }
        DocumentEvent::ClipMoved { old, new } => {
            let data = if undo { old } else { new };
            if let Some(i) = clip_row(clips_model, data.id as i32) {
                clips_model.set_row_data(i, to_clip_data(data));
            }
        }
        DocumentEvent::ClipDeleted(clip) => {
            if undo {
                clips_model.push(to_clip_data(clip));
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
                    clips_model.set_row_data(i, to_clip_data(left_before));
                }
                clips_model.push(to_clip_data(right));
            } else {
                if let Some(i) = clip_row(clips_model, left.id as i32) {
                    clips_model.set_row_data(i, to_clip_data(left));
                }
                if let Some(i) = clip_row(clips_model, right.id as i32) {
                    clips_model.remove(i);
                }
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
) {
    project.subscribe(move |event, kind| {
        apply_change(&tracks_model, &clips_model, event, kind == ChangeKind::Undo);
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
    let transport = Rc::new(RefCell::new(Transport::default()));

    let (tracks_model, clips_model) = initial_bind(&ui, &project.borrow());
    bind_document_events(&project.borrow(), tracks_model, clips_model.clone());

    // Piano roll da janela principal (estado por janela; cada float tem o seu).
    wire_piano_roll(&ui, Rc::new(RefCell::new(PianoRollEditor::new())));

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

    // Reaplica o estado do mixer nas janelas (principal + todas as flutuantes) a partir do core.
    let apply_mixer_state = {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        move || {
            let proj = project.borrow();
            let (mutes, soloes, arms, volumes, pans) = mixer_state(&proj);
            let mutes = Rc::new(slint::VecModel::from(mutes));
            let soloes = Rc::new(slint::VecModel::from(soloes));
            let arms = Rc::new(slint::VecModel::from(arms));
            let volumes = Rc::new(slint::VecModel::from(volumes));
            let pans = Rc::new(slint::VecModel::from(pans));
            let (insert_counts, insert_names, insert_enabled, max_inserts) = mixer_inserts_state(&proj);
            let insert_counts = Rc::new(slint::VecModel::from(insert_counts));
            let insert_names = Rc::new(slint::VecModel::from(insert_names));
            let insert_enabled = Rc::new(slint::VecModel::from(insert_enabled));
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_mixer_mutes(mutes.clone().into());
                ui.set_mixer_soloes(soloes.clone().into());
                ui.set_mixer_arms(arms.clone().into());
                ui.set_mixer_volumes(volumes.clone().into());
                ui.set_mixer_pans(pans.clone().into());
                ui.set_mixer_insert_counts(insert_counts.clone().into());
                ui.set_mixer_insert_names(insert_names.clone().into());
                ui.set_mixer_insert_enabled(insert_enabled.clone().into());
                ui.set_mixer_max_inserts(max_inserts);
            }
            let slots = panels.borrow();
            for slot in slots.iter().flatten() {
                slot.set_mixer_mutes(mutes.clone().into());
                slot.set_mixer_soloes(soloes.clone().into());
                slot.set_mixer_arms(arms.clone().into());
                slot.set_mixer_volumes(volumes.clone().into());
                slot.set_mixer_pans(pans.clone().into());
                slot.set_mixer_insert_counts(insert_counts.clone().into());
                slot.set_mixer_insert_names(insert_names.clone().into());
                slot.set_mixer_insert_enabled(insert_enabled.clone().into());
                slot.set_mixer_max_inserts(max_inserts);

                // Mixer flutuante: a largura acompanha o nº de canais.
                if slot.get_tool_tab() == 0 {
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
        ui.on_float_toggle(move |tab| {
            let ui = match ui_weak.upgrade() {
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
            p.set_mixer_meters(ui.get_mixer_meters());

            // Piano roll próprio da janela flutuante.
            wire_piano_roll(&p, Rc::new(RefCell::new(PianoRollEditor::new())));

            // Fecha via botão da própria janela flutuante.
            {
                let ui_weak = ui_weak.clone();
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
                let ui_weak = ui_weak.clone();
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

            // Ajustes feitos dentro da janela flutuante → core (reprojeta tudo).
            {
                let project = project.clone();
                let apply_mixer = apply_mixer.clone();
                p.on_mixer_param_changed(move |channel, param, value| {
                    let Some(param) = MixerParam::from_str(param.as_str()) else {
                        return;
                    };
                    project.borrow_mut().set_mixer_param(channel as usize, param, value);
                    apply_mixer();
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

    // Hit-test da ferramenta cola: devolve o id do clip sob o cursor
    // (linha da track + coordenadas com scroll), ou -1 se nenhum.
    {
        let project = project.clone();
        ui.on_glue_clip_at(move |row, x, scroll_x, px_per_beat| -> i32 {
            let project = project.borrow();
            let row = row.max(0.0) as i32;
            let x = x as f32;
            let sx = scroll_x as f32;
            let ppb = px_per_beat as f32;
            for clip in &project.clips {
                if clip.track_id as i32 == row {
                    let cx = clip.start_beat as f32 * ppb - sx;
                    let cw = clip.length_beats as f32 * ppb;
                    if x >= cx && x < cx + cw {
                        return clip.id as i32;
                    }
                }
            }
            -1
        });
    }

    // Mover clip via drag (ferramenta select).
    {
        let project = project.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_move_started(move |id, _beat| {
            // Armazena o beat original para referência visual
            let mut proj = project.borrow_mut();
            if let Some(clip) = proj.clip(id as u64) {
                // Armazena o beat original no campo `beat` do modelo para referência
                // O delta será aplicado visualmente no modelo
            }
        });
    }

    // Mover clip: delta durante o drag (feedback visual imediato)
    {
        let clips_model = clips_model.clone();
        ui.on_clip_move_delta(move |id, delta| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut clip) = clips_model.row_data(i) {
                    let new_beat = (clip.beat + delta).max(0.0);
                    clip.beat = new_beat;
                    clips_model.set_row_data(i, clip);
                }
            }
        });
    }

    // Mover clip: finalizado → persiste no core
    {
        let project = project.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_move_finished(move |id| {
            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(clip) = clips_model.row_data(i) {
                    let new_beat = clip.beat.round().max(0.0) as u32;
                    let track_id = clip.track_index as u64;
                    let mut proj = project.borrow_mut();
                    proj.move_clip(id as u64, track_id, new_beat);
                }
            }
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

    // Parâmetros do mixer → core.
    {
        let project = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        ui.on_mixer_param_changed(move |channel, param, value| {
            let Some(param) = MixerParam::from_str(param.as_str()) else {
                return;
            };
            project.borrow_mut().set_mixer_param(channel as usize, param, value);
            apply_mixer();
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
                    if let Ok(entries) = list_directory(&path) {
                        let file_entries: Vec<slint::SharedString> = entries
                            .iter()
                            .map(|e| {
                                let entry_type = if e.is_directory { "Folder" } else { "File" };
                                format!("{}\t{}\t{}\t{}", e.name, entry_type, e.path.display(), e.is_directory)
                            })
                            .map(|s| s.into())
                            .collect();
                        ui.set_explorer_entries(Rc::new(slint::VecModel::from(file_entries)).into());
}
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

                    if let Ok(entries) = list_directory(&path_buf) {
                        let file_entries: Vec<slint::SharedString> = entries
                            .iter()
                            .map(|e| {
                                let entry_type = if e.is_directory { "Folder" } else { "File" };
                                format!("{}\t{}\t{}\t{}", e.name, entry_type, e.path.display(), e.is_directory)
                            })
                            .map(|s| slint::SharedString::from(s.as_str()))
                            .collect();
                        ui.set_explorer_entries(Rc::new(slint::VecModel::from(file_entries)).into());
                    }
                }
            }
        });
    }

    let mut seed = 0x1234_5678_9abc_def0u64;

    // Playback: o playhead vive no Transport do core; o timer só
    // avança o core e projeta a posição na UI.
    {
        let ui_weak = ui.as_weak();
        let timer = Rc::new(RefCell::new(slint::Timer::default()));
        let timer_ref = timer.clone();
        let transport = transport.clone();
        ui.on_play_state_changed(move |playing| {
            transport.borrow_mut().toggle();
            if playing {
                let ui_weak = ui_weak.clone();
                let transport = transport.clone();
                timer_ref.borrow().start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(50),
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let mut t = transport.borrow_mut();
                            t.advance(48);
                            ui.set_transport_position(t.position_ticks as i32);
                        }
                    },
                );
            } else {
                timer_ref.borrow().stop();
            }
        });
    }

    // Meters animados (áudio falso) — só rodam enquanto "tocando".
    let meter_timer = Rc::new(slint::Timer::default());
    {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        let timer = meter_timer.clone();
        ui.on_play_state_changed(move |playing| {
            if playing {
                let project = project.clone();
                let ui_weak = ui_weak.clone();
                let panels = panels.clone();
                let timer = timer.clone();
                timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(80),
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let count = project.borrow().tracks.len() + 1;
                            let mut levels: Vec<f32> = Vec::new();
                            for _ in 0..count {
                                let r = next_random(&mut seed);
                                levels.push(r * r);
                            }
                            let model = Rc::new(slint::VecModel::from(levels));
                            ui.set_mixer_meters(model.clone().into());
                            let slots = panels.borrow();
                            for slot in slots.iter().flatten() {
                                slot.set_mixer_meters(model.clone().into());
                            }
                        }
                    },
                );
            } else {
                timer.stop();
            }
        });
    }

    ui.on_tool_changed(move |tool| {
        let _ = set_cursor_for_tool(tool);
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

    ui.run()
}