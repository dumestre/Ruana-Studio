//! Janelas flutuantes: mixer e sampler desacoplam para a própria janela.
//!
//! `install` cria os slots, espelha o estado `floated` e assina
//! todos os handlers de cada painel flutuante.
use crate::MainWindow;
use slint::ComponentHandle;

use std::cell::RefCell;
use std::rc::Rc;

use crate::FloatPanel;
use crate::icon;
use crate::importer::is_audio_path;
use crate::mixer::{MixerModels, apply_mixer_models, mixer_inserts_state};
use daw_core::mixer::MixerParam;
use daw_core::project::Project;

pub struct FloatHandles {
    pub apply_mixer_state: Rc<dyn Fn()>,
    pub fit_float_mixer_windows: Rc<dyn Fn()>,
}

pub fn install(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    mixer_models: Rc<RefCell<MixerModels>>,
    meter_models: Rc<crate::meters::MeterModels>,
    audio: crate::audio_bridge::AudioBridge,
) -> FloatHandles {
    // Slots das janelas flutuantes (0=Mixer, 1=Sampler).
    let float_panels: Rc<RefCell<[Option<FloatPanel>; 2]>> =
        Rc::new(RefCell::new([None, None]));

    // Espelha quais janelas estão abertas na propriedade `floated` da UI.
    let sync_floated = {
        let ui_weak = ui.as_weak();
        let panels = float_panels.clone();
        move || {
            let p = panels.borrow();
            let arr: Vec<bool> = vec![p[0].is_some(), p[1].is_some()];
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
        let meter_values_for_float = meter_models.values.clone();
        let meter_clipped_for_float = meter_models.clipped.clone();
        let audio_for_float = audio.clone();
        ui.on_float_toggle(move |tab| {
            let _ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let idx = tab as usize;
            if idx > 1 {
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
            // Meters compartilhados: o bombeamento escreve nos mesmos
            // `Rc<VecModel>` das outras janelas, então a barra do mixer
            // flutuante e a da timeline não podem divergir.
            p.set_channel_meters(meter_values_for_float.clone().into());
            p.set_channel_clipped(meter_clipped_for_float.clone().into());

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
                    let channel = channel.max(0) as usize;
                    let core_value = match param {
                        MixerParam::Volume => crate::meters::fader_pos_to_linear(value),
                        _ => value,
                    };
                    if project
                        .borrow_mut()
                        .set_mixer_param(channel, param, core_value)
                    {
                        mixer_models.borrow().update_channel(channel, param, value);
                        audio.mark_dirty();
                    }
                });
            }

            // Luz de clip na flutuante → apaga o latch daquele canal.
            {
                let audio = audio_for_float.clone();
                p.on_channel_clip_cleared(move |channel| {
                    audio.clear_meter_clip(channel.max(0) as usize);
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

            // OS drag & drop (Explorer do SO) na janela flutuante do
            // sampler: arquivos de áudio viram sample do sampler — o
            // mesmo fluxo do drop no painel principal. O estado do
            // sampler é espelhado para a janela flutuante via global.
            {
                use i_slint_backend_winit::WinitWindowAccessor;
                use i_slint_backend_winit::winit::event::WindowEvent;
                let ui_main = ui_weak.clone();
                let p_weak = p.as_weak();
                p.window().on_winit_window_event(move |_win, event| {
                    match event {
                        WindowEvent::HoveredFile(path) => {
                            if is_audio_path(&path) {
                                if let Some(panel) = p_weak.upgrade() {
                                    panel.set_sampler_drop_hover(true);
                                }
                            }
                        }
                        WindowEvent::HoveredFileCancelled => {
                            if let Some(panel) = p_weak.upgrade() {
                                panel.set_sampler_drop_hover(false);
                            }
                        }
                        WindowEvent::DroppedFile(path) => {
                            if let Some(panel) = p_weak.upgrade() {
                                panel.set_sampler_drop_hover(false);
                            }
                            if !is_audio_path(&path) {
                                return i_slint_backend_winit::EventResult::Propagate;
                            }
                            if let Some(ui) = ui_main.upgrade() {
                                let name = path
                                    .file_stem()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("Audio")
                                    .to_string();
                                ui.invoke_sampler_sample_dropped(
                                    name.into(),
                                    path.to_string_lossy().to_string().into(),
                                );
                            }
                        }
                        _ => {}
                    }
                    i_slint_backend_winit::EventResult::Propagate
                });
            }

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

/// Modo "sem janelas flutuantes" (arquitetura do AGENTS.md): a UI é composta
/// como overlay offscreen numa única janela winit/wgpu. `install_no_floats`
/// entrega os mesmos [`FloatHandles`] que os módulos esperam, mas sem criar
/// janelas — `apply_mixer_state` espelha no painel principal e
/// `fit_float_mixer_windows` é no-op.
///
/// Os medidores NÃO precisam de nada aqui: os `Rc<VecModel>` são
/// publicados uma vez na janela principal e o `MeterPump` os redimensiona
/// in-place quando o número de trilhas muda, então a timeline enxerga a
/// mudança de comprimento sem re-publicar.
pub fn install_no_floats(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    mixer_models: Rc<RefCell<MixerModels>>,
    _audio: crate::audio_bridge::AudioBridge,
) -> FloatHandles {
    // Nenhum float: todos os slots fechados para sempre.
    let sync_floated = {
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_floated(Rc::new(slint::VecModel::from(vec![false, false])).into());
            }
        }
    };
    sync_floated();

    // Reaplica o estado do mixer no painel principal a partir do core
    // (mesma lógica do `install`, sem a projeção nos slots flutuantes).
    let apply_mixer_state = {
        let project = project.clone();
        let ui_weak = ui.as_weak();
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
        }
    };

    // `fit_float_mixer_windows`: sem janelas para encaixar.
    let fit_float_mixer_windows = { move || {} };

    FloatHandles {
        apply_mixer_state: Rc::new(apply_mixer_state.clone()),
        fit_float_mixer_windows: Rc::new(fit_float_mixer_windows.clone()),
    }
}
