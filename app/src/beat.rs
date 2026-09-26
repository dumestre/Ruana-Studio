//! Bridge do beat sequencer: persistência no core.
//!
//! Core → UI: `refresh_beat` re-projeta patterns e parâmetros das
//! trilhas no global `BeatState`.
//!
//! UI → Core: os callbacks do global gravam no `Project` e disparam
//! `refresh_beat`, mantendo o core como fonte única de verdade.
//!
//! Áudio: toda mutação marca o snapshot como sujo (ver
//! [`crate::audio_bridge`]), e os gestos que o usuário espera ouvir —
//! soltar um sample, ligar um passo, clicar no botão da trilha —
//! disparam pré-escuta na hora, com o transporte parado ou não.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel};

use daw_core::project::Project;
use daw_core::track::TrackKind;
use daw_core::transport::Transport;
use daw_midi::{DrumPattern, DrumTrackParams};

use crate::audio_bridge::AudioBridge;
use crate::{BeatState, MainWindow};

/// Cache global (sessão) do SAMPLER por sample path.
///
/// Toda linha do beat com sample é um instrumento; o painel Sampler
/// mostra o instrumento da linha selecionada. Decodificar o arquivo a
/// cada clique de seleção seria lento — cacheamos aqui a "vista"
/// (nome/formato/duração/peaks), sempre derivada do MESMO decoder que
/// alimenta o engine.
struct SamplerViewData {
    name: String,
    format: String,
    duration: String,
}

static SAMPLE_VIEW_CACHE: std::sync::OnceLock<
    Mutex<HashMap<String, Arc<SamplerViewData>>>,
> = std::sync::OnceLock::new();

fn sample_cache() -> &'static Mutex<HashMap<String, Arc<SamplerViewData>>> {
    SAMPLE_VIEW_CACHE.get_or_init(Default::default)
}

/// Nome curto de um sample a partir do caminho.
fn sample_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Vista do sampler CACHEADA (sem decode, sem I/O).
fn sampler_view_cached(path: &str) -> Option<Arc<SamplerViewData>> {
    sample_cache()
        .lock()
        .ok()
        .and_then(|m| m.get(path).cloned())
}

/// Sincroniza o painel Sampler com a linha selecionada do beat.
///
/// Cada linha do beat com sample tem UM sampler: é este painel,
/// apontando para a linha selecionada. Chamado quando a seleção da
/// linha muda e no fim de `refresh_beat` (drop/clear refletem na hora).
fn sync_sampler_to_beat(ui: &MainWindow, project: &Project) {
    let index = ui.get_beat_selected_track().max(0) as usize;
    let path = project
        .current_beat_pattern()
        .and_then(|p| p.track_params.get(index))
        .and_then(|t| t.sample_path.clone());
    let Some(path) = path else {
        clear_sampler(ui);
        return;
    };
    // Já decodificado? Aplica na hora.
    if let Some(view) = sampler_view_cached(&path) {
        apply_sampler_view(ui, &view);
        return;
    }
    // Ainda não: mostra vazio e decodifica NA WORKER thread. Quando o
    // resultado chega (event loop), o painel é atualizado aqui mesmo.
    clear_sampler(ui);
    let ui_weak = ui.as_weak();
    let cache_path = path.clone();
    crate::decode_worker::DecodeWorker::default().schedule_decode(
        std::path::Path::new(&path),
        0.0,
        512,
        move |ready| {
            let Some(w) = ready else {
                return;
            };
            let entry = Arc::new(SamplerViewData {
                name: w.name,
                format: w.format,
                duration: format!("{:.2}s", w.duration_seconds),
            });
            if let Ok(mut map) = sample_cache().lock() {
                map.insert(cache_path, entry.clone());
            }
            if let Some(ui) = ui_weak.upgrade() {
                apply_sampler_view(&ui, &entry);
            }
        },
    );
}

/// Aplica uma vista decodificada nos models do painel Sampler.
fn apply_sampler_view(ui: &MainWindow, view: &SamplerViewData) {
    ui.set_sampler_sample_name(view.name.clone().into());
    ui.set_sampler_sample_loaded(true);
    ui.set_sampler_sample_format(view.format.clone().into());
    ui.set_sampler_sample_duration(view.duration.clone().into());
}

