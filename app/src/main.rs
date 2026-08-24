slint::include_modules!();

mod audio_bridge;
mod beat;
mod cursors;
mod icon;
mod playback;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::cursors::{init_cursor_manager, set_cursor_for_tool, set_cursor_window_handle, set_timeline_bounds, reset_cursor_to_default};
use daw_core::clip::{Clip, ClipKind};
use daw_core::mixer::MixerParam;
use daw_core::project::{ChangeKind, DocumentEvent, Project};
use daw_core::timeline::Grid;
use daw_core::track::TrackKind;
use daw_core::transport::Transport;
use daw_media::{filesystem::default_samples_dir, filesystem::list_directory};
use daw_media::waveform::{decode_audio, waveform_points};
use daw_media::waveform_image::waveform_image_rgba;
use daw_midi::{Note, NoteEditor};
use slint::Model;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Máximo de slots de insert renderizados por canal no mixer.
const MAX_INSERTS: usize = 8;

/// Cache de waveform: clip_id → (picos, imagem RGBA pré-renderizada).
///
/// A imagem é gerada UMA vez por import (tiny-skia) e a UI desenha
/// um único `Image` em vez de ~1000 retângulos por clip.
type WaveformCache = std::collections::HashMap<u64, (Vec<f32>, Option<Vec<u8>>)>;

/// Renderiza uma waveform no tamanho EXATO pedido (px de tela).
///
/// É o que as DAWs fazem no zoom: re-rasterizar para a resolução
/// real de exibição, nunca esticar um bitmap pequeno.
fn render_waveform_at(peaks: &[f32], width_px: u32) -> Option<slint::Image> {
    use daw_media::waveform_image::waveform_image_rgba_at;
    let rgba = waveform_image_rgba_at(peaks, width_px)?;
    Some(waveform_image_to_slint(Some(rgba)))
}

/// Compassos necessários para caber até `end_beat`: teto em
/// compassos + 8 de folga, mínimo 32. Cresce monotonicamente
/// (undo não encolhe a timeline — comportamento padrão de DAW).
fn bars_for_end_beat(end_beat: f64, beats_per_bar: i32) -> i32 {
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
        DocumentEvent::ClipsGlued { left_before, right, .. } => {
            Some(both(left_before, right))
        }
        DocumentEvent::ClipResized { old, new } => Some(both(old, new)),
        _ => None,
    }
}

/// Mapeia um clip do core para o DTO da camada Slint.
fn to_clip_data(clip: &Clip, waveforms: &WaveformCache) -> ClipData {
    let (peaks, img) = match waveforms.get(&clip.id) {
        Some((p, i)) => (p.clone(), i.clone()),
        None => (Vec::new(), None),
    };
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat,
        len: clip.length_beats,
        is_midi: clip.is_midi(),
        color: clip.color as i32,
        waveform_points: Rc::new(slint::VecModel::from(peaks)).into(),
        waveform_image: waveform_image_to_slint(img),
    }
}

/// Converte o RGBA cru do cache num `slint::Image`.
/// Largura 0 = sem imagem (fallback de retângulos na UI).
pub fn waveform_image_to_slint(rgba: Option<Vec<u8>>) -> slint::Image {
    use slint::Rgba8Pixel;
    match rgba {
        Some(data) => {
            let buf = slint::SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                &data,
                daw_media::waveform_image::WAVEFORM_IMG_WIDTH as u32,
                512,
            );
            slint::Image::from_rgba8(buf)
        }
        None => slint::Image::default(),
    }
}

/// Atualiza apenas os pontos/imagem de um ClipData existente a partir do cache.
fn to_clip_data_from_id(id: u64, base: &ClipData, waveforms: &WaveformCache) -> ClipData {
    let (peaks, img) = match waveforms.get(&id) {
        Some((p, i)) => (p.clone(), i.clone()),
        None => (Vec::new(), None),
    };
    ClipData {
        id: base.id,
        name: base.name.clone(),
        track_index: base.track_index,
        beat: base.beat,
        len: base.len,
        is_midi: base.is_midi,
        color: base.color,
        waveform_points: Rc::new(slint::VecModel::from(peaks)).into(),
        waveform_image: waveform_image_to_slint(img),
    }
}

