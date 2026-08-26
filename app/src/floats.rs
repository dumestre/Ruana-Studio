//! Janelas flutuantes: mixer, sampler e piano roll desacoplam
//! para a própria janela, várias ao mesmo tempo.
//!
//! `install` cria os slots, espelha o estado `floated` e assina
//! todos os handlers de cada painel flutuante.
use crate::MainWindow;
use slint::ComponentHandle;

use std::cell::RefCell;
use std::rc::Rc;

use crate::FloatPanel;
use crate::icon;
use crate::mixer::{MixerModels, apply_mixer_models, mixer_inserts_state};
use crate::piano_roll::wire_piano_roll;
use daw_core::mixer::MixerParam;
use daw_core::project::Project;
use daw_midi::NoteEditor;

pub struct FloatHandles {
    pub apply_mixer_state: Rc<dyn Fn()>,
    pub fit_float_mixer_windows: Rc<dyn Fn()>,
}

pub fn install(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    mixer_models: Rc<RefCell<MixerModels>>,
    meter_model: Rc<slint::VecModel<f32>>,
    audio: crate::audio_bridge::AudioBridge,
) -> FloatHandles {
    // Slots das janelas flutuantes, um por tool (0=Mixer, 1=Sampler, 2=Piano Roll).
    let float_panels: Rc<RefCell<[Option<FloatPanel>; 3]>> =
        Rc::new(RefCell::new([None, None, None]));

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
            let (insert_counts, insert_names, insert_enabled, max_inserts) =
                mixer_inserts_state(&proj);
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
                    let w = ((proj.tracks.len() + 1) as f32 * 105.0 * slot.window().scale_factor())
                        as u32;
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
                    project
                        .borrow_mut()
                        .remove_mixer_insert(channel as usize, slot as usize);
                    apply_mixer();
                    fit();
                });
            }
            {
                let project = project.clone();
                let apply_mixer = apply_mixer.clone();
                p.on_mixer_insert_toggled(move |channel, slot| {
                    project
                        .borrow_mut()
                        .toggle_mixer_insert(channel as usize, slot as usize);
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
                            let new_h =
                                ((h0 as f32 + (y - y0) * sf).clamp(240.0 * sf, 640.0 * sf)) as u32;
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

    FloatHandles {
        apply_mixer_state: Rc::new(apply_mixer_state.clone()),
        fit_float_mixer_windows: Rc::new(fit_float_mixer_windows.clone()),
    }
}