/// Limpa o painel Sampler (linha sem sample / sem padrão).
fn clear_sampler(ui: &MainWindow) {
    ui.set_sampler_sample_name(SharedString::default());
    ui.set_sampler_sample_loaded(false);
    ui.set_sampler_sample_format(SharedString::default());
    ui.set_sampler_sample_duration(SharedString::default());
}

/// Aplica uma mutação no projeto e re-projeta o beat na sequência.
///
/// O snapshot do áudio é marcado como sujo aqui: qualquer coisa que
/// muda o pattern muda o que a thread RT precisa ver.
fn commit(
    ui: &MainWindow,
    project: &RefCell<Project>,
    audio: &AudioBridge,
    f: impl FnOnce(&mut Project),
) {
    f(&mut project.borrow_mut());
    audio.mark_dirty();
    refresh_beat(ui, &project.borrow());
}

/// Re-projeta o estado do beat sequencer (core) nos models da UI.
pub fn refresh_beat(ui: &MainWindow, project: &Project) {
    let st = BeatState::get(ui);
    let bp = &project.beat_patterns;

    // patterns: cada trilha vira um array plano de bools
    // (índice global = track * steps + step).
    let rows: Vec<ModelRc<bool>> = bp
        .patterns
        .iter()
        .map(|p| Rc::new(VecModel::from(p.grid.concat())).into())
        .collect();
    st.set_patterns(Rc::new(VecModel::from(rows)).into());

    // Nomes dos patterns para o Clip Pool.
    let pattern_names: Vec<SharedString> =
        bp.patterns.iter().map(|p| p.name.clone().into()).collect();
    st.set_pattern_names(Rc::new(VecModel::from(pattern_names)).into());

    // Durações em beats (para o Clip Pool criar o clip com o
    // comprimento correspondente ao arrastar o padrão pra timeline).
    let pattern_durations: Vec<f32> =
        bp.patterns.iter().map(|p| p.length_beats()).collect();
    st.set_pattern_durations(Rc::new(VecModel::from(pattern_durations)).into());

    let Some(current) = bp.current_pattern() else {
        // Sem padrão → sampler limpo.
        sync_sampler_to_beat(ui, project);
        return;
    };

    st.set_current_pattern(bp.current as i32);
    st.set_grid_resolution(current.resolution as i32);
    st.set_humanize(current.humanize);
    st.set_swing(current.swing);

    let names: Vec<SharedString> = current
        .track_params
        .iter()
        .map(|t| t.name.clone().into())
        .collect();
    st.set_instrument_names(Rc::new(VecModel::from(names)).into());

    // Sample de cada linha: "" quando vazia. A linha precisa mostrar
    // que tem som carregado — renomear a trilha não é sinal suficiente
    // (uma linha renomeada à mão pareceria carregada).
    let samples: Vec<SharedString> = current
        .track_params
        .iter()
        .map(|t| {
            t.sample_path
                .as_deref()
                .map(sample_label)
                .unwrap_or_default()
                .into()
        })
        .collect();
    st.set_track_sample_names(Rc::new(VecModel::from(samples)).into());

    let active: Vec<bool> = current.track_params.iter().map(|t| t.active).collect();
    let solo: Vec<bool> = current.track_params.iter().map(|t| t.solo).collect();
    let pan: Vec<f32> = current.track_params.iter().map(|t| t.pan).collect();
    let volume: Vec<f32> = current.track_params.iter().map(|t| t.volume).collect();
    let channels: Vec<i32> = current
        .track_params
        .iter()
        .map(|t| t.mixer_channel)
        .collect();

    st.set_track_active(Rc::new(VecModel::from(active)).into());
    st.set_track_solo(Rc::new(VecModel::from(solo)).into());
    st.set_track_pan(Rc::new(VecModel::from(pan)).into());
    st.set_track_volume(Rc::new(VecModel::from(volume)).into());
    st.set_track_mixer_channel(Rc::new(VecModel::from(channels)).into());

    // Por fim, o SAMPLER da linha selecionada: qualquer mudança no
    // beat (drop de sample, clear, delete...) reflete na hora.
    sync_sampler_to_beat(ui, project);
}