/// Extensões de áudio que o `daw-media` sabe decodificar.
const AUDIO_EXTS: [&str; 5] = ["WAV", "MP3", "FLAC", "OGG", "OGA"];

/// O arquivo é um áudio importável?
fn is_audio_path(path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();
    AUDIO_EXTS.contains(&ext.as_str())
}

/// Quantos beats um trecho de `seconds` ocupa a `bpm`.
///
/// É isto que impede a waveform de ser achatada: o clip passa a ter o
/// comprimento real do arquivo em vez de um valor fixo em que o
/// desenho tinha que caber.
fn beats_for_seconds(seconds: f32, bpm: f32) -> f32 {
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };
    (seconds * bpm / 60.0).max(0.01)
}

/// Importa um arquivo de áudio para a timeline.
///
/// Caminho ÚNICO de importação: o drop do explorador interno e o drop
/// do Sistema Operacional passam os dois por aqui, então as duas rotas
/// ganham a mesma duração real, o mesmo waveform e a mesma garantia de
/// não empilhar clip em cima de clip.
///
/// Retorna o id do clip criado, ou `None` se o arquivo não decodificou
/// / não havia trilha com espaço.
fn import_audio_file(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<ClipData>,
) -> Option<u64> {
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();

    // Decodifica ANTES de tocar no documento: sem duração real não há
    // como saber o comprimento do clip, e um arquivo quebrado não deve
    // deixar um clip morto na timeline.
    let decoded = match decode_audio(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("[import] {} não decodificou: {err}", name);
            return None;
        }
    };
    let length = beats_for_seconds(decoded.duration_seconds, bpm);

    // Resolução proporcional à duração: com um número fixo de pontos,
    // um loop de 1 s e um take de 3 min viravam o mesmo borrão. O teto
    // existe porque cada ponto é um retângulo no desenho do clip.
    // Resolução alta: com poucos pontos, um take denso virava
    // um tijolo cheio sem forma. O teto existe porque cada
    // ponto é um retângulo no desenho do clip.
    let points = ((length * 24.0) as usize).clamp(256, 2048);
    let peaks = waveform_points(&decoded, points);
    // Pré-renderiza a waveform em RGBA (cor padrão de áudio) uma
    // única vez; a UI desenha um Image em vez de ~1000 retângulos.
    // Se o usuário trocar a cor do clip, re-renderizamos adiante
    // (ver on_clip_color_cycle_requested).
    let img = waveform_image_rgba(&peaks, 0, false);

    let clip_id = {
        let mut proj = project.borrow_mut();
        let preferred = track_hint.max(0) as u64;
        // Prefere a trilha onde o usuário soltou; se ela já está
        // ocupada naquele instante, tenta outra trilha antes de
        // empurrar no tempo — é o que uma DAW faz ao importar vários
        // arquivos de uma vez.
        let track_id = proj
            .track_with_room(preferred, beat_hint.max(0.0), length)
            .or_else(|| proj.track(preferred).map(|t| t.id))?;
        let start = proj.find_free_slot(track_id, beat_hint.max(0.0), length);
        proj.add_clip_with_path(
            track_id,
            ClipKind::Audio,
            name.as_str(),
            start,
            length,
            Some(path.to_string_lossy().to_string()),
        )?
    };

    // `ClipAdded` já disparou e leu o cache ainda vazio; preenche
    // agora (picos + imagem pré-renderizada) e reprojeta só a
    // linha desse clip.
    if !peaks.is_empty() {
        waveforms.borrow_mut().insert(clip_id, (peaks, img));
        let wf = waveforms.borrow();
        if let Some(i) = clip_row(clips_model, clip_id as i32) {
            if let Some(data) = clips_model.row_data(i) {
                clips_model.set_row_data(i, to_clip_data_from_id(clip_id, &data, &wf));
            }
        }
    }
    Some(clip_id)
}

