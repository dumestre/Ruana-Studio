//! Importação de arquivos de áudio para a timeline.
//!
//! Caminho único usado pelo drop do explorador interno e pelo
//! drop do Sistema Operacional.

use crate::clip_cache::CachedSample;
use crate::decode_worker::DecodeWorker;
use crate::models::clip_row;
use crate::waveform::{CachedWave, WaveformCache, build_lods, to_clip_data_from_id};
use daw_core::clip::ClipKind;
use daw_core::project::Project;
use daw_media::waveform::{decode_audio, waveform_points};
use std::cell::RefCell;
use std::rc::Rc;
use slint::Model;

/// Extensões de áudio que o `daw-media` sabe decodificar.
pub const AUDIO_EXTS: [&str; 5] = ["WAV", "MP3", "FLAC", "OGG", "OGA"];

/// O arquivo é um áudio importável?
pub fn is_audio_path(path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();
    AUDIO_EXTS.contains(&ext.as_str())
}

/// Quantos beats um trecho de `seconds` ocupa a `bpm`.
fn beats_for_seconds(seconds: f32, bpm: f32) -> f32 {
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };
    (seconds * bpm / 60.0).max(0.01)
}

/// Detecta silêncio nas bordas e devolve o áudio RECORTADO.
fn trim_silence(
    data: &daw_media::waveform::WaveformData,
) -> Option<(daw_media::waveform::WaveformData, usize)> {
    const THRESHOLD: f32 = 0.001;
    const MIN_TRIM_SECS: f32 = 0.05;

    let ch = data.channels.max(1) as usize;
    let frames = data.frames();
    if frames == 0 {
        return None;
    }
    let audible = |f: usize| {
        let base = f * ch;
        (0..ch).any(|c| data.interleaved[base + c].abs() > THRESHOLD)
    };
    let first = (0..frames).find(|&f| audible(f))?;
    let last = (0..frames).rev().find(|&f| audible(f))?;

    let min_trim = (MIN_TRIM_SECS * data.sample_rate as f32) as usize;
    let lead = first;
    let trail = frames.saturating_sub(last + 1);
    if lead < min_trim && trail < min_trim {
        return None;
    }

    let pad = ((0.002 * data.sample_rate as f32) as usize).max(1);
    let start = first.saturating_sub(pad);
    let end = (last + 1 + pad).min(frames);
    if end - start < 2 {
        return None;
    }

    let trimmed_samples = data.samples[start..end].to_vec();
    let trimmed = daw_media::waveform::WaveformData {
        samples: trimmed_samples,
        display_samples: Vec::new(),
        mipmap: daw_media::waveform_image::Mipmap { levels: Vec::new() },
        interleaved: data.interleaved[start * ch..end * ch].to_vec(),
        sample_rate: data.sample_rate,
        channels: data.channels,
        duration_seconds: (end - start) as f32 / data.sample_rate as f32,
    };
    Some((trimmed, start))
}

/// Importa um arquivo de áudio para a timeline (síncrono).
pub fn import_audio_file(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<crate::ClipData>,
    track_colors_model: &slint::VecModel<i32>,
) -> Option<u64> {
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();

    let decoded = match decode_audio(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("[import] {} não decodificou: {err}", name);
            return None;
        }
    };

    let (source, first_frame) = match trim_silence(&decoded) {
        Some((trimmed, first)) => (trimmed, first),
        None => (decoded, 0usize),
    };
    let length = beats_for_seconds(source.duration_seconds, bpm);
    let source_start_beats = beats_for_seconds(
        first_frame as f32 / source.sample_rate as f32,
        bpm,
    );

    // Constrói LODs do trecho aparado.
    let lods = build_lods(&source.samples);

    let clip_id = {
        let mut proj = project.borrow_mut();
        let preferred = track_hint.max(0) as u64;
        let track_id = proj
            .track_with_room(preferred, beat_hint.max(0.0), length)
            .or_else(|| proj.track(preferred).map(|t| t.id))?;
        let start = proj.find_free_slot(track_id, beat_hint.max(0.0), length);
        let id = proj.add_clip_with_source(
            track_id,
            ClipKind::Audio,
            name.as_str(),
            start,
            length,
            source_start_beats,
            Some(path.to_string_lossy().to_string()),
        )?;
        let color = proj
            .clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.color)
            .unwrap_or(0);
        if let Some(track_index) = proj.tracks.iter().position(|t| t.id == track_id) {
            proj.tracks[track_index].color = color;
            if track_index < track_colors_model.row_count() {
                track_colors_model.set_row_data(track_index, color as i32);
            }
        }
        id
    };

    if !lods.is_empty() {
        let _points = waveform_points(&source, ((length * 24.0) as usize).clamp(256, 2048));
        waveforms.borrow_mut().insert(
            clip_id,
            CachedWave { lods, frames: source.samples.len() },
        );
        let wf = waveforms.borrow();
        if let Some(i) = clip_row(clips_model, clip_id as i32) {
            if let Some(data) = clips_model.row_data(i) {
                clips_model.set_row_data(i, to_clip_data_from_id(clip_id, &data, &wf));
            }
        }
    }
    Some(clip_id)
}

