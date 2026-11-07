//! Worker de render do timewarp: o keylock em background.
//!
//! Fluxo HIBRIDO (o mesmo do Cubase durante um gesto de tempo):
//!
//! 1. o tempo muda -> o MOTOR ja toca o original em varispeed: o audio
//!    acompanha a grade no mesmo frame, com o pitch acompanhando;
//! 2. este worker renderiza a versao com pitch PRESERVADO, partindo do
//!    buffer que o motor ja tem em memoria (sem reabrir o arquivo);
//! 3. o render pronto e inserido no cache do motor e o snapshot e
//!    republicado -> o clip passa a tocar o buffer esticado, 1:1.
//!
//! Nada disto toca a thread de audio: o worker so conversa com o cache
//! (Mutex) e a publicacao e uma troca de `Arc`.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use daw_audio::stretch::{render_stretch, stretch_target_for};
use daw_audio::{AudioBufferKind, EngineHandle};
use daw_core::project::Project;

/// Quantos renders ficam publicados no cache do motor.
///
/// Cada alvo distinto (um BPM, um warp) e um buffer completo em RAM. Um
/// arrasto continuo de tempo gera muitos alvos; manter os mais recentes e
/// o suficiente para o que esta na tela e evita crescer sem limite.
const MAX_PUBLISHED: usize = 24;

/// Render pronto, a espera da publicacao na thread principal.
struct Ready {
    key: String,
    buffer: AudioBufferKind,
}

/// Renders terminados (produtor: threads do worker).
static READY: Mutex<Vec<Ready>> = Mutex::new(Vec::new());

/// Alvos em render agora (evita disparar o mesmo alvo duas vezes).
static IN_FLIGHT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Alvos que JA FALHARAM (nao insistir).
///
/// Sem isto um render que falha (formato inesperado, ratio invalido) seria
/// re-pedido a cada tick do bridge, criando uma thread nova a cada 90 ms.
static FAILED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Chaves ja publicadas, da mais antiga para a mais recente (LRU).
static PUBLISHED: Mutex<Option<VecDeque<String>>> = Mutex::new(None);

/// Marca o alvo como em render. `false` = ja estava.
fn mark_in_flight(key: &str) -> bool {
    let Ok(mut guard) = IN_FLIGHT.lock() else {
        return false;
    };

    guard.get_or_insert_with(HashSet::new).insert(key.to_string())
}

fn clear_in_flight(key: &str) {
    if let Ok(mut guard) = IN_FLIGHT.lock() {
        if let Some(set) = guard.as_mut() {
            set.remove(key);
        }
    }
}

/// Registra um alvo como falho, para nao repetir o pedido.
fn mark_failed(key: &str) {
    if let Ok(mut guard) = FAILED.lock() {
        guard.get_or_insert_with(HashSet::new).insert(key.to_string());
    }
}

fn is_failed(key: &str) -> bool {
    FAILED
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .map(|set| set.contains(key))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Publica os renders prontos no cache do motor.
///
/// Devolve quantos foram publicados. Republica o snapshot UMA vez quando
/// algum entra (a RT so ve o estado novo inteiro). Roda na thread principal.
pub fn publish_ready(handle: &EngineHandle, project: &Project) -> usize {
    let ready: Vec<Ready> = {
        let Ok(mut queue) = READY.lock() else {
            return 0;
        };
        std::mem::take(&mut *queue)
    };

    if ready.is_empty() {
        return 0;
    }

    let mut published = 0;

    for item in ready {
        clear_in_flight(&item.key);

        if handle.insert_buffer(&item.key, item.buffer) {
            published += 1;
            remember_published(handle, item.key);
        }
    }

    if published > 0 {
        handle.sync_project(project);
    }

    published
}

/// Guarda a chave no LRU e descarta os renders antigos demais.
fn remember_published(handle: &EngineHandle, key: String) {
    let Ok(mut guard) = PUBLISHED.lock() else {
        return;
    };

    let queue = guard.get_or_insert_with(VecDeque::new);
    queue.push_back(key);

    while queue.len() > MAX_PUBLISHED {
        if let Some(old) = queue.pop_front() {
            handle.evict_buffer(&old);
        }
    }
}

/// Pede o render dos clips musicais cujo alvo ainda nao esta pronto.
///
/// Devolve quantos pedidos foram disparados. Roda na thread principal; cada
/// pedido sai numa thread propria e devolve o buffer por [`publish_ready`].
pub fn request_missing(handle: &EngineHandle, project: &Project) -> usize {
    let mut requested = 0;

    for clip in project.clips.iter() {
        if clip.is_midi() || !clip.is_musical() {
            continue;
        }

        let Some(path) = clip.file_path.as_deref() else {
            continue;
        };

        let Some(source) = handle.buffer_for(path) else {
            continue;
        };

        let rate = source.sample_rate().max(1) as f64;
        let natural_seconds = source.frames() as f64 / rate;

        let Some(target) = stretch_target_for(clip, &project.tempo_map, natural_seconds) else {
            continue;
        };

        // Ratio praticamente 1.0: tocar o original JA e a versao correta.
        if !target.needs_render() {
            continue;
        }

        let key = target.key();

        if handle.has_buffer(&key) || is_failed(&key) || !mark_in_flight(&key) {
            continue;
        }

        requested += 1;

        // A closure leva uma COPIA da chave: a de fora continua
        // valida para limpar o in-flight se a thread nem subir.
        let thread_key = key.clone();

        let spawned = std::thread::Builder::new()
            .name("timewarp-render".into())
            .spawn(move || {
                match render_stretch(&source, &target) {
                    Ok(buffer) => {
                        let ready = Ready {
                            key: thread_key.clone(),
                            buffer: AudioBufferKind::Memory(Arc::new(buffer)),
                        };

                        match READY.lock() {
                            Ok(mut queue) => queue.push(ready),
                            Err(_) => clear_in_flight(&thread_key),
                        }
                    }
                    Err(err) => {
                        eprintln!("[timewarp] render falhou ({thread_key}): {err}");
                        clear_in_flight(&thread_key);
                        mark_failed(&thread_key);
                    }
                }
            });

        if spawned.is_err() {
            clear_in_flight(&key);
            requested -= 1;
        }
    }

    requested
}