use std::sync::{Arc, Mutex};

use daw_core::Transport;

use crate::backend::{AudioConfig, AudioError, CpalBackend};
use crate::mixer::{clamp_stereo, mix_stereo_frame, track_is_active};

/// Snapshot do estado de um clip para o audio thread.
/// Não referencia o Project diretamente (que não é Send).
#[derive(Clone)]
struct ClipSnapshot {
    track_id: u64,
    start_beat: u32,
    length_beats: u32,
    has_audio: bool,
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
            .map(|c| ClipSnapshot {
                track_id: c.track_id,
                start_beat: c.start_beat,
                length_beats: c.length_beats,
                has_audio: c.file_path.is_some() && c.kind == daw_core::ClipKind::Audio,
            })
            .collect();
    }
}

/// Audio engine profissional.
///
/// Possui a stream CPAL e avança o Transport a cada callback de áudio.
/// A UI apenas lê `transport.position_ticks` (nunca avança).
///
/// O `Project` NÃO é acessado diretamente na thread de áudio.
/// Em vez disso, usa-se um `ProjectSnapshot` atualizado pela thread principal.
pub struct AudioEngine {
    _backend: CpalBackend,
    output_config: AudioConfig,
    snapshot: Arc<Mutex<ProjectSnapshot>>,
}

impl AudioEngine {
    /// Cria e inicia o engine.
    ///
    /// O `transport` deve ser `Arc<Mutex<Transport>>`.
    /// Retorna o engine e um `Arc<Mutex<ProjectSnapshot>>` que deve ser
    /// atualizado periodicamente pela thread principal.
    pub fn start(
        transport: Arc<Mutex<Transport>>,
    ) -> Result<(Self, Arc<Mutex<ProjectSnapshot>>), AudioError> {
        let snapshot = Arc::new(Mutex::new(ProjectSnapshot::default()));
        let snapshot_cb = snapshot.clone();
        let transport_cb = transport.clone();

        let backend = CpalBackend::new(move |output: &mut [f32]| {
            Self::audio_callback(output, &transport_cb, &snapshot_cb);
        })?;

        let output_config = AudioConfig {
            sample_rate: backend.config().sample_rate,
            buffer_size: backend.config().buffer_size,
            channels: backend.config().channels,
        };

        let engine = Self {
            _backend: backend,
            output_config,
            snapshot: snapshot.clone(),
        };

        Ok((engine, snapshot))
    }

    pub fn output_config(&self) -> &AudioConfig {
        &self.output_config
    }

    pub fn update_snapshot(&self, project: &daw_core::Project) {
        if let Ok(mut snap) = self.snapshot.lock() {
            snap.update_from(project);
        }
    }

    /// Callback de áudio — roda na thread de áudio em tempo real.
    fn audio_callback(
        output: &mut [f32],
        transport: &Arc<Mutex<Transport>>,
        snapshot: &Arc<Mutex<ProjectSnapshot>>,
    ) {
        let frames = output.len() / 2;
        let (bpm, ppq, position_at_start);

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

            let sample_rate = 48000u64;
            let ticks_per_sample = (bpm as f64 / 60.0) * ppq as f64 / sample_rate as f64;
            let delta = (frames as f64 * ticks_per_sample).round() as u64;
            t.advance(delta);
        }

        let snap = match snapshot.lock() {
            Ok(guard) => guard,
            Err(_) => {
                output.fill(0.0);
                return;
            }
        };

        let any_soloed = snap.tracks.iter().any(|t| t.soloed);

        for frame in 0..frames {
            let mut left = 0.0f32;
            let mut right = 0.0f32;

            let sample_rate = 48000u64;
            let ticks_per_sample = (bpm as f64 / 60.0) * ppq as f64 / sample_rate as f64;
            let frame_ticks =
                position_at_start + (frame as f64 * ticks_per_sample).round() as u64;

            for track in &snap.tracks {
                if !track_is_active(track.muted, track.soloed, any_soloed) {
                    continue;
                }

                for clip in snap
                    .clips
                    .iter()
                    .filter(|c| c.track_id == track.id && c.has_audio)
                {
                    let clip_start_ticks = clip.start_beat as u64 * ppq as u64;
                    let clip_end_ticks =
                        clip_start_ticks + clip.length_beats as u64 * ppq as u64;

                    if frame_ticks >= clip_start_ticks && frame_ticks < clip_end_ticks {
                        // TODO: ler do BufferCache em vez de tone sintético
                        let beats_pos = (frame_ticks - clip_start_ticks) as f64 / ppq as f64;
                        let sample =
                            (beats_pos * 440.0 * 2.0 * std::f64::consts::PI / sample_rate as f64)
                                .sin() as f32
                                * 0.3;

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