/// Mostra um diretório no explorador.
///
/// `push_history` distingue navegar (empurra no histórico, cortando
/// o "avançar" pendente, como um navegador) de andar no histórico
/// (só troca o conteúdo; o índice é de quem chamou).
///
/// Retorna `false` se o caminho não é um diretório legível — assim o
/// botão de voltar não move o índice para uma pasta que sumiu.
fn show_explorer_dir(ui: &MainWindow, path: &std::path::Path, push_history: bool) -> bool {
    if !path.is_dir() {
        return false;
    }
    let text = slint::SharedString::from(path.to_string_lossy().as_ref());

    if push_history {
        let history: Vec<slint::SharedString> = ui.get_explorer_path_history().iter().collect();
        let idx = ui.get_explorer_history_index();
        // Corta o ramo à frente: navegar depois de voltar descarta o
        // "avançar", igual a um navegador.
        let keep = ((idx + 1).max(0) as usize).min(history.len());
        let mut new_history = history[..keep].to_vec();
        new_history.push(text.clone());
        ui.set_explorer_history_index(new_history.len() as i32 - 1);
        ui.set_explorer_path_history(Rc::new(slint::VecModel::from(new_history)).into());
    }

    ui.set_explorer_current_path(text);
    ui.set_explorer_can_go_up(path.parent().is_some_and(|p| p.is_dir()));
    let filter = ui.get_explorer_search_text();
    let entries = load_directory_entries(path, filter.as_str());
    ui.set_explorer_entries(Rc::new(slint::VecModel::from(entries)).into());
    true
}

