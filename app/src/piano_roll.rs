//! Piano roll: modelos derivados do core (grid, réguas) e
//! wiring das ações de edição de notas.

use slint::{Global, Model};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{PianoNote, PianoRollActions, PianoRollData, RulerTick};
use daw_core::timeline::Grid;
use daw_midi::{Note, NoteEditor};

/// Editor de notas do piano roll (sem suporte a clip ainda: lista global
/// da janela). A fonte da verdade vive no core (`daw_midi::NoteEditor`);
/// o modelo Slint é re-sincronizado a cada edição.

/// Converte uma nota do core para o DTO da camada Slint.
pub fn to_piano_note(note: &Note) -> PianoNote {
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
pub fn refresh_notes<C: slint::ComponentHandle>(ui: &C, editor: &RefCell<NoteEditor>)
where
    for<'a> PianoRollData<'a>: slint::Global<'a, C>,
{
    let notes: Vec<PianoNote> = editor.borrow().notes().iter().map(to_piano_note).collect();
    PianoRollData::get(ui).set_notes(Rc::new(slint::VecModel::from(notes)).into());
}

/// Semeia os globals do piano roll e assina as ações de edição de notas.
pub fn wire_piano_roll<C: slint::ComponentHandle + 'static>(ui: &C, editor: Rc<RefCell<NoteEditor>>)
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
                PianoRollData::get(&ui)
                    .set_selected_notes(Rc::new(slint::VecModel::from(vec![id])).into());
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        actions.on_note_selection_toggled(move |id| {
            if let Some(ui) = ui_weak.upgrade() {
                let data = PianoRollData::get(&ui);
                let mut ids: Vec<i32> = data.get_selected_notes().iter().collect();
                if let Some(pos) = ids.iter().position(|selected| *selected == id) {
                    ids.remove(pos);
                } else {
                    ids.push(id);
                }
                data.set_selected_notes(Rc::new(slint::VecModel::from(ids.clone())).into());
                data.set_selected_note(ids.first().copied().unwrap_or(-1));
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
                    PianoRollData::get(&ui).set_selected_notes(
                        Rc::new(slint::VecModel::from(Vec::<i32>::new())).into(),
                    );
                    refresh_notes(&ui, &editor);
                }
            }
        });
    }
    {
        let editor = editor.clone();
        let ui_weak = ui.as_weak();
        actions.on_lasso_selection_requested(
            move |x1, y1, x2, y2, px_per_beat, note_height, last_note| {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                // Hit-test de seleção: vive no core (testável), a UI só projeta.
                let ids = editor.borrow().notes_in_rect(
                    x1,
                    y1,
                    x2,
                    y2,
                    px_per_beat,
                    note_height,
                    last_note,
                );
                let first = ids.first().copied().unwrap_or(-1);

                PianoRollData::get(&ui)
                    .set_selected_notes(Rc::new(slint::VecModel::from(ids)).into());
                PianoRollData::get(&ui).set_selected_note(first);
            },
        );
    }
}