/// Toca o sample de uma linha do pattern.
///
/// Respeita volume/pan da linha, então a pré-escuta soa como vai soar
/// no pattern — não em ganho unitário.
fn preview_row(project: &RefCell<Project>, audio: &AudioBridge, track: i32) {
    if track < 0 {
        return;
    }
    let proj = project.borrow();
    let Some(t) = proj
        .beat_patterns
        .current_pattern()
        .and_then(|p| p.track_params.get(track as usize))
    else {
        return;
    };
    let Some(path) = t.sample_path.as_deref() else {
        return;
    };
    audio.preview(path, t.volume, t.pan);
}

/// Tolerância para casar a nota pintada com a nota já existente.
///
/// Só existe porque as posições são `f32`: o mesmo passo pode ter sido
/// gravado num clip e agora ser re-calculado a partir de um playhead
/// ligeiramente diferente. Fica bem abaixo da grade mínima (1/16 =
/// 0.0625), então nunca casa com o passo vizinho.
const PAINT_EPS: f32 = 0.0005;

/// Trilha que recebe a pintura: a primeira de bateria, senão a primeira
/// que aceita MIDI. Se o projeto não tem nenhuma, cria uma — um padrão
/// sem track não tem onde morar, e o usuário não deve ter que montar a
/// trilha antes de conseguir programear.
fn paint_track_id(project: &mut Project) -> u64 {
    if let Some(t) = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Drum)
        .or_else(|| project.tracks.iter().find(|t| t.kind.is_midi()))
    {
        return t.id;
    }
    project.add_track(TrackKind::Drum, "Drums")
}

/// Compasso em que o pattern pintado mora, em beats.
///
/// O pattern entra na timeline sempre INTEIRO e alinhado ao compasso:
/// com o playhead no meio do compasso 2, um padrão de 4 beats ocupa
/// `2..6`, não `2.3..6.3`. É o que mantém o compasso musicalmente
/// fechado, e o que faz a pintura coincidir com o clip do Clip Pool.
fn paint_origin(playhead: f32, length_beats: f32) -> f32 {
    if length_beats <= 0.0 {
        return playhead.max(0.0);
    }
    (playhead / length_beats).floor() * length_beats
}

/// Duas notas ocupam a mesma célula do grid?
fn same_cell(note: daw_midi::Note, pitch: i32, start: f32) -> bool {
    note.pitch == pitch && (note.start - start).abs() <= PAINT_EPS
}

