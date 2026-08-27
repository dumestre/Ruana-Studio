//! Cache de samples em RAM — cópia instantânea e import sem disco.
//!
//! Ao copiar um clip (Ctrl+C / menu) ou ao iniciar um drag de um
//! arquivo de áudio, guardamos aqui TUDO que é preciso para criar
//! uma cópia sem reler o disco:
//!
//! - metadados (nome, duração, comprimento em beats)
//! - waveform decodificada (picos + mono + imagem RGBA)
//! - handle de áudio pré-carregado no engine (BufferCache)
//!
//! A colagem/insersão vira uma operação puramente em memória:
//! nenhum reprocessamento, nenhuma leitura de disco.

use crate::waveform::CachedWave;
use daw_media::waveform::WaveformData;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

/// Entrada do cache: tudo o que uma cópia de um sample precisa.
#[derive(Clone)]
#[allow(dead_code)] // `path`/`decoded` ficam para uso futuro (recorte de fonte)
pub struct CachedSample {
    /// Nome exibido (stem do arquivo).
    pub name: String,
    /// Path original (persistido no clip).
    pub path: String,
    /// Duração real do arquivo em segundos.
    pub duration_seconds: f32,
    /// Dados decodificados (mono p/ desenho + interleaved p/ motor).
    ///
    /// Guardamos o mono dentro de CachedWave; o interleaved fica
    /// aqui porque a colagem pode precisar recortar a fonte quando
    /// o clip copiado estava com offset (`source_start_beats`).
    pub decoded: std::sync::Arc<WaveformData>,
    /// Waveform pronta para a UI (picos + mono + imagem RGBA).
    pub wave: CachedWave,
}

/// Cache global em memória: path → sample completo.
pub type SampleCache = HashMap<String, CachedSample>;

/// Decodifica e preenche o cache (chamado no drag-start / copy).
///
/// Se o path já está cacheado, NÃO decodifica de novo — retorna o
/// que já existe. É isto que impede reprocessamento repetido.
pub fn ensure_cached(
    cache: &RefCell<SampleCache>,
    path: &std::path::Path,
    bpm: f32,
) -> Option<CachedSample> {
    // Primeiro: drena o que a thread de fundo já terminou (sem I/O).
    drain_pending(cache);
    let key = norm_key(&path.to_path_buf());

    if let Some(hit) = cache.borrow().get(&key) {
        return Some(hit.clone());
    }

    let decoded_raw = daw_media::waveform::decode_audio(path).ok()?;
    let duration = decoded_raw.duration_seconds;
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };
    let length_beats = (duration * bpm / 60.0).max(0.01);

    // Mesma resolução da importação (importer.rs): densa mas com teto.
    let points = ((length_beats * 24.0) as usize).clamp(256, 2048);
    let peaks = daw_media::waveform::waveform_points(&decoded_raw, points);
    let img = daw_media::waveform_image::waveform_image_rgba(&peaks, 0, false);

    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();

    let entry = CachedSample {
        name,
        path: key.clone(),
        duration_seconds: duration,
        decoded: std::sync::Arc::new(decoded_raw),
        wave: CachedWave {
            peaks,
            mono: Vec::new(),
            image: img,
        },
    };
    cache.borrow_mut().insert(key, entry.clone());
    Some(entry)
}
/// Key de cache normalizada (lowercase) — Windows é case-insensitive.
pub fn norm_key(p: &PathBuf) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Pré-decodifica um arquivo numa THREAD DE FUNDO e guarda no cache.
///
/// Chamado quando o cursor com um arquivo do SO entra na janela
/// (HoveredFile): enquanto o usuário ainda está arrastando, o decode
/// já acontece — quando ele solta, o sample está em RAM.
///
/// A thread de fundo escreve no buffer global PENDING (Mutex); a
/// main thread o drena em `ensure_cached`/`take_pending`, sem
/// bloquear a UI em nenhum momento.
static PENDING: std::sync::Mutex<Option<std::collections::HashMap<String, CachedSample>>> =
    std::sync::Mutex::new(None);

/// Move do buffer global para o cache da main thread (sem decode,
/// sem I/O). Chame antes de ler o cache.
pub fn drain_pending(cache: &RefCell<SampleCache>) {
    let Ok(mut pending) = PENDING.lock() else {
        return;
    };
    if let Some(map) = pending.take() {
        cache.borrow_mut().extend(map);
    }
}

pub fn spawn_prefetch(cache: &RefCell<SampleCache>, path: &std::path::Path, bpm: f32) {
    let key = norm_key(&path.to_path_buf());
    // Já cacheado ou já decodificando? Não repete (requisito:
    // nunca reprocessar o mesmo sample).
    if cache.borrow().contains_key(&key)
        || PENDING
            .lock()
            .map(|p| p.as_ref().is_some_and(|m| m.contains_key(&key)))
            .unwrap_or(false)
    {
        return;
    }
    let path_bg = path.to_path_buf();
    std::thread::spawn(move || {
        if let Some(entry) = decode_entry(&path_bg, bpm) {
            if let Ok(mut pending) = PENDING.lock() {
                pending
                    .get_or_insert_with(Default::default)
                    .insert(key, entry);
            }
        }
    });
}

/// Decodifica + agrega waveform de um arquivo (rota compartilhada
/// pela thread de fundo e pelo caminho síncrono).
fn decode_entry(path: &std::path::Path, bpm: f32) -> Option<CachedSample> {
    let decoded_raw = daw_media::waveform::decode_audio(path).ok()?;
    let duration = decoded_raw.duration_seconds;
    let bpm = if bpm > 0.0 { bpm } else { 120.0 };
    let length_beats = (duration * bpm / 60.0).max(0.01);
    let points = ((length_beats * 24.0) as usize).clamp(256, 2048);
    let peaks = daw_media::waveform::waveform_points(&decoded_raw, points);
    let img = daw_media::waveform_image::waveform_image_rgba(&peaks, 0, false);
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Audio Clip")
        .to_string();
    Some(CachedSample {
        name,
        path: path.to_string_lossy().to_string(),
        duration_seconds: duration,
        decoded: std::sync::Arc::new(decoded_raw),
        wave: CachedWave {
            peaks,
            mono: Vec::new(),
            image: img,
        },
    })
}