/// Lista um diretório para o explorador.
///
/// `filter` vazio lista tudo; senão mantém só os nomes que contêm o
/// termo (case-insensitive). Diretórios sempre passam, para o usuário
/// continuar navegando enquanto filtra.
fn load_directory_entries(path: &std::path::Path, filter: &str) -> Vec<BrowserFileEntry> {
    let needle = filter.trim().to_lowercase();
    let Ok(entries) = list_directory(path) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| {
            needle.is_empty() || e.is_directory || e.name.to_lowercase().contains(&needle)
        })
        .map(|e| {
            let entry_type = if e.is_directory {
                "Folder".to_string()
            } else {
                let ext = e
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_uppercase();
                // Todo formato que o decoder aceita mostra a extensão —
                // é isso que habilita o arraste no `file_browser`.
                if AUDIO_EXTS.contains(&ext.as_str()) {
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
/// sincronizados de forma granular. `ui` é usado para crescer
/// `total-bars` quando um clip passa do fim atual da timeline.
fn bind_document_events(
    project: &Project,
    tracks_model: Rc<slint::VecModel<slint::SharedString>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    ui: slint::Weak<MainWindow>,
) {
    project.subscribe(move |event, kind| {
        let wf = waveforms.borrow();
        apply_change(&tracks_model, &clips_model, event, kind == ChangeKind::Undo, &wf);
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

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    init_cursor_manager().expect("Failed to initialize cursor manager");

    let window = ui.window();
    window.on_close_requested(move || {
        let _ = reset_cursor_to_default();
        slint::CloseRequestResponse::HideWindow
    });

    // Ícone da janela/barra de tarefas: aplica já na criação e de novo
    // após o event loop subir (a janela winit pode ser criada tardiamente).
    icon::apply(window);
    {
        let ui_weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
            if let Some(ui) = ui_weak.upgrade() {
                icon::apply(ui.window());
            }
        });
    }

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

    // Motor de áudio: abre UMA vez e fica aberto pela vida do app.
    //
    // Antes o stream nascia no play e morria no stop. Isso custava
    // ~100ms de handshake com o driver no início de cada play (o
    // primeiro compasso saía picado) e tornava impossível pré-escutar
    // sample com o transporte parado — que é justamente o que o
    // channel rack do FL Studio faz. Se o dispositivo não abrir, o
    // app continua rodando em silêncio em vez de morrer.
    let audio_engine = match daw_audio::AudioEngine::start(transport.clone()) {
        Ok(engine) => {
            let cfg = engine.output_config();
            eprintln!(
                "[audio] {} Hz, {} canais",
                cfg.sample_rate, cfg.channels
            );
            Some(engine)
        }
        Err(err) => {
            eprintln!("[audio] dispositivo indisponível: {err} — seguindo em silêncio");
            None
        }
    };
    let audio = audio_bridge::AudioBridge::new(audio_engine.as_ref().map(|e| e.handle()));

    // Qualquer evento de documento (clip criado, movido, cortado,
    // apagado, undo/redo) muda o que a thread RT precisa ouvir.
    // Assinar aqui é melhor que marcar sujo em cada call site: um
    // caminho novo de edição não pode nascer mudo por esquecimento.
    {
        let audio = audio.clone();
        project.borrow().subscribe(move |_event, _kind| {
            audio.mark_dirty();
        });
    }

    // Republica o projeto na thread RT quando algo muda (coalescido).
    let _audio_sync = audio_bridge::start_sync_timer(audio.clone(), project.clone());

    // Cache de waveforms: clip_id → picos normalizados.
    let waveforms: Rc<RefCell<WaveformCache>> = Rc::new(RefCell::new(HashMap::new()));

    let (tracks_model, clips_model, track_colors_model) =
        initial_bind(&ui, &project.borrow(), &waveforms.borrow());
    bind_document_events(
        &project.borrow(),
        tracks_model,
        clips_model.clone(),
        waveforms.clone(),
        ui.as_weak(),
    );

    // Métrica musical: espelha o core na UI (fonte única — sem
    // hardcodes nas telas) e dimensiona a timeline pelo projeto.
    {
        let proj = project.borrow();
        let (bpm, ppq, bpb) = {
            let t = transport.lock().unwrap();
            (t.bpm as i32, t.ppq as i32, t.beats_per_bar as i32)
        };
        ui.set_ppq(ppq);
        ui.set_beats_per_bar(bpb);
        ui.set_bpm(bpm);
        let end = proj
            .clips
            .iter()
            .map(|c| c.end_beat())
            .fold(0.0f32, f32::max);
        ui.set_total_bars(bars_for_end_beat(end as f64, bpb));
    }

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

    // Beat sequencer: projeção inicial + persistência no core.
    {
        let project = project.clone();
        beat::wire_beat(&ui, project, audio.clone());
    }

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
        let audio_for_float = audio.clone();
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
                let audio = audio_for_float.clone();
                p.on_mixer_param_changed(move |channel, param, value| {
                    let Some(param) = MixerParam::from_str(param.as_str()) else {
                        return;
                    };
                    let channel = channel as usize;
                    project.borrow_mut().set_mixer_param(channel, param, value);
                    mixer_models.borrow().update_channel(channel, param, value);
                    audio.mark_dirty();
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
            icon::apply(p.window());
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
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        let ui_weak = ui.as_weak();
        ui.on_clip_dropped(move |track, beat, name, is_midi, path| {
            let bpm = ui_weak.upgrade().map(|u| u.get_bpm() as f32).unwrap_or(120.0);

            // Arquivo real de áudio → mesma rota do drop do Sistema
            // Operacional: duração real, waveform e vaga livre.
            if !is_midi && !path.is_empty() {
                let file = std::path::PathBuf::from(path.as_str());
                if is_audio_path(&file) && file.is_file() {
                    import_audio_file(
                        &file,
                        track,
                        beat,
                        bpm,
                        &project,
                        &waveforms,
                        &clips_model,
                    );
                    return;
                }
            }

            // Sem arquivo (clip pool / MIDI): bloco padrão de 4 beats.
            let kind = if is_midi {
                ClipKind::Midi
            } else {
                ClipKind::Audio
            };
            let track_id = track.max(0) as u64;
            let mut proj = project.borrow_mut();
            if proj.track(track_id).is_some() {
                // Nunca empilha: se o beat pedido está ocupado, o clip
                // encosta depois do vizinho em vez de sumir embaixo dele.
                let start = proj.find_free_slot(track_id, beat.max(0.0), 4.0);
                proj.add_clip(track_id, kind, name.as_str(), start, 4.0);
            }
        });
    }

    // Re-renderiza a waveform de um clip na resolução exata de
    // exibição (zoom/resize mudaram a largura em px).
    //
    // DEBOUNCE: escrever no modelo dentro do próprio `changed
    // width` reconstrói a linha e redispara o handler (loop).
    // Guardamos o pedido e aplicamos num timer de 120 ms; se um
    // novo pedido chegar, reiniciamos o timer. Também ignoramos
    // larguras iguais à última renderizada.
    {
        use std::collections::HashMap;
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        let pending: Rc<RefCell<HashMap<i32, u32>>> = Rc::new(RefCell::new(HashMap::new()));
        let done_width: Rc<RefCell<HashMap<u64, u32>>> = Rc::new(RefCell::new(HashMap::new()));
        let timer: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));

        ui.global::<TimelineMetrics>().on_waveform_resolution_requested(move |id, width_px| {
            let id_u = id.max(0) as u64;
            let w = (width_px.round() as u32).clamp(16, 8192);

            // Já está nessa resolução? Nada a fazer.
            if done_width.borrow().get(&id_u) == Some(&w) {
                return;
            }
            pending.borrow_mut().insert(id, w);

            // Reinicia o timer (debounce do gesto de zoom).
            let t = slint::Timer::default();
            *timer.borrow_mut() = Some(t);
            if let Some(t) = timer.borrow().as_ref() {
                let pending = pending.clone();
                let done_width = done_width.clone();
                let waveforms = waveforms.clone();
                let clips_model = clips_model.clone();
                t.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(120),
                    move || {
                        let jobs: Vec<(i32, u32)> = pending.borrow_mut().drain().collect();
                        for (id, w) in jobs {
                            let id_u = id.max(0) as u64;
                            let peaks: Vec<f32> = {
                                let wf = waveforms.borrow();
                                wf.get(&id_u).map(|(p, _)| p.clone()).unwrap_or_default()
                            };
                            if peaks.is_empty() {
                                continue;
                            }
                            if let Some(img) = render_waveform_at(&peaks, w) {
                                if let Some(i) = clip_row(&clips_model, id) {
                                    if let Some(mut data) = clips_model.row_data(i) {
                                        data.waveform_image = img;
                                        clips_model.set_row_data(i, data);
                                        done_width.borrow_mut().insert(id_u, w);
                                    }
                                }
                            }
                        }
                    },
                );
            }
        });
    }

    // Adiciona tracks via botão "+" da timeline → core.
    {
        let project = project.clone();
        let apply_mixer = apply_mixer_state.clone();
        let fit_float_mixers = fit_float_mixer_windows.clone();
        let colors_model = track_colors_model.clone();
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
            colors_model.push(0);
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
            let at = clip.start_beat + position.max(0.0);
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
            let beat = (x as f32 + scroll_x as f32).max(0.0) / ppb;
            project
                .clip_at_beat(row as u64, beat)
                .map(|clip| clip.id as i32)
                .unwrap_or(-1)
        });
    }

    // ========================================================
    // MOVE / RESIZE DE CLIPS — COMMIT ÚNICO
    //
    // Todo o gesto acontece na UI (global ClipDragState em
    // puro Slint, sem roundtrips). O core só recebe UMA
    // mutação por gesto, aqui.
    // ========================================================

    // Mover clip: track e beat finais, aplicados uma única vez.
    {
        let project = project.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_move_committed(move |id, new_track_index, new_beat| {
            let num_tracks = project.borrow().tracks.len() as i32;
            if num_tracks == 0 {
                return;
            }

            // Track clampada ao número de tracks; beat exatamente como a
            // UI calculou — o core guarda beats fracionários, então não
            // existe mais divergência entre o visual e o documento.
            let track_index = new_track_index.clamp(0, num_tracks - 1);
            let beat = new_beat.max(0.0);

            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut clip) = clips_model.row_data(i) {
                    clip.track_index = track_index;
                    clip.beat = beat;
                    clips_model.set_row_data(i, clip);
                    project
                        .borrow_mut()
                        .move_clip(id as u64, track_index as u64, beat);
                }
            }
        });
    }

    // Resize de clip: len (e beat, na borda esquerda) finais,
    // aplicados uma única vez.
    {
        let project = project.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_resize_committed(move |id, side, new_len, new_beat| {
            let len = new_len.max(0.25);
            let beat = new_beat.max(0.0);

            if let Some(i) = clip_row(&clips_model, id) {
                if let Some(mut clip) = clips_model.row_data(i) {
                    let start = if side < 0 { beat } else { clip.beat };
                    if side < 0 {
                        clip.beat = beat;
                    }
                    clip.len = len;
                    clips_model.set_row_data(i, clip);
                    project.borrow_mut().resize_clip(id as u64, start, len);
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

    // Ciclo de cor do clip (swatch click) → core.
    // Paleta de 20 cores (ver ui/clip_palette.slint).
    {
        let project = project.clone();
        ui.on_clip_color_cycle_requested(move |id| {
            let new_color = {
                let proj = project.borrow();
                let current = proj.clips.iter().find(|c| c.id as i32 == id).map(|c| c.color).unwrap_or(0);
                (current + 1) % 20
            };
            project.borrow_mut().set_clip_color(id as u64, new_color);
        });
    }

    // Botão "add color" do cabeçalho TRACKS → aplica o índice de cor
    // escolhido à track (cabeçalho) e a TODOS os seus clips.
    {
        let project = project.clone();
        let colors_model = track_colors_model.clone();
        ui.on_track_recolor_requested(move |track_index, color| {
            let mut proj = project.borrow_mut();
            if proj.tracks.is_empty() {
                return;
            }
            let idx = (track_index.max(0) as usize).min(proj.tracks.len() - 1);
            let track_id = proj.tracks[idx].id;
            let color = color.clamp(0, 255) as u8;

            // A cor da track em si (indicador do cabeçalho).
            proj.tracks[idx].color = color;
            colors_model.set_row_data(idx, color as i32);

            // E todos os clips dela.
            let ids: Vec<u64> = proj
                .clips
                .iter()
                .filter(|c| c.track_id == track_id)
                .map(|c| c.id)
                .collect();
            for id in ids {
                proj.set_clip_color(id, color);
            }
        });
    }

    // Clipboard interno de clips: clique esquerdo copia, clique em
    // área vazia da timeline cola uma cópia na posição clicada.
    let clip_clipboard: Rc<RefCell<Option<Clip>>> = Rc::new(RefCell::new(None));
    {
        let project = project.clone();
        let clipboard = clip_clipboard.clone();
        ui.on_clip_copy_requested(move |id| {
            if let Some(clip) = project.borrow().clips.iter().find(|c| c.id as i32 == id) {
                *clipboard.borrow_mut() = Some(clip.clone());
            }
        });
    }
    {
        let project = project.clone();
        let clipboard = clip_clipboard.clone();
        let ui_paste = ui.as_weak();
        ui.on_clip_paste_requested(move |beat, track| {
            let Some(src) = clipboard.borrow().clone() else {
                return;
            };
            // Snap ON → cola no início da célula; OFF → onde o mouse clicou.
            let snap = ui_paste.upgrade().map(|u| u.get_snap_enabled()).unwrap_or(true);
            let mut proj = project.borrow_mut();
            let track_count = proj.tracks.len().max(1);
            let track_id = (track.max(0) as usize).min(track_count - 1) as u64;
            let desired = if snap { beat.floor() } else { beat }.max(0.0);
            let start = proj.find_free_slot(track_id, desired, src.length_beats);
            proj.add_clip_with_path(
                track_id,
                src.kind,
                src.name.as_str(),
                start,
                src.length_beats,
                src.file_path.clone(),
            );
        });
    }

    // Duplicate de clip (menu de contexto) → cópia na mesma posição.
    {
        let project = project.clone();
        ui.on_clip_duplicate_requested(move |id| {
            let Some(src) = project
                .borrow()
                .clips
                .iter()
                .find(|c| c.id as i32 == id)
                .cloned()
            else {
                return;
            };
            let mut proj = project.borrow_mut();
            // Duplicar coloca a cópia DEPOIS do original, encostada.
            // Na mesma posição ela ficaria invisível e inaudível.
            let start = proj.find_free_slot(src.track_id, src.end_beat(), src.length_beats);
            proj.add_clip_with_path(
                src.track_id,
                src.kind,
                src.name.as_str(),
                start,
                src.length_beats,
                src.file_path.clone(),
            );
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
        let audio = audio.clone();
        ui.on_mixer_param_changed(move |channel, param, value| {
            let Some(param) = MixerParam::from_str(param.as_str()) else {
                return;
            };
            let channel = channel as usize;
            project.borrow_mut().set_mixer_param(channel, param, value);
            mixer_models.borrow().update_channel(channel, param, value);
            audio.mark_dirty();
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
    //
    // Uma função só faz o trabalho ("mostre esta pasta"); o que muda
    // entre os handlers é apenas se o histórico avança ou não.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_path_changed(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                show_explorer_dir(&ui, &PathBuf::from(path.as_str()), true);
            }
        });
    }

    // Filesystem: entrar em diretório (duplo papel do path-changed,
    // mantido separado porque a UI distingue clique em pasta de
    // navegação por caminho).
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_directory_entered(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                show_explorer_dir(&ui, &PathBuf::from(path.as_str()), true);
            }
        });
    }

    // Filesystem: subir um nível.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_go_up(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current = PathBuf::from(ui.get_explorer_current_path().as_str());
                if let Some(parent) = current.parent() {
                    show_explorer_dir(&ui, &parent.to_path_buf(), true);
                }
            }
        });
    }

    // Filesystem: voltar / avançar.
    //
    // Anda NO histórico em vez de empurrar nele — empurrar era o que
    // fazia o "voltar" virar um caminho novo e travar o botão.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_history_go(move |delta| {
            if let Some(ui) = ui_weak.upgrade() {
                let history: Vec<slint::SharedString> =
                    ui.get_explorer_path_history().iter().collect();
                let target = ui.get_explorer_history_index() + delta;
                if target < 0 || target as usize >= history.len() {
                    return;
                }
                let path = PathBuf::from(history[target as usize].as_str());
                if show_explorer_dir(&ui, &path, false) {
                    ui.set_explorer_history_index(target);
                }
            }
        });
    }

    // Filesystem: filtro de busca — relê a pasta atual.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_search_changed(move |term| {
            if let Some(ui) = ui_weak.upgrade() {
                let current = PathBuf::from(ui.get_explorer_current_path().as_str());
                let entries = load_directory_entries(&current, term.as_str());
                ui.set_explorer_entries(Rc::new(slint::VecModel::from(entries)).into());
            }
        });
    }


    // Playback/seek/meters: módulo dedicado (app/src/playback.rs).
    playback::install(
        &ui.as_weak(),
        transport.clone(),
        project.clone(),
        meter_model.clone(),
        meter_state.clone(),
        audio.clone(),
    );


    ui.on_tool_changed(move |tool| {
        let _ = set_cursor_for_tool(tool);
    });

    ui.on_timeline_bounds_changed(move |x, y, w, h| {
        set_timeline_bounds(x, y, w, h);
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

        ui.window().on_winit_window_event(move |_win, event| {
            use i_slint_backend_winit::winit::event::WindowEvent;
            match event {
                WindowEvent::DroppedFile(path) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        if is_audio_path(&path) {
                            // Pergunta ao sistema onde o cursor ESTÁ
                            // agora. Durante um drag OLE do Windows os
                            // eventos CursorMoved não chegam, então a
                            // última posição rastreada ficava velha e o
                            // clip caía em outra track.
                            #[cfg(windows)]
                            let (phys_x, phys_y) = {
                                use windows::Win32::Foundation::POINT;
                                use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
                                let mut pt = POINT { x: 0, y: 0 };
                                unsafe { let _ = GetCursorPos(&mut pt); }
                                (pt.x as f64, pt.y as f64)
                            };
                            #[cfg(not(windows))]
                            let (phys_x, phys_y) = (0.0, 0.0);

                            // Coordenadas físicas de TELA → lógicas da
                            // janela: desconta a origem da janela antes
                            // do scale factor.
                            let scale_factor = ui.window().scale_factor();
                            let win_pos = ui.window().position();
                            let logical_x =
                                ((phys_x - win_pos.x as f64) / scale_factor as f64) as f32;
                            let logical_y =
                                ((phys_y - win_pos.y as f64) / scale_factor as f64) as f32;

                            let coords = ui.invoke_map_window_coords_to_timeline(logical_x, logical_y);

                            import_audio_file(
                                &path,
                                coords.track,
                                coords.beat,
                                ui.get_bpm() as f32,
                                &project,
                                &waveforms,
                                &clips_model,
                            );
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