/// Pinta um passo do pattern na timeline — o modelo do channel rack do
/// FL Studio: ligar o passo GRAVA a nota no compasso do playhead,
/// desligar REMOVE.
///
/// Sem isto o pattern só existia dentro do beat sequencer, e como o
/// play da timeline silencia o step sequencer (ver `PlayMode::Timeline`),
/// a bateria não soaria na timeline. Pintada, ela vira clip MIDI de
/// verdade e o motor de áudio a toca pelo caminho normal dos clips
/// (`engine.rs` mapeia o pitch de volta pra linha do pattern).
///
/// O clip resultante é um MIDI comum e INDEPENDENTE do pattern: mudar o
/// swing ou a resolução depois não reescreve o que já foi gravado (como
/// no FL), e apagar a nota no piano roll não religa o passo.
///
/// O clip leva o NOME DO PADRÃO de propósito: é assim que o Clip Pool já
/// reconhece um clip de bateria (`timeline.rs`), então o clip pintado
/// passo a passo e o clip arrastado do pool são o mesmo tipo de objeto.
///
/// Devolve `true` se o documento mudou (algo foi pintado ou removido).
fn paint_step(
    proj: &mut Project,
    transport: &Arc<Mutex<Transport>>,
    row: usize,
    step: usize,
    on: bool,
) -> bool {
    let playhead = transport.lock().unwrap().position_beats();

    // Geometria da nota + grade do padrão. Usa `cell_note` (e não
    // `step_note`) porque DESLIGAR o passo também precisa saber onde a
    // nota estava. A conversão é a mesma do drop do Clip Pool, então
    // pintar e arrastar soam idênticos.
    let (clip_name, note, pattern_len) = {
        let Some(p) = proj.current_beat_pattern() else {
            return false;
        };
        let Some(note) = p.cell_note(row, step) else {
            return false;
        };
        (p.name.clone(), note, p.length_beats())
    };

    // `note.start` é a COLUNA do passo dentro do padrão (0..len), já
    // com o swing. A origem na timeline é o compasso do playhead.
    let origin = paint_origin(playhead, pattern_len);
    let len = pattern_len.max(0.25);
    let track_id = paint_track_id(proj);

    // Clip de pintura que comporta o compasso: MIDI, na trilha alvo, com
    // o nome do padrão e cobrindo a origem. Sem ele (primeira pintura, ou
    // o usuário arrastou o clip para outro compasso) criamos um.
    let existing = proj
        .clips
        .iter()
        .find(|c| {
            c.track_id == track_id
                && c.is_midi()
                && c.name == clip_name
                && c.start_beat <= origin
                && origin < c.start_beat + c.length_beats
        })
        .map(|c| c.id);

    // A nota é a COLUNA dentro do padrão, não o offset do playhead: o
    // pattern é uma peça inteira, então se `find_free_slot` empurrar o
    // clip para o próximo compasso o pattern inteiro vai junto, e a nota
    // continua valendo `origin + coluna` em termos absolutos. Usar
    // `pos - clip.start_beat` aqui daria posição NEGATIVA justamente no
    // caso do clip deslocado.
    let local = note.start;

    if let Some(id) = existing {
        // Clip já existe: edita as notas.
        let Some(clip) = proj.clip(id) else {
            return false;
        };
        // Já está no estado pedido? Então não é operação — repintar o
        // mesmo passo não deve gerar evento de documento (e um id de
        // nota novo) sem mudar nada.
        let painted = clip.notes.iter().any(|n| same_cell(*n, note.pitch, local));
        if painted == on {
            return false;
        }

        let mut notes = clip.notes.clone();
        // Casa por pitch + posição, nunca por id: o id é reatribuído a
        // cada `set_clip_notes` e não sobrevive entre rotas de escrita.
        notes.retain(|n| !same_cell(*n, note.pitch, local));

        if on {
            notes.push(daw_midi::Note {
                id: notes.iter().map(|n| n.id).max().unwrap_or(0) + 1,
                pitch: note.pitch,
                start: local,
                length: note.length,
                velocity: note.velocity,
            });
        }
        return proj.set_clip_notes(id, notes);
    }

    if !on {
        // Nunca pintou neste compasso e o passo foi desligado: não há o
        // que remover.
        return false;
    }

    // Primeira nota do compasso: cria o clip JÁ COM ela, num evento só,
    // para um clique = um Ctrl+Z. Nasce no início do compasso, não na
    // coluna: o swing desloca a NOTA, e um clip que começasse na coluna
    // cortaria o onset da última coluna da grade.
    let start = proj.find_free_slot(track_id, origin, len);
    proj.add_midi_clip_with_notes(
        track_id,
        &clip_name,
        start,
        len,
        vec![daw_midi::Note {
            id: 1,
            pitch: note.pitch,
            start: local,
            length: note.length,
            velocity: note.velocity,
        }],
    )
    .is_some()
}