/// Importa um arquivo de áudio para a timeline com DECODE NA WORKER.
pub fn import_audio_file_async(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: Rc<RefCell<Project>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<crate::ClipData>>,
    track_colors_model: Rc<slint::VecModel<i32>>,
    worker: &DecodeWorker,
) {
    let path_str = path.to_string_lossy().to_string();
    let err_label = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();
    worker.schedule_import(path, bpm, move |ready| {
        let Some(imp) = ready else {
            eprintln!("[import] {err_label} não decodificou");
            return;
        };

        let clip_id = {
            let mut proj = project.borrow_mut();
            let preferred = track_hint.max(0) as u64;
            let track_id = proj
                .track_with_room(preferred, beat_hint.max(0.0), imp.length_beats)
                .or_else(|| proj.track(preferred).map(|t| t.id));
            let Some(track_id) = track_id else {
                return;
            };
            let start = proj.find_free_slot(track_id, beat_hint.max(0.0), imp.length_beats);
            let Some(id) = proj.add_clip_with_source(
                track_id,
                ClipKind::Audio,
                imp.name.as_str(),
                start,
                imp.length_beats,
                imp.source_start_beats,
                Some(path_str.clone()),
            ) else {
                return;
            };
            let color = proj
                .clips
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.color)
                .unwrap_or(0);
            if let Some(track_index) = proj.tracks.iter().position(|t| t.id == track_id) {
                proj.tracks[track_index].color = color;
                if track_index < track_colors_model.row_count() {
                    track_colors_model.set_row_data(track_index, color as i32);
                }
            }
            id
        };

        if !imp.lods.is_empty() {
            waveforms.borrow_mut().insert(
                clip_id,
                CachedWave { lods: imp.lods, frames: imp.frames },
            );
            let wf = waveforms.borrow();
            if let Some(i) = clip_row(&clips_model, clip_id as i32) {
                if let Some(data) = clips_model.row_data(i) {
                    clips_model.set_row_data(i, to_clip_data_from_id(clip_id, &data, &wf));
                }
            }
        }
    });
}

/// Importa usando o CACHE DE SAMPLES em RAM.
pub fn import_audio_file_with_cache(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<crate::ClipData>,
    track_colors_model: &slint::VecModel<i32>,
    sample_cache: &RefCell<crate::clip_cache::SampleCache>,
) -> Option<u64> {
    if let Some(entry) = crate::clip_cache::ensure_cached(sample_cache, path, bpm) {
        return import_from_entry(
            entry,
            path,
            track_hint,
            beat_hint,
            bpm,
            project,
            waveforms,
            clips_model,
            track_colors_model,
        );
    }
    import_audio_file(
        path,
        track_hint,
        beat_hint,
        bpm,
        project,
        waveforms,
        clips_model,
        track_colors_model,
    )
}

/// Importa via cache se já em RAM; senão agenda decode na worker.
pub fn import_audio_file_with_cache_async(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: Rc<RefCell<Project>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<crate::ClipData>>,
    track_colors_model: Rc<slint::VecModel<i32>>,
    sample_cache: &RefCell<crate::clip_cache::SampleCache>,
    worker: &DecodeWorker,
) {
    if crate::clip_cache::peek_cached(sample_cache, path) {
        let _ = import_audio_file_with_cache(
            path,
            track_hint,
            beat_hint,
            bpm,
            &*project,
            &*waveforms,
            &clips_model,
            &track_colors_model,
            sample_cache,
        );
        return;
    }
    import_audio_file_async(
        path,
        track_hint,
        beat_hint,
        bpm,
        project,
        waveforms,
        clips_model,
        track_colors_model,
        worker,
    );
}

