//! Importação de arquivos de áudio para a timeline.
//!
//! Caminho único usado pelo drop do explorador interno e pelo
//! drop do Sistema Operacional.

use daw_core::clip::ClipKind;
use daw_core::project::Project;
use std::cell::RefCell;

use crate::ClipData;
use crate::models::clip_row;
use crate::waveform::{CachedWave, WaveformCache, to_clip_data_from_id};
use daw_media::waveform::{decode_audio, waveform_points};
use daw_media::waveform_image::waveform_image_rgba;
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
///
/// É isto que impede a waveform de ser achatada: o clip passa a ter o
/// comprimento real do arquivo em vez de um valor fixo em que o
/// desenho tinha que caber.
fn beats_for_seconds(seconds: f32, bpm: f32) -> f32 {
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };
    (seconds * bpm / 60.0).max(0.01)
}

/// Detecta silêncio nas bordas e devolve o áudio RECORTADO.
///
/// Volta `Some((dados recortados, frame inicial do recorte))` quando
/// há silêncio significativo no início/fim — o "aparar até o sinal
/// zero" da importação: o clip não fica com um vão vazio de áudio
/// zero. `None` = nada a aparar (conteúdo presente nas duas bordas
/// ou arquivo todo em silêncio).
///
/// A detecção usa o buffer INTERLEAVED (fonte do motor): stereo em
/// fase oposta cancela no mono de exibição, mas é audível — aparar
/// pelo mono apagaria o sample.
fn trim_silence(
    data: &daw_media::waveform::WaveformData,
) -> Option<(daw_media::waveform::WaveformData, usize)> {
    // Limiar de "sinal" (~ -60 dBFS) e silêncio mínimo para valer a
    // pena aparar. Micro-silêncios são ruído do fim de transiente.
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

    // Respiro de ~2 ms nas bordas: não cortar o ataque/decay rente.
    let pad = ((0.002 * data.sample_rate as f32) as usize).max(1);
    let start = first.saturating_sub(pad);
    let end = (last + 1 + pad).min(frames);
    if end - start < 2 {
        return None;
    }

    let trimmed = daw_media::waveform::WaveformData {
        samples: data.samples[start..end].to_vec(),
        interleaved: data.interleaved[start * ch..end * ch].to_vec(),
        sample_rate: data.sample_rate,
        channels: data.channels,
        duration_seconds: (end - start) as f32 / data.sample_rate as f32,
    };
    Some((trimmed, start))
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
pub fn import_audio_file(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<ClipData>,
    track_colors_model: &slint::VecModel<i32>,
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
    // Apara silêncio nas bordas: o clip cobre a parte audível do
    // arquivo, não um vão cheio de zeros (source_start_beats marca o
    // deslocamento dentro do arquivo; o motor continua certo).
    let (source, first_frame) = match trim_silence(&decoded) {
        Some((trimmed, first)) => (trimmed, first),
        None => (decoded, 0usize),
    };
    let length = beats_for_seconds(source.duration_seconds, bpm);
    let source_start_beats = beats_for_seconds(
        first_frame as f32 / source.sample_rate as f32,
        bpm,
    );

    // Resolução proporcional à duração: com um número fixo de pontos,
    // um loop de 1 s e um take de 3 min viravam o mesmo borrão. O teto
    // existe porque cada ponto é um retângulo no desenho do clip.
    // Resolução alta: com poucos pontos, um take denso virava
    // um tijolo cheio sem forma. O teto existe porque cada
    // ponto é um retângulo no desenho do clip.
    let points = ((length * 24.0) as usize).clamp(256, 2048);
    let peaks = waveform_points(&source, points);
    // Pré-renderiza a waveform em RGBA (cor padrão de áudio) uma
    // única vez; a UI desenha um Image em vez de ~1000 retângulos.
    // Se o usuário trocar a cor do clip, re-renderizamos adiante
    // (ver on_clip_color_cycle_requested).
    let img = waveform_image_rgba(&peaks, 0, false);
    let lods = daw_media::waveform_image::build_waveform_lods(&source.samples);

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
        let id = proj.add_clip_with_source(
            track_id,
            ClipKind::Audio,
            name.as_str(),
            start,
            length,
            source_start_beats,
            Some(path.to_string_lossy().to_string()),
        )?;
        // Cor herdada do SAMPLE (definida no add_clip_with_path):
        // sincroniza a cor da track com a cor primária do arquivo.
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

    // `ClipAdded` já disparou e leu o cache ainda vazio; preenche
    // agora (picos + mono de exibição + pirâmide + imagem) e
    // reprojeta só a linha desse clip.
    if !peaks.is_empty() {
        let (mono_ds, mip) = daw_media::waveform_image::build_display_wave(&source.samples);
        waveforms.borrow_mut().insert(
            clip_id,
            CachedWave {
                peaks,
                mono_ds,
                mip,
                lods,
                image: img,
            },
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

/// Importa um arquivo de áudio usando o CACHE DE SAMPLES em RAM.
///
/// Rota do drop do Sistema Operacional: como o pré-decode começou no
/// `HoveredFile` (cursor entrando na janela), normalmente a entrada
/// já está no cache e NADA é relido do disco — a inserção é imediata.
/// Se por acaso o decode em background ainda não terminou, cai no
/// caminho síncrono original (nunca falha).
pub fn import_audio_file_with_cache(
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<ClipData>,
    track_colors_model: &slint::VecModel<i32>,
    sample_cache: &RefCell<crate::clip_cache::SampleCache>,
) -> Option<u64> {
    // Tenta pela rota cacheada (instantânea).
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
    // Cache indisponível (arquivo quebrado): rota antiga para dar o
    // erro adequado / não criar clip morto.
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

/// Cria o clip a partir de uma entrada JÁ decodificada do cache.
/// Zero leitura de disco: metadados + waveform vêm da RAM.
fn import_from_entry(
    entry: crate::clip_cache::CachedSample,
    path: &std::path::Path,
    track_hint: i32,
    beat_hint: f32,
    bpm: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<ClipData>,
    track_colors_model: &slint::VecModel<i32>,
) -> Option<u64> {
    let name = entry.name.clone();
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };

    // Apara silêncio nas bordas (mesma regra do import direto). O
    // `entry.wave` cobre o arquivo INTEIRO; quando aparado, a
    // waveform é reconstruída do trecho audível.
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
    let points = ((length * 24.0) as usize).clamp(256, 2048);
    let peaks = waveform_points(&source, points);
    let img = waveform_image_rgba(&peaks, 0, false);
    let lods = daw_media::waveform_image::build_waveform_lods(&source.samples);

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
        // Cor herdada do SAMPLE (definida no add_clip_with_source):
        // sincroniza a cor da track com a cor primária do arquivo.
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

    // Waveform RECORTADA (silêncio aparado): picos + mono + pirâmide
    // + imagem construídos do trecho, e reprojeta a linha.
    if !peaks.is_empty() {
        let (mono_ds, mip) = daw_media::waveform_image::build_display_wave(&source.samples);
        waveforms.borrow_mut().insert(
            clip_id,
            CachedWave {
                peaks,
                mono_ds,
                mip,
                lods,
                image: img,
            },
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

/// Rota "sem recorte" do import via cache: waveform já pronta em RAM,
/// só publica no cache de UI e reprojeta (zero reprocessamento).
fn import_from_entry_untrimmed(
    entry: crate::clip_cache::CachedSample,
    path: &std::path::Path,
    length: f32,
    track_hint: i32,
    beat_hint: f32,
    project: &RefCell<Project>,
    waveforms: &RefCell<WaveformCache>,
    clips_model: &slint::VecModel<ClipData>,
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
        // Cor herdada do SAMPLE (definida no add_clip_with_path):
        // sincroniza a cor da track com a cor primária do arquivo.
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

    // Waveform já pronta em RAM: só publica no cache de UI e
    // reprojeta a linha desse clip.
    if !entry.wave.peaks.is_empty() {
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