/// Conecta todos os callbacks do beat sequencer ao core.
pub fn wire_beat(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    audio: AudioBridge,
    transport: Arc<Mutex<Transport>>,
) {
    let st = BeatState::get(ui);

    // Snapshot para Copy Beat / Copy Track (paste consumirá quando
    // a UI ganhar a ação).
    let beat_clipboard: Rc<RefCell<Option<DrumPattern>>> = Rc::new(RefCell::new(None));
    let track_clipboard: Rc<RefCell<Option<(Vec<bool>, DrumTrackParams)>>> =
        Rc::new(RefCell::new(None));

    // =========================================================
    // PASSOS
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        let transport = transport.clone();
        st.on_step_committed(move |index, value| {
            // `index` é a posição GLOBAL na grade achatada
            // (track * steps + step); a largura vem do padrão atual.
            let (row, step) = {
                let stride = project
                    .borrow()
                    .current_beat_pattern()
                    .map(|p| p.steps())
                    .unwrap_or(16)
                    .max(1);
                ((index as usize) / stride, (index as usize) % stride)
            };

            commit(&ui, &project, &audio, |proj| {
                proj.set_beat_step(row, step, value);
                // O passo não fica preso ao sequenciador: ele também
                // vira nota na timeline, no compasso do playhead. É o
                // que faz a bateria soar no play da timeline (ver
                // `paint_step`).
                //
                // Dentro do `commit` de propósito: o `mark_dirty` do
                // áudio vem DEPOIS, então o snapshot da thread RT já
                // enxerga o clip novo — pintando fora, o motor poderia
                // ler o projeto sem a nota até o próximo flush.
                paint_step(proj, &transport, row, step, value);
            });

            // Ligar um passo toca o som da linha na hora (FL Studio faz
            // isso). Desligar não toca nada. Durante a reprodução a
            // pré-escuta é DISPENSÁVEL: a nota acabou de ser pintada e
            // o motor dispara o sample no bloco atual — sem o corte, o
            // mesmo sample tocaria duas vezes.
            if value && !transport.lock().unwrap().playing {
                preview_row(&project, &audio, row as i32);
            }
        });
    }

    // =========================================================
    // PADRÃO ATUAL / RESOLUÇÃO / HUMANIZE / SWING
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_pattern_changed(move |index| {
            if index < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.select_beat_pattern(index as usize);
            });
        });
    }

    // =========================================================
    // SELECTOR DE LINHA → SAMPLER
    //
    // Cada linha do beat com sample tem UM sampler (o painel da aba
    // 1, apontando para a linha selecionada). Selecionar a linha
    // sincroniza o painel com o instrumento daquela track.
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_track_selected(move |index| {
            ui.set_beat_selected_track(index);
            sync_sampler_to_beat(&ui, &project.borrow());
        });
    }

    // Botão "SP" da linha: abre a aba do sampler mostrando a linha.
    {
        let ui = ui.clone_strong();
        let project = project.clone();
        st.on_sampler_open(move |index| {
            ui.set_beat_selected_track(index);
            sync_sampler_to_beat(&ui, &project.borrow());
            ui.set_bottom_panel_open(true);
            ui.set_bottom_active_tab(1);
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_grid_resolution_committed(move |resolution| {
            if resolution <= 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.set_beat_resolution(resolution as u32);
            });
        });
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_humanize_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.humanize = value.clamp(0.0, 1.0);
            }
            audio.mark_dirty();
        });
    }

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_swing_committed(move |value| {
            if let Some(p) = project.borrow_mut().current_beat_pattern_mut() {
                p.swing = value.clamp(0.0, 1.0);
            }
            audio.mark_dirty();
        });
    }

    // =========================================================
    // CRIAR NOVO PADRÃO (botão "+" do Clip Pool)
    //
    // Cria um padrão vazio no core, re-projeta e abre o popup do
    // beat — o usuário vê imediatamente onde editar o novo padrão.
    // =========================================================

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_create_pattern(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.create_beat_pattern();
            });
            ui.set_beat_open(true);
        });
    }

    // =========================================================
    // ABRE POPUP DO BEAT SEQUENCER (do duplo clique no Clip Pool)
    // =========================================================

    {
        let ui = ui.clone_strong();
        st.on_open_beat_popup(move || {
            ui.set_beat_open(true);
        });
    }

    // =========================================================
    // BEAT (padrão inteiro)
    // =========================================================

    {
        let clipboard = beat_clipboard.clone();
        let project = project.clone();
        st.on_copy_beat(move || {
            *clipboard.borrow_mut() = project.borrow().current_beat_pattern().cloned();
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_duplicate_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.duplicate_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_clear_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.clear_beat_pattern();
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_delete_beat(move || {
            commit(&ui, &project, &audio, |proj| {
                proj.delete_beat_pattern();
            });
        });
    }

    // =========================================================
    // TRILHAS
    // =========================================================

    {
        let clipboard = track_clipboard.clone();
        let project = project.clone();
        st.on_copy_track(move |track| {
            let proj = project.borrow();
            let i = track.max(0) as usize;
            if let Some(p) = proj.current_beat_pattern() {
                if i < p.num_tracks() {
                    *clipboard.borrow_mut() = Some((p.grid[i].clone(), p.track_params[i].clone()));
                }
            }
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_duplicate_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.duplicate_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_delete_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.delete_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_clear_track(move |track| {
            if track < 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.clear_beat_track(track as usize);
            });
        });
    }

    {
        let ui = ui.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        st.on_insert_pattern(move |_track, interval| {
            if interval <= 0 {
                return;
            }
            commit(&ui, &project, &audio, |proj| {
                proj.insert_beat_steps(interval as usize);
            });
        });
    }

    // =========================================================
    // PARÂMETROS DE TRILHA (active/solo/pan/volume/channel)
    //
    // Mudam continuamente (knobs), então gravam direto sem
    // re-projeção — o binding global ↔ popup já reflete na tela.
    // =========================================================

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_track_param_committed(move |track, param, value| {
            if track < 0 {
                return;
            }
            if let Some(t) = project.borrow_mut().beat_track_params_mut(track as usize) {
                match param.as_str() {
                    "active" => t.active = value >= 0.5,
                    "solo" => t.solo = value >= 0.5,
                    "pan" => t.pan = value.clamp(-1.0, 1.0),
                    "volume" => t.volume = value.clamp(0.0, 1.5),
                    "channel" => t.mixer_channel = value.round().clamp(0.0, 15.0) as i32,
                    _ => {}
                }
            }
            audio.mark_dirty();
        });
    }

    // =========================================================
    // DROP DE SAMPLE NA TRILHA (drag & drop do browser)
    //
    // Persiste o path no core e renomeia a trilha com o nome do
    // arquivo (mesmo comportamento do channel rack do FL Studio).
    // =========================================================

    {
        let win = ui.clone_strong();
        let handler = win.clone_strong();
        let project = project.clone();
        let audio = audio.clone();
        win.on_beat_sample_dropped(move |track, _name, path| {
            eprintln!("[drop] beat-sample-dropped track={track} path={path}");
            if track < 0 {
                return;
            }
            // Seleciona a linha alvo: o sampler mostra o sample da
            // linha selecionada, então soltar numa linha deve trazer
            // o sampler dela para a frente (feedback visual imediato,
            // como arrastar para um canal frente no FL).
            handler.set_beat_selected_track(track);
            commit(&handler, &project, &audio, |proj| {
                let Some(t) = proj.beat_track_params_mut(track as usize) else {
                    return;
                };
                t.sample_path = if path.is_empty() {
                    None
                } else {
                    Some(path.to_string())
                };

                if let Some(stem) = std::path::Path::new(path.as_str())
                    .file_stem()
                    .and_then(|n| n.to_str())
                {
                    t.name = stem.to_string();
                }
            });

            // "arrasta e reproduz": soltar o sample toca ele na hora,
            // com o transporte parado ou não. O decode acontece aqui
            // (thread da UI) e fica no cache, então quando o passo
            // disparar a thread RT já encontra o buffer pronto.
            if !path.is_empty() {
                audio.preload(path.as_str());
                preview_row(&project, &audio, track);
            }
        });
    }

    // =========================================================
    // PRÉ-ESCUTA (clique no botão da trilha)
    //
    // Mesma coisa que clicar no canal do channel rack do FL:
    // audiciona a linha sem mexer no pattern.
    // =========================================================

    {
        let project = project.clone();
        let audio = audio.clone();
        st.on_preview_track(move |track| {
            preview_row(&project, &audio, track);
        });
    }

    // Projeção inicial do estado do core.
    refresh_beat(ui, &project.borrow());
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::clip::ClipKind;
    use daw_midi::{BeatPatterns, DrumPattern, DRUM_PITCH_BASE};

    /// Pattern 8x16 = 4 beats, passo de 0.25, sem swing.
    fn pattern() -> DrumPattern {
        DrumPattern::new("PAT", 8, 16)
    }

    /// Projeto com o padrão "PAT" selecionado e UMA trilha de bateria.
    fn project_with_drum() -> RefCell<Project> {
        let mut p = Project::default();
        p.add_track(TrackKind::Drum, "Drums");
        p.beat_patterns = BeatPatterns {
            patterns: vec![pattern()],
            current: 0,
        };
        RefCell::new(p)
    }

    /// Transporte parado com o playhead em `beats` (PPQ 960).
    fn transport_at(beats: f32) -> Arc<Mutex<Transport>> {
        let mut t = Transport::default();
        t.seek((beats * t.ppq as f32) as u64);
        Arc::new(Mutex::new(t))
    }

    /// Liga o passo no padrão (o que o `commit` faz antes de pintar) e
    /// devolve o documento pintado, como na UI.
    fn paint(
        project: &RefCell<Project>,
        t: &Arc<Mutex<Transport>>,
        row: usize,
        step: usize,
        on: bool,
    ) -> bool {
        let mut proj = project.borrow_mut();
        proj.set_beat_step(row, step, on);
        paint_step(&mut proj, t, row, step, on)
    }

    fn midi_clips(proj: &RefCell<Project>) -> Vec<(u64, f32, f32, Vec<daw_midi::Note>)> {
        proj.borrow()
            .clips
            .iter()
            .filter(|c| c.is_midi())
            .map(|c| (c.id, c.start_beat, c.length_beats, c.notes.clone()))
            .collect()
    }

    #[test]
    fn painting_a_step_creates_a_midi_clip_on_the_drum_track() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        assert!(paint(&proj, &t, 0, 0, true));

        let clips = midi_clips(&proj);
        assert_eq!(clips.len(), 1, "um passo pintado cria um clip");
        let (_, start, len, notes) = &clips[0];
        assert_eq!(*start, 0.0);
        assert_eq!(*len, 4.0, "o clip tem a duração do padrão");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].pitch, DRUM_PITCH_BASE, "linha 0 = pitch base");
        assert_eq!(notes[0].start, 0.0, "notas são relativas ao clip");
        assert!((notes[0].length - 0.25).abs() < 1e-6, "duração = 1 passo");
    }

    #[test]
    fn turning_a_step_off_removes_the_note_and_keeps_the_clip() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        paint(&proj, &t, 0, 0, true);
        assert!(paint(&proj, &t, 0, 0, false));

        let clips = midi_clips(&proj);
        assert_eq!(clips.len(), 1, "o clip sobrevive ao passo desligado");
        assert!(clips[0].3.is_empty(), "a nota foi removida");
    }

    #[test]
    fn painting_uses_the_bar_the_playhead_is_in() {
        // Playhead no meio do compasso 0 → o padrão cabe em 0..4.
        let proj = project_with_drum();
        paint(&proj, &transport_at(2.5), 0, 0, true);
        assert_eq!(midi_clips(&proj)[0].1, 0.0);

        // Playhead no compasso 1 → o padrão vai para 4..8.
        let proj = project_with_drum();
        paint(&proj, &transport_at(4.0), 0, 0, true);
        let clips = midi_clips(&proj);
        assert_eq!(clips[0].1, 4.0);
        assert_eq!(clips[0].2, 4.0);
    }

    #[test]
    fn steps_in_the_same_bar_reuse_one_clip() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        paint(&proj, &t, 0, 0, true);
        // Mesmo compasso (origin 0), outra linha e outra coluna.
        paint(&proj, &t, 2, 4, true);

        let clips = midi_clips(&proj);
        assert_eq!(clips.len(), 1, "o mesmo compasso reusa o clip");
        let mut notes = clips[0].3.clone();
        notes.sort_by(|a, b| a.pitch.cmp(&b.pitch));
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, DRUM_PITCH_BASE);
        assert_eq!(notes[1].pitch, DRUM_PITCH_BASE + 2, "linha 2");
        assert!((notes[1].start - 1.0).abs() < 1e-6, "coluna 4 = beat 1");
    }

    #[test]
    fn other_bars_get_their_own_clip() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        paint(&proj, &t, 0, 0, true);
        paint(&proj, &transport_at(4.0), 0, 0, true);
        assert_eq!(midi_clips(&proj).len(), 2, "cada compasso tem seu clip");
    }

    #[test]
    fn painting_the_same_step_twice_does_not_duplicate_the_note() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        paint(&proj, &t, 0, 0, true);
        // Reaplica o mesmo passo (mesma linha/coluna/compasso).
        assert!(!paint(&proj, &t, 0, 0, true), "nada a fazer");
        assert_eq!(midi_clips(&proj)[0].3.len(), 1);
    }

    #[test]
    fn swing_offsets_the_note_but_not_the_clip() {
        let mut p = Project::default();
        p.add_track(TrackKind::Drum, "Drums");
        let mut pat = pattern();
        pat.swing = 1.0;
        p.beat_patterns = BeatPatterns {
            patterns: vec![pat],
            current: 0,
        };
        let proj = RefCell::new(p);
        let t = transport_at(0.0);

        // Coluna 1 (ímpar) recebe swing de meio passo: 0.25 + 0.125.
        paint(&proj, &t, 0, 1, true);
        // Coluna 15 (ímpar), a última: precisa caber no clip de 4 beats.
        paint(&proj, &t, 0, 15, true);

        let clips = midi_clips(&proj);
        let mut notes = clips[0].3.clone();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert!((notes[0].start - 0.375).abs() < 1e-6, "0.25 + swing 0.125");
        // 15 * 0.25 = 3.75 + 0.125 = 3.875: o ONSET da última coluna
        // ainda cai dentro do clip de 4 beats (é o que o motor usa para
        // disparar). A CAUDA pode passar do fim — o motor de bateria
        // ignora o comprimento da nota, e o drop do Clip Pool produz
        // exatamente o mesmo overhang, então os dois caminhos coincidem.
        assert!((notes[1].start - 3.875).abs() < 1e-6);
        assert!(notes[1].start < clips[0].2, "onset dentro do clip");
    }

    #[test]
    fn an_occupied_slot_moves_the_whole_pattern_not_the_note() {
        let mut p = Project::default();
        let drum = p.add_track(TrackKind::Drum, "Drums");
        p.beat_patterns = BeatPatterns {
            patterns: vec![pattern()],
            current: 0,
        };
        // Já ocupa 0..8 na trilha: o paint tem de ir para 8.
        p.add_clip(drum, ClipKind::Audio, "ocupa", 0.0, 8.0);
        let proj = RefCell::new(p);

        assert!(paint(&proj, &transport_at(0.0), 0, 0, true));
        let clips = midi_clips(&proj);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].1, 8.0, "clip empurrado para depois do vizinho");
        // A coluna continua valendo 0 — a nota NUNCA fica negativa.
        assert_eq!(clips[0].3[0].start, 0.0);
    }

    #[test]
    fn a_project_without_a_drum_track_gets_one() {
        let mut p = Project::default();
        p.beat_patterns = BeatPatterns {
            patterns: vec![pattern()],
            current: 0,
        };
        let proj = RefCell::new(p);

        assert!(paint(&proj, &transport_at(0.0), 0, 0, true));
        let proj = proj.borrow();
        assert_eq!(proj.tracks.len(), 1);
        assert_eq!(proj.tracks[0].kind, TrackKind::Drum);
        assert_eq!(proj.clips[0].track_id, proj.tracks[0].id);
    }

    #[test]
    fn an_existing_midi_track_is_preferred_over_creating_one() {
        let mut p = Project::default();
        let midi = p.add_track(TrackKind::Midi, "Sint");
        p.beat_patterns = BeatPatterns {
            patterns: vec![pattern()],
            current: 0,
        };
        let proj = RefCell::new(p);

        assert!(paint(&proj, &transport_at(0.0), 0, 0, true));
        let proj = proj.borrow();
        assert_eq!(proj.tracks.len(), 1, "nenhuma trilha nova");
        assert_eq!(proj.clips[0].track_id, midi);
    }

    #[test]
    fn painting_is_recorded_in_the_undo_history() {
        let proj = project_with_drum();
        let t = transport_at(0.0);
        paint(&proj, &t, 0, 0, true);
        assert!(proj.borrow().can_undo(), "cada pintura é um passo de undo");

        // Desfazer devolve a timeline ao estado anterior: o clip criado
        // pela pintura some junto.
        assert!(proj.borrow_mut().undo());
        assert!(midi_clips(&proj).is_empty());
    }

    #[test]
    fn paint_origin_snaps_to_the_pattern_length() {
        assert_eq!(paint_origin(0.0, 4.0), 0.0);
        assert_eq!(paint_origin(3.9, 4.0), 0.0);
        assert_eq!(paint_origin(4.0, 4.0), 4.0);
        assert_eq!(paint_origin(7.5, 4.0), 4.0);
        // Padrão degenerado: sem divisão por zero.
        assert_eq!(paint_origin(3.0, 0.0), 3.0);
    }

    #[test]
    fn same_cell_ignores_float_noise_but_not_the_neighbour() {
        let note = daw_midi::Note {
            id: 1,
            pitch: DRUM_PITCH_BASE,
            start: 1.0,
            length: 0.25,
            velocity: 100,
        };
        assert!(same_cell(note, DRUM_PITCH_BASE, 1.0001));
        assert!(!same_cell(note, DRUM_PITCH_BASE, 1.25), "coluna vizinha");
        assert!(!same_cell(note, DRUM_PITCH_BASE + 1, 1.0), "outra linha");
    }
}