/// Cria o clip a partir de uma entrada JÁ decodificada do cache.
fn import_from_entry(
    entry: CachedSample,
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<crate::ClipData>,
    track_colors_model: &slint::VecModel<i32>,
) -> Option<u64> {
    let name = entry.name.clone();
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };

    let (source, first_frame) = match trim_silence(&entry.decoded) {
        Some((trimmed, first)) => (trimmed, first),
        None => {
            let length = (entry.duration_seconds * bpm / 60.0).max(0.01);
            return import_from_entry_untrimmed(
                entry,
                path,
                length,
                track_hint,
                beat_hint,
                project,
                waveforms,
                clips_model,
                track_colors_model,
            );
        }
    };
    let length = (source.duration_seconds * bpm / 60.0).max(0.01);
    let source_start_beats =
        beats_for_seconds(first_frame as f32 / source.sample_rate as f32, bpm);
    let lods = build_lods(&source.samples);

    let clip_id = {
        let mut proj = project.borrow_mut();
        let preferred = track_hint.max(0) as u64;
        let track_id = proj
            .track_with_room(preferred, beat_hint.max(0.0), length)
            .or_else(|| proj.track(preferred).map(|t| t.id))?;
        let start = proj.find_free_slot(track_id, beat_hint.max(0.0), length);
        let id = proj.add_clip_with_source(
            track_id,
            ClipKind::Audio,
            name.as_str(),
            start,
            length,
            source_start_beats,
            Some(path.to_string_lossy().to_string()),
        )?;
        let color = proj
            .clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.color)
            .unwrap_or(0);
        if let Some(track_index) = proj.tracks.iter().position(|t| t.id == track_id) {
            proj.tracks[track_index].color = color;
            if track_index < track_colors_model.row_count() {
                track_colors_model.set_row_data(track_index, color as i32);
            }
        }
        id
    };

    if !lods.is_empty() {
        let _points = waveform_points(&source, ((length * 24.0) as usize).clamp(256, 2048));
        waveforms.borrow_mut().insert(
            clip_id,
            CachedWave { lods, frames: source.samples.len() },
        );
        let wf = waveforms.borrow();
        if let Some(i) = clip_row(clips_model, clip_id as i32) {
            if let Some(data) = clips_model.row_data(i) {
                clips_model.set_row_data(i, to_clip_data_from_id(clip_id, &data, &wf));
            }
        }
    }
    Some(clip_id)
}

/// Rota "sem recorte" do import via cache: waveform já pronta em RAM.
fn import_from_entry_untrimmed(
    entry: CachedSample,
    path: &std::path::Path,
    length: f32,
    track_hint: i32,
    beat_hint: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<crate::ClipData>,
    track_colors_model: &slint::VecModel<i32>,
) -> Option<u64> {
    let name = entry.name.clone();
    let clip_id = {
        let mut proj = project.borrow_mut();
        let preferred = track_hint.max(0) as u64;
        let track_id = proj
            .track_with_room(preferred, beat_hint.max(0.0), length)
            .or_else(|| proj.track(preferred).map(|t| t.id))?;
        let start = proj.find_free_slot(track_id, beat_hint.max(0.0), length);
        let id = proj.add_clip_with_path(
            track_id,
            ClipKind::Audio,
            name.as_str(),
            start,
            length,
            Some(path.to_string_lossy().to_string()),
        )?;
        let color = proj
            .clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.color)
            .unwrap_or(0);
        if let Some(track_index) = proj.tracks.iter().position(|t| t.id == track_id) {
            proj.tracks[track_index].color = color;
            if track_index < track_colors_model.row_count() {
                track_colors_model.set_row_data(track_index, color as i32);
            }
        }
        id
    };

    if !entry.wave.lods.is_empty() {
        waveforms.borrow_mut().insert(clip_id, entry.wave.clone());
        let wf = waveforms.borrow();
        if let Some(i) = clip_row(clips_model, clip_id as i32) {
            if let Some(data) = clips_model.row_data(i) {
                clips_model.set_row_data(i, to_clip_data_from_id(clip_id, &data, &wf));
            }
        }
    }
    Some(clip_id)
}