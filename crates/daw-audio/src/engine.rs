use std::sync::{Arc, Mutex};

use daw_core::Transport;

use crate::backend::{AudioConfig, AudioError, CpalBackend};
use crate::buffer::BufferCache;
use crate::mixer::{clamp_stereo, mix_stereo_frame, track_is_active};

/// Snapshot do estado de um clip para o audio thread.
/// Não referencia o Project diretamente (que não é Send).
#[derive(Clone)]
struct ClipSnapshot {
    id: u64,
    track_id: u64,
    start_beat: u32,
    length_beats: u32,
}

/// Snapshot do estado de uma track para o audio thread.
#[derive(Clone)]
struct TrackSnapshot {
    id: u64,
    volume: f32,
    pan: f32,
    muted: bool,
    soloed: bool,
}

/// Snapshot do projeto para a thread de áudio.
/// Contém apenas os dados necessários para mix, sem referências ao Project.
#[derive(Clone, Default)]
pub struct ProjectSnapshot {
    tracks: Vec<TrackSnapshot>,
    clips: Vec<ClipSnapshot>,
}

impl ProjectSnapshot {
    /// Atualiza o snapshot a partir do Project (chamar na thread principal).
    pub fn update_from(&mut self, project: &daw_core::Project) {
        self.tracks = project
            .tracks
            .iter()
            .map(|t| TrackSnapshot {
                id: t.id,
                volume: t.mixer.volume,
                pan: t.mixer.pan,
                muted: t.mixer.muted,
                soloed: t.mixer.soloed,
            })
            .collect();

        self.clips = project
            .clips
            .iter()
            .filter(|c| c.file_path.is_some() && c.kind == daw_core::ClipKind::Audio)
            .map(|c| ClipSnapshot {
                id: c.id,
                track_id: c.track_id,
                start_beat: c.start_beat,
                length_beats: c.length_beats,
            })
            .collect();
    }
}

/// Audio engine profissional.
///
/// Possui a stream CPAL e avança o Transport a cada callback de áudio.
/// A UI apenas lê `transport.position_ticks` (nunca avança).
pub struct AudioEngine {
    _backend: CpalBackend,
    output_config: AudioConfig,
    snapshot: Arc<Mutex<ProjectSnapshot>>,
    buffer_cache: Arc<Mutex<BufferCache>>,
}

impl AudioEngine {
    /// Cria e inicia o engine.
    ///
    /// Retorna:
    /// - `AudioEngine` (mantém a stream viva)
    /// - `Arc<Mutex<ProjectSnapshot>>` — atualizar com `update_from()` periodicamente
    /// - `Arc<Mutex<BufferCache>>` — preencher com clips antes de tocar
    pub fn start(
        transport: Arc<Mutex<Transport>>,
        output_sample_rate: u32,
    ) -> Result<
        (
            Self,
            Arc<Mutex<ProjectSnapshot>>,
            Arc<Mutex<BufferCache>>,
        ),
        AudioError,
    > {
        let snapshot = Arc::new(Mutex::new(ProjectSnapshot::default()));
        let buffer_cache = Arc::new(Mutex::new(BufferCache::new(output_sample_rate)));

        let snapshot_cb = snapshot.clone();
        let buffer_cb = buffer_cache.clone();
        let transport_cb = transport.clone();
        let rate_for_cb = output_sample_rate;

        let backend = CpalBackend::new(move |output: &mut [f32]| {
            Self::audio_callback(
                output,
                &transport_cb,
                &snapshot_cb,
                &buffer_cb,
                rate_for_cb,
            );
        })?;

        // Use real device sample rate if we got a valid config back
        let actual_rate = backend.config().sample_rate;
        // Update the buffer cache with the real rate if it differs
        if actual_rate != output_sample_rate {
            if let Ok(mut cache) = buffer_cache.lock() {
                *cache = BufferCache::new(actual_rate);
            }
        }

        let output_config = AudioConfig {
            sample_rate: backend.config().sample_rate,
            buffer_size: backend.config().buffer_size,
            channels: backend.config().channels,
        };

        let engine = Self {
            _backend: backend,
            output_config,
            snapshot: snapshot.clone(),
            buffer_cache: buffer_cache.clone(),
        };

        Ok((engine, snapshot, buffer_cache))
    }

