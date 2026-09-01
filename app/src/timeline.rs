//! Timeline: todos os handlers de edição de clips — drop,
//! slice, glue, move/resize (commit único), delete/rename/cor,
//! clipboard, duplicate, add-track e undo/redo.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ClipData;
use crate::MainWindow;
use crate::importer::{import_audio_file_async, is_audio_path};
use crate::models::clip_row;
use crate::waveform::WaveformCache;
use daw_core::clip::{Clip, ClipKind};
use daw_core::project::Project;
use daw_core::track::TrackKind;
use slint::ComponentHandle;
use slint::Model;

pub fn install(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    track_colors_model: Rc<slint::VecModel<i32>>,
    sample_cache: Rc<RefCell<crate::clip_cache::SampleCache>>,
    apply_mixer_state: Rc<dyn Fn()>,
    fit_float_mixer_windows: Rc<dyn Fn()>,
) {
    // Drop de clip (ClipPool / Explorer) → core (evento reprojeta).
    {
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        let worker = crate::decode_worker::DecodeWorker::default();
        let drop_track_colors = track_colors_model.clone();
        let ui_weak = ui.as_weak();
        ui.on_clip_dropped(move |track, beat, name, is_midi, path| {
            let bpm = ui_weak
                .upgrade()
                .map(|u| u.get_bpm() as f32)
                .unwrap_or(120.0);

            // Arquivo real de áudio → mesma rota do drop do Sistema
            // Operacional: duração real, waveform e vaga livre.
            // Decode na worker thread; a UI segue fluida.
            if !is_midi && !path.is_empty() {
                let file = std::path::PathBuf::from(path.as_str());
                if is_audio_path(&file) && file.is_file() {
                    import_audio_file_async(
                        &file,
                        track,
                        beat,
                        bpm,
                        project.clone(),
                        waveforms.clone(),
                        clips_model.clone(),
                        drop_track_colors.clone(),
                        &worker,
                    );
                    return;
                }
            }

            // Sem arquivo (clip pool / MIDI): bloco padrão de 4 beats —
            // ou a duração REAL quando o nome é de um padrão do beat
            // sequencer (arrastado da seção BEAT PATTERNS do pool).
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
                let len = if kind == ClipKind::Midi {
                    proj.beat_patterns
                        .patterns
                        .iter()
                        .find(|p| p.name == name.as_str())
                        .map(|p| p.length_beats())
                        .unwrap_or(4.0)
                } else {
                    4.0
                };
                let start = proj.find_free_slot(track_id, beat.max(0.0), len);
                proj.add_clip(track_id, kind, name.as_str(), start, len);
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
    //
    // Depois do corte no core, divide a waveform cacheada entre as
    // duas metades: sem isso o lado esquerdo mostra o arquivo inteiro
    // espremido e o direito não mostra nada (corte não visual).
    {
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_slice_requested(move |id, position| {
            let id = id as u64;
            let mut proj = project.borrow_mut();
            let Some(clip) = proj.clip(id).cloned() else {
                return;
            };
            let at = clip.start_beat + position.max(0.0);
            let Some(right_id) = proj.slice_clip(id, at) else {
                return;
            };
            drop(proj);

            // Fração de tempo que ficou no lado esquerdo.
            let fraction = ((at - clip.start_beat) / clip.length_beats) as f64;
            let mut wf = waveforms.borrow_mut();
            let Some(cw) = wf.get(&id).cloned() else {
                return;
            };
            let (left, right) = crate::waveform::split_cached_wave(&cw, fraction);
            wf.insert(id, left);
            wf.insert(right_id, right);
            drop(wf);

            let wf = waveforms.borrow();
            crate::waveform::reproject_clip(&clips_model, &wf, id);
            crate::waveform::reproject_clip(&clips_model, &wf, right_id);
        });
    }

    // Glue de clip (ferramenta cola) → core: funde com o vizinho à
    // direita. Depois, uni as duas metades da waveform de volta num
    // cache só (o inverso exato do slice).
    {
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_glue_requested(move |id| {
            let id = id as u64;
            let removed = {
                let mut proj = project.borrow_mut();
                proj.glue_clip(id)
            };
            let Some(right_id) = removed else {
                return;
            };
            let mut wf = waveforms.borrow_mut();
            let merged = match (wf.get(&id).cloned(), wf.get(&right_id).cloned()) {
                (Some(a), Some(b)) => Some(crate::waveform::merge_cached_waves(&a, &b)),
                (Some(a), None) => Some(a),
                _ => None,
            };
            if let Some(m) = merged {
                wf.insert(id, m);
                wf.remove(&right_id);
                drop(wf);
                let wf = waveforms.borrow();
                crate::waveform::reproject_clip(&clips_model, &wf, id);
            }
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
    //
    // A waveform NÃO é re-projetada nem re-escalada pelo resize:
    // o cache é aparado ao trecho visível novo (crop ANCORADO ao
    // grid — borda só esconde/revela) e o re-render seguinte já
    // desenha o trecho na mesma escala. Quem altera o desenho do
    // wave são o ZOOM e o FADE in/out.
    {
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        ui.on_clip_resize_committed(move |id, side, new_len, new_beat| {
            let id_u = id.max(0) as u64;
            let len = new_len.max(0.25);
            let beat = new_beat.max(0.0);

            let Some(i) = clip_row(&clips_model, id) else {
                return;
            };
            let Some(mut clip) = clips_model.row_data(i) else {
                return;
            };

            let old_start = clip.beat;
            let old_len = clip.len;
            let start = if side < 0 { beat } else { old_start };

            // Apara o cache ao trecho que o clip passa a enxergar:
            // borda esquerda avança o source (janela fraciona a partir
            // do início do OLD len); borda direita só encurta. O wave
            // mantém a mesma amostra-por-pixel → nada de esticamento.
            if !clip.is_midi && old_len > 0.0 {
                let f0 = (((beat - old_start) / old_len).clamp(0.0, 1.0)) as f64;
                let f1 = ((len / old_len).clamp(0.0, 1.0 - f0 as f32)) as f64;
                let mut wf = waveforms.borrow_mut();
                if let Some(cw) = wf.get(&id_u).cloned() {
                    let trimmed = crate::waveform::trim_cached_wave(&cw, f0, f1);
                    wf.insert(id_u, trimmed);
                }
                drop(wf);
                let wf = waveforms.borrow();
                let updated = crate::waveform::to_clip_data_from_id(id_u, &clip, &wf);
                clip.beat = beat;
                clip.len = len;
                // Só os DADOS mudam aqui: a imagem anterior (ancorada)
                // continua no lugar até o re-render do trecho novo chegar
                // (o corpo com `clip` já mostra o crop correto no meio tempo).
                clip.waveform_mins = updated.waveform_mins;
                clip.waveform_maxes = updated.waveform_maxes;
            } else {
                clip.beat = beat;
                clip.len = len;
            }

            clips_model.set_row_data(i, clip);
            project.borrow_mut().resize_clip(id_u, start, len);
        });
    }

    // Delete de clip → core.
    {
        let project = project.clone();
        ui.on_clip_delete_requested(move |id| {
            project.borrow_mut().delete_clip(id as u64);
        });
    }

    // Delete em massa (Delete/Backspace): apaga TODA a seleção —
    // a seleção única (`selected-clip`, o último clip clicado) MAIS
    // os clips marcados pela multi-seleção (marquee/Ctrl+arrasto).
    // O core não conhece seleção (estado visual da UI) — ler direto
    // do modelo de clips do Slint.
    {
        let project = project.clone();
        let ui_weak = ui.as_weak();
        ui.on_multi_delete_requested(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let clips = ui.get_clips();

            // Seleção única primeiro (a "origem" da instância).
            let single = ui.get_selected_clip();
            let single = if single >= 0 { Some(single as u64) } else { None };

            let mut ids: Vec<u64> = (0..clips.row_count())
                .filter_map(|i| clips.row_data(i))
                .filter(|d| d.multi_selected)
                .map(|d| d.id as u64)
                .collect();
            if let Some(s) = single {
                if !ids.contains(&s) {
                    ids.push(s);
                }
            }
            if ids.is_empty() {
                return;
            }

            // Limpa a seleção única: o clip apagado pode ser exatamente
            // o selecionado. Os multi-selecionados que restarem seguem
            // marcados — DAWs mantêm a seleção após apagar.
            ui.set_selected_clip(-1);

            let mut proj = project.borrow_mut();
            for id in ids {
                proj.delete_clip(id);
            }
        });
    }

    // Rename de clip (double-click + edição inline) → core.
    {
        let project = project.clone();
        ui.on_clip_rename_requested(move |id, new_name| {
            project
                .borrow_mut()
                .rename_clip(id as u64, new_name.as_str());
        });
    }

    // Ciclo de cor do clip (swatch click) → core.
    // Paleta de 20 cores (ver ui/clip_palette.slint).
    {
        let project = project.clone();
        ui.on_clip_color_cycle_requested(move |id| {
            let new_color = {
                let proj = project.borrow();
                let current = proj
                    .clips
                    .iter()
                    .find(|c| c.id as i32 == id)
                    .map(|c| c.color)
                    .unwrap_or(0);
                (current + 1) % 20
            };
            project.borrow_mut().set_clip_color(id as u64, new_color);
        });
    }

    // Fade do clip (drag no pino do clip) → core, com undo unitário.
    // O gesto é 100% local no TrackClip e só aqui cai o commit.
    {
        let project = project.clone();
        ui.on_clip_fade_committed(move |id, fade_in, fade_out| {
            project
                .borrow_mut()
                .set_clip_fades(id as u64, fade_in, fade_out);
        });
    }

    // Gain do clip (drag na barra central) → core, com undo unitário.
    // O gesto é 100% local no TrackClip e só aqui cai o commit.
    {
        let project = project.clone();
        ui.on_clip_gain_committed(move |id, gain_db| {
            project
                .borrow_mut()
                .set_clip_gain(id as u64, gain_db);
        });
    }

    // Botão "add color" do cabeçalho TRACKS → aplica o índice de cor
    // escolhido à track (cabeçalho) e a TODOS os seus clips.
    {
        let project = project.clone();
        let colors_model = track_colors_model.clone();
        let ui_weak = ui.as_weak();
        ui.on_track_recolor_requested(move |track_index, color| {
            let mut proj = project.borrow_mut();
            if proj.tracks.is_empty() {
                return;
            }
            let idx = (track_index.max(0) as usize).min(proj.tracks.len() - 1);
            let track_id = proj.tracks[idx].id;
            let color = color.clamp(0, 255) as u8;

            eprintln!(
                "[RECOLOR] track_index={} -> idx={} color={} selected_clip={}",
                track_index, idx, color, ui_weak.upgrade().map(|u| u.get_selected_clip()).unwrap_or(-999)
            );

            // Com um clip selecionado, a paleta do Track Panel atua
            // somente nele. Sem seleção, continua colorindo a track.
            if let Some(ui) = ui_weak.upgrade() {
                let selected = ui.get_selected_clip();
                if selected >= 0 {
                    proj.set_clip_color(selected as u64, color);
                    return;
                }
            }

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

    // Clipboard interno de clips: Ctrl+C copia, Shift+clique em área
    // vazia da timeline cola uma cópia na posição clicada.
    //
    // CACHE EM RAM: além dos metadados do Clip, o Ctrl+C também
    // garante a waveform do clip no SampleCache — a colagem vira
    // operação puramente de memória (sem re-decode, sem disco).
    let clip_clipboard: Rc<RefCell<Option<Clip>>> = Rc::new(RefCell::new(None));
    {
        let project = project.clone();
        let clipboard = clip_clipboard.clone();
        let sample_cache = sample_cache.clone();
        let ui_copy = ui.as_weak();
        ui.on_clip_copy_requested(move |id| {
            let Some(src) = project
                .borrow()
                .clips
                .iter()
                .find(|c| c.id as i32 == id)
                .cloned()
            else {
                return;
            };
            // Pré-carrega o sample do clip no cache (se for arquivo;
            // já cacheado = no-op). Colagem instantânea depois.
            if let Some(path_str) = src.file_path.clone() {
                let bpm = ui_copy
                    .upgrade()
                    .map(|u| u.get_bpm() as f32)
                    .unwrap_or(120.0);
                crate::clip_cache::spawn_prefetch(
                    &sample_cache,
                    std::path::Path::new(&path_str),
                    bpm,
                );
            }
            *clipboard.borrow_mut() = Some(src);
        });
    }
    {
        let project = project.clone();
        let clipboard = clip_clipboard.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
        let ui_paste = ui.as_weak();
        ui.on_clip_paste_requested(move |beat, track| {
            let Some(src) = clipboard.borrow().clone() else {
                return;
            };
            // Snap ON → cola no início da célula; OFF → onde o mouse clicou.
            let snap = ui_paste
                .upgrade()
                .map(|u| u.get_snap_enabled())
                .unwrap_or(true);
            let mut proj = project.borrow_mut();
            let track_count = proj.tracks.len().max(1);
            let track_id = (track.max(0) as usize).min(track_count - 1) as u64;
            let desired = if snap { beat.floor() } else { beat }.max(0.0);
            let start = proj.find_free_slot(track_id, desired, src.length_beats);
            // Cópia herda a cor (e o override) do clip de origem.
            let new_id = proj.add_clip_from(&src, track_id, start);

            // Waveform da cópia: mesmo arquivo do clipboard →
            // copia a entrada do cache e reprojeta a linha.
            // (Clona FORA do borrow para não segurar o RefCell.)
            let src_wf = waveforms.borrow().get(&src.id).cloned();
            if let (Some(new_id), Some(cw)) = (new_id, src_wf) {
                waveforms.borrow_mut().insert(new_id, cw);
                crate::waveform::reproject_clip(&clips_model, &waveforms.borrow(), new_id);
            }
        });
    }

    // Duplicate de clip (menu de contexto) → cópia na mesma posição.
    {
        let project = project.clone();
        let waveforms = waveforms.clone();
        let clips_model = clips_model.clone();
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
            // Duplicata herda a cor (e o override) do original.
            let new_id = proj.add_clip_from(&src, src.track_id, start);

            // Waveform: a duplicata aponta pro MESMO arquivo →
            // copia a entrada do cache (picos + samples + raster)
            // do clip de origem e reprojeta a linha da cópia.
            if let Some(new_id) = new_id {
                let cached = waveforms.borrow().get(&src.id).cloned();
                if let Some(cw) = cached {
                    waveforms.borrow_mut().insert(new_id, cw);
                    crate::waveform::reproject_clip(&clips_model, &waveforms.borrow(), new_id);
                }
            }
        });
    }

    // Marquee (Ctrl+arrasto): marca multi-selected nos clips
    // que intersectam o retângulo (beats × trilhas).
    {
        let clips_model = clips_model.clone();
        ui.on_marquee_select(move |beat0: f32, beat1: f32, track0: i32, track1: i32| {
            for i in 0..clips_model.row_count() {
                if let Some(mut d) = clips_model.row_data(i) {
                    let hit = d.beat < beat1
                        && (d.beat + d.len) > beat0
                        && d.track_index >= track0
                        && d.track_index <= track1;
                    if d.multi_selected != hit {
                        d.multi_selected = hit;
                        clips_model.set_row_data(i, d);
                    }
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
}