    pub fn output_config(&self) -> &AudioConfig {
        &self.output_config
    }

    /// Atualiza o snapshot do projeto (chamar na thread principal).
    pub fn update_snapshot(&self, project: &daw_core::Project) {
        if let Ok(mut snap) = self.snapshot.lock() {
            snap.update_from(project);
        }
    }

    /// Pré-carrega os samples de todos os clips de áudio do projeto.
    /// Chamar na thread principal (pode demorar para decodificar).
    pub fn preload_clips(&self, project: &daw_core::Project) {
        let mut cache = match self.buffer_cache.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        for clip in &project.clips {
            if clip.file_path.is_some() && clip.kind == daw_core::ClipKind::Audio {
                cache.get_or_load(clip);
            }
        }
    }

    /// Callback de áudio — roda na thread de áudio em tempo real.
    fn audio_callback(
        output: &mut [f32],
        transport: &Arc<Mutex<Transport>>,
        snapshot: &Arc<Mutex<ProjectSnapshot>>,
        buffer_cache: &Arc<Mutex<BufferCache>>,
        sample_rate: u32,
    ) {
        let frames = output.len() / 2;
        let (bpm, ppq, position_at_start);

        let sample_rate = sample_rate as u64;

        // 1. Lê transport e avança
        {
            let mut t = match transport.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if !t.playing {
                output.fill(0.0);
                return;
            }

            bpm = t.bpm;
            ppq = t.ppq;
            position_at_start = t.position_ticks;

            let ticks_per_sample = (bpm as f64 / 60.0) * ppq as f64 / sample_rate as f64;
            let delta = (frames as f64 * ticks_per_sample).round() as u64;
            t.advance(delta);
        }

        let ticks_per_sample = (bpm as f64 / 60.0) * ppq as f64 / sample_rate as f64;

        // 2. Lê snapshot
        let snap = match snapshot.lock() {
            Ok(guard) => guard,
            Err(_) => {
                output.fill(0.0);
                return;
            }
        };

        let any_soloed = snap.tracks.iter().any(|t| t.soloed);

        // 3. Mix — para cada frame
        for frame in 0..frames {
            let mut left = 0.0f32;
            let mut right = 0.0f32;

            let frame_ticks =
                position_at_start + (frame as f64 * ticks_per_sample).round() as u64;

            for track in &snap.tracks {
                if !track_is_active(track.muted, track.soloed, any_soloed) {
                    continue;
                }

                for clip in snap.clips.iter().filter(|c| c.track_id == track.id) {
                    let clip_start_ticks = clip.start_beat as u64 * ppq as u64;
                    let clip_end_ticks =
                        clip_start_ticks + clip.length_beats as u64 * ppq as u64;

                    if frame_ticks >= clip_start_ticks && frame_ticks < clip_end_ticks {
                        let offset_ticks = frame_ticks - clip_start_ticks;

                        // Converte ticks → índice de sample no buffer
                        // ticks_per_sample = (bpm/60) * ppq / sample_rate
                        // sample_index = offset_ticks / ticks_per_sample
                        let sample_idx = (offset_ticks as f64 / ticks_per_sample).round() as u64;

                        // Lê sample do buffer cache
                        let sample = if let Ok(cache) = buffer_cache.try_lock() {
                            if let Some(buf) = cache.get_by_id(clip.id) {
                                buf.read(sample_idx)
                            } else {
                                // Buffer não carregado ainda → silêncio
                                0.0
                            }
                        } else {
                            // Lock contended → silêncio neste frame
                            0.0
                        };

                        mix_stereo_frame(
                            sample,
                            track.volume,
                            track.pan,
                            &mut left,
                            &mut right,
                        );
                    }
                }
            }

            let (l, r) = clamp_stereo(left, right);
            output[frame * 2] = l;
            output[frame * 2 + 1] = r;
        }
    }
}
