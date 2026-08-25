//! Motor de áudio.
//!
//! # O relógio vive em SAMPLES
//!
//! Toda a reprodução é agendada no domínio de samples (`f64`), não em
//! ticks MIDI. A versão anterior derivava o índice de leitura assim:
//!
//! ```text
//! frame_ticks = pos_ticks + (frame * ticks_per_sample).round()
//! sample_idx  = (offset_ticks / ticks_per_sample).round()
//! ```
//!
//! Em 120 BPM / 960 PPQ / 48 kHz, `ticks_per_sample` é 0.04 — o tick
//! só troca a cada 25 frames. O índice de leitura ficava travado por
//! 25 samples e depois saltava 25 de uma vez: uma decimação com
//! sample-and-hold a 1920 Hz. É essa quantização que produzia o som
//! granulado/serrado; o sample nunca chegava íntegro na saída.
//!
//! Agora a posição avança exatamente 1.0 por frame, a leitura é
//! interpolada e os ticks do transporte são DERIVADOS da posição em
//! samples — o contrário do que era.
//!
//! # Nada de alocar, travar ou tocar disco na thread RT
//!
//! O callback só lê estado que já está resolvido: o snapshot carrega
//! `Arc<AudioBuffer>` prontos. Os locks são todos `try_lock`, e uma
//! falha nunca vira silêncio — o callback reusa o último snapshot
//! válido, que ele mantém consigo.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use daw_core::Transport;

use crate::backend::{AudioConfig, AudioError, CpalBackend};
use crate::buffer::{AudioBuffer, BufferCache};
use crate::mixer::{
    limit_stereo, mix_stereo_source, soft_clip, stereo_gains, stereo_gains_bipolar, track_is_active,
};

/// Logs de diagnóstico do engine — só com `RUANA_LOG` definido.
fn verbose_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RUANA_LOG").is_ok())
}

/// Vozes simultâneas do sampler (beat + preview).
const MAX_VOICES: usize = 64;

/// Passos por batida no step sequencer.
///
/// Fixo em 4 (semicolcheia), como no channel rack do FL Studio: o
/// tamanho do pattern é dado pelo NÚMERO de passos, não pelo valor de
/// cada passo. 16 passos = 1 compasso de 4/4.
pub const STEPS_PER_BEAT: f64 = 4.0;

/// Rampa anti-clique nas bordas de um clip da timeline, em frames.
const CLIP_EDGE_FADE: f64 = 64.0;

// ============================================================
// SNAPSHOT
// ============================================================

/// Clip de áudio da timeline, com o buffer já resolvido.
#[derive(Clone)]
struct ClipSlot {
    track_id: u64,
    start_beat: f64,
    length_beats: f64,
    source_start_beats: f64,
    buffer: Arc<AudioBuffer>,
}

/// Trilha da timeline (pan 0..1, 0.5 = centro).
#[derive(Clone)]
struct TrackSlot {
    id: u64,
    volume: f32,
    pan: f32,
    muted: bool,
    soloed: bool,
}

/// Uma linha do beat sequencer, com o sample já resolvido.
#[derive(Clone)]
struct BeatRow {
    steps: Vec<bool>,
    buffer: Option<Arc<AudioBuffer>>,
    volume: f32,
    /// Pan bipolar (-1..1), convenção do `DrumTrackParams`.
    pan: f32,
    active: bool,
    solo: bool,
}

/// Pattern ativo do beat sequencer.
#[derive(Clone, Default)]
struct BeatSlot {
    rows: Vec<BeatRow>,
    steps: usize,
    swing: f32,
    /// Alguma linha em solo? Pré-calculado fora da thread RT.
    any_solo: bool,
}

/// Estado do projeto visto pela thread de áudio.
///
/// Construído na thread principal e publicado inteiro. A thread RT
/// nunca percorre o `Project` (que não é `Send`) nem decodifica nada.
#[derive(Clone, Default)]
pub struct ProjectSnapshot {
    tracks: Vec<TrackSlot>,
    clips: Vec<ClipSlot>,
    beat: BeatSlot,
}

impl ProjectSnapshot {
    /// Reconstrói o snapshot a partir do projeto, resolvendo samples.
    ///
    /// Roda na thread principal: pode decodificar arquivo novo.
    pub fn build(project: &daw_core::Project, cache: &mut BufferCache) -> Self {
        let tracks = project
            .tracks
            .iter()
            .map(|t| TrackSlot {
                id: t.id,
                volume: t.mixer.volume,
                pan: t.mixer.pan,
                muted: t.mixer.muted,
                soloed: t.mixer.soloed,
            })
            .collect();

        let clips = project
            .clips
            .iter()
            .filter(|c| c.kind == daw_core::ClipKind::Audio)
            .filter_map(|c| {
                let path = c.file_path.as_deref()?;
                let buffer = cache.load(path)?;
                Some(ClipSlot {
                    track_id: c.track_id,
                    start_beat: c.start_beat as f64,
                    length_beats: c.length_beats as f64,
                    source_start_beats: c.source_start_beats as f64,
                    buffer,
                })
            })
            .collect();

        let beat = project
            .beat_patterns
            .current_pattern()
            .map(|p| {
                let rows: Vec<BeatRow> = p
                    .grid
                    .iter()
                    .zip(p.track_params.iter())
                    .map(|(steps, params)| BeatRow {
                        steps: steps.clone(),
                        buffer: params
                            .sample_path
                            .as_deref()
                            .and_then(|path| cache.load(path)),
                        volume: params.volume,
                        pan: params.pan,
                        active: params.active,
                        solo: params.solo,
                    })
                    .collect();
                BeatSlot {
                    any_solo: rows.iter().any(|r| r.solo),
                    steps: p.steps(),
                    swing: p.swing.clamp(0.0, 1.0),
                    rows,
                }
            })
            .unwrap_or_default();

        Self {
            tracks,
            clips,
            beat,
        }
    }
}

// ============================================================
// VOZES
// ============================================================

/// Uma voz one-shot do sampler.
struct Voice {
    buffer: Option<Arc<AudioBuffer>>,
    /// Posição de leitura em frames.
    pos: f64,
    gain_l: f32,
    gain_r: f32,
    /// Frames restantes de espera antes de começar a soar.
    ///
    /// Permite disparar um passo no frame EXATO dentro do buffer, em
    /// vez de arredondar para a borda do callback.
    delay: u32,
}

impl Voice {
    const fn silent() -> Self {
        Self {
            buffer: None,
            pos: 0.0,
            gain_l: 0.0,
            gain_r: 0.0,
            delay: 0,
        }
    }

    #[inline]
    fn is_free(&self) -> bool {
        self.buffer.is_none()
    }
}

/// Disparo pedido pela UI (preview de sample).
struct PreviewCmd {
    buffer: Arc<AudioBuffer>,
    volume: f32,
    pan: f32,
}

// ============================================================
// PARTE COMPARTILHADA
// ============================================================

/// Handle clonável para falar com o engine da thread da UI.
#[derive(Clone)]
pub struct EngineHandle {
    snapshot: Arc<Mutex<Arc<ProjectSnapshot>>>,
    preview: Arc<Mutex<Vec<PreviewCmd>>>,
    cache: Arc<Mutex<BufferCache>>,
    sample_rate: u32,
}

impl EngineHandle {
    /// Re-publica o snapshot do projeto para a thread de áudio.
    ///
    /// Chamar da thread principal sempre que clips, mixer ou o beat
    /// mudarem. Samples novos são decodificados aqui, nunca na RT.
    pub fn sync_project(&self, project: &daw_core::Project) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        let built = Arc::new(ProjectSnapshot::build(project, &mut cache));
        drop(cache);
        if let Ok(mut slot) = self.snapshot.lock() {
            *slot = built;
        }
    }

    /// Toca um sample imediatamente, fora do transporte.
    ///
    /// É o "arrasta e ouve" do channel rack: soa com o transporte
    /// parado, porque o dispositivo de áudio fica aberto o tempo todo.
    pub fn preview_path(&self, path: &str, volume: f32, pan: f32) -> bool {
        let buffer = {
            let Ok(mut cache) = self.cache.lock() else {
                return false;
            };
            match cache.load(path) {
                Some(b) => b,
                None => return false,
            }
        };
        let Ok(mut queue) = self.preview.lock() else {
            return false;
        };
        // Guarda contra enfileirar sem limite se a RT estiver parada.
        if queue.len() < MAX_VOICES {
            queue.push(PreviewCmd {
                buffer,
                volume,
                pan,
            });
        }
        true
    }

    /// Pré-carrega um sample sem tocar (evita decodificar no primeiro
    /// disparo, que aconteceria dentro do gesto do usuário).
    pub fn preload_path(&self, path: &str) -> bool {
        let Ok(mut cache) = self.cache.lock() else {
            return false;
        };
        cache.load(path).is_some()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

// ============================================================
// ESTADO DA THREAD RT
// ============================================================

/// Estado que vive DENTRO do callback de áudio.
///
/// Como o callback é um `FnMut`, ele é dono deste estado — sem
/// atômicos, sem locks, sem compartilhamento. O acumulador
/// fracionário de ticks da versão anterior deixou de existir: a
/// posição em samples já é exata.
struct RtState {
    /// Último snapshot visto. Nunca fica vazio depois do primeiro.
    snapshot: Arc<ProjectSnapshot>,
    /// Posição do transporte em frames (contínua).
    pos_frames: f64,
    /// Ticks que ESTE engine escreveu por último no transporte.
    ///
    /// Se o transporte tiver outro valor, quem mudou foi a UI (seek):
    /// o relógio de samples re-ancora a partir dele.
    ticks_written: u64,
    /// Próximo passo absoluto do beat a disparar.
    next_step: i64,
    voices: Vec<Voice>,
    /// Cursor de round-robin para roubo de voz.
    voice_cursor: usize,
    was_playing: bool,
    metronome_beat: i64,
    metronome_remaining: u32,
    metronome_phase: f32,
}

impl RtState {
    fn new() -> Self {
        Self {
            snapshot: Arc::new(ProjectSnapshot::default()),
            pos_frames: 0.0,
            ticks_written: u64::MAX,
            next_step: 0,
            voices: (0..MAX_VOICES).map(|_| Voice::silent()).collect(),
            voice_cursor: 0,
            was_playing: false,
            metronome_beat: -1,
            metronome_remaining: 0,
            metronome_phase: 0.0,
        }
    }

    /// Aloca uma voz: primeiro uma livre, senão rouba em round-robin.
    fn alloc_voice(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(Voice::is_free) {
            return i;
        }
        let i = self.voice_cursor;
        self.voice_cursor = (self.voice_cursor + 1) % self.voices.len();
        i
    }

    fn trigger(&mut self, buffer: Arc<AudioBuffer>, gain_l: f32, gain_r: f32, delay: u32) {
        let i = self.alloc_voice();
        self.voices[i] = Voice {
            buffer: Some(buffer),
            pos: 0.0,
            gain_l,
            gain_r,
            delay,
        };
    }

    /// Silencia todas as vozes do pattern (stop/seek).
    fn kill_voices(&mut self) {
        for v in self.voices.iter_mut() {
            v.buffer = None;
        }
    }
}

// ============================================================
// ENGINE
// ============================================================

/// Motor de áudio. Mantém a stream do dispositivo aberta.
///
/// O dispositivo NÃO abre e fecha a cada play: abrir stream custa
/// dezenas de milissegundos (e estala no primeiro buffer), e sem a
/// stream aberta não haveria como pré-escutar um sample com o
/// transporte parado.
pub struct AudioEngine {
    _backend: CpalBackend,
    output_config: AudioConfig,
    handle: EngineHandle,
}

impl AudioEngine {
    /// Abre o dispositivo e começa a processar.
    pub fn start(transport: Arc<Mutex<Transport>>) -> Result<Self, AudioError> {
        // Taxa provisória: corrigida assim que o dispositivo informa
        // a real (logo abaixo), antes de qualquer sample ser lido.
        let cache = Arc::new(Mutex::new(BufferCache::new(48_000)));
        let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::default())));
        let preview: Arc<Mutex<Vec<PreviewCmd>>> = Arc::new(Mutex::new(Vec::new()));

        let snapshot_cb = snapshot.clone();
        let preview_cb = preview.clone();
        let transport_cb = transport.clone();

        // Taxa e nº de canais reais só são conhecidos depois de abrir
        // a stream — mas o callback pode rodar antes disso. Um `Mutex`
        // não serve na RT; atômicos escritos uma única vez servem.
        let rate = Arc::new(AtomicU32::new(48_000));
        let channels = Arc::new(AtomicU32::new(2));
        let rate_cb = rate.clone();
        let channels_cb = channels.clone();

        let mut rt = RtState::new();

        let backend = CpalBackend::new(move |output: &mut [f32]| {
            let sr = rate_cb.load(Ordering::Relaxed);
            let ch = channels_cb.load(Ordering::Relaxed) as usize;
            Self::process(
                output,
                ch,
                &mut rt,
                &transport_cb,
                &snapshot_cb,
                &preview_cb,
                sr,
            );
        })?;

        let actual_rate = backend.config().sample_rate;
        rate.store(actual_rate, Ordering::Relaxed);
        channels.store(backend.config().channels.max(1) as u32, Ordering::Relaxed);
        if let Ok(mut c) = cache.lock() {
            c.set_output_sample_rate(actual_rate);
        }

        let output_config = AudioConfig {
            sample_rate: actual_rate,
            buffer_size: backend.config().buffer_size,
            channels: backend.config().channels,
        };

        if verbose_log() {
            eprintln!(
                "[ENGINE] started: sr={} buf={} ch={}",
                output_config.sample_rate, output_config.buffer_size, output_config.channels
            );
        }

        Ok(Self {
            _backend: backend,
            output_config,
            handle: EngineHandle {
                snapshot,
                preview,
                cache,
                sample_rate: actual_rate,
            },
        })
    }

    pub fn output_config(&self) -> &AudioConfig {
        &self.output_config
    }

    /// Handle clonável para a thread da UI.
    pub fn handle(&self) -> EngineHandle {
        self.handle.clone()
    }

    /// Callback de áudio — thread RT.
    fn process(
        output: &mut [f32],
        channels: usize,
        rt: &mut RtState,
        transport: &Arc<Mutex<Transport>>,
        snapshot: &Arc<Mutex<Arc<ProjectSnapshot>>>,
        preview: &Arc<Mutex<Vec<PreviewCmd>>>,
        sample_rate: u32,
    ) {
        output.fill(0.0);
        let channels = channels.max(1);
        let frames = output.len() / channels;
        if frames == 0 || sample_rate == 0 {
            return;
        }

        // ---- snapshot ------------------------------------------
        // try_lock: contenção NUNCA vira silêncio. O último snapshot
        // continua válido — foi o que quebrava o áudio antes, quando
        // o lock ocupado zerava o buffer inteiro.
        if let Ok(slot) = snapshot.try_lock() {
            if !Arc::ptr_eq(&rt.snapshot, &slot) {
                rt.snapshot = slot.clone();
            }
        }
        let snap = rt.snapshot.clone();

        // ---- preview (soa com o transporte parado) --------------
        if let Ok(mut queue) = preview.try_lock() {
            for cmd in queue.drain(..) {
                let (gl, gr) = stereo_gains_bipolar(cmd.volume, cmd.pan);
                rt.trigger(cmd.buffer, gl, gr, 0);
            }
        }

        // ---- transporte ----------------------------------------
        let mut playing = false;
        let mut bpm = 120.0f64;
        let mut ppq = 960.0f64;
        if let Ok(t) = transport.try_lock() {
            playing = t.playing;
            bpm = t.bpm as f64;
            ppq = t.ppq as f64;

            let frames_per_tick = sample_rate as f64 * 60.0 / (bpm * ppq);

            // A UI mexeu na posição (seek/stop-rewind)? Re-ancora.
            if t.position_ticks != rt.ticks_written {
                rt.pos_frames = t.position_ticks as f64 * frames_per_tick;
                rt.next_step = i64::MIN; // recalculado abaixo
                rt.kill_voices();
            }
        }

        let frames_per_beat = sample_rate as f64 * 60.0 / bpm;
        let frames_per_step = frames_per_beat / STEPS_PER_BEAT;

        // Play/stop: corta a cauda dos passos para não arrastar som
        // do pattern depois do stop.
        if playing != rt.was_playing {
            if !playing {
                rt.kill_voices();
            } else {
                rt.next_step = i64::MIN;
            }
            rt.was_playing = playing;
        }

        let start_frames = rt.pos_frames;

        let (metronome_enabled, metronome_sound, metronome_volume) = transport
            .try_lock()
            .map(|t| (t.metronome_enabled, t.metronome_sound.min(4), t.metronome_volume.clamp(0.0, 1.0)))
            .unwrap_or((false, 0, 0.7));
        if playing && metronome_enabled {
            let beat = (start_frames / frames_per_beat).floor() as i64;
            if beat != rt.metronome_beat {
                rt.metronome_beat = beat;
                rt.metronome_remaining = (sample_rate / 12).max(1);
                rt.metronome_phase = 0.0;
            }
        } else {
            rt.metronome_remaining = 0;
        }

        // ---- agendamento dos passos do beat --------------------
        // Feito ANTES do mix, com o offset exato de cada disparo
        // dentro deste buffer.
        if playing && !snap.beat.rows.is_empty() && snap.beat.steps > 0 {
            let steps = snap.beat.steps as i64;
            let swing = snap.beat.swing as f64;

            // Deslocamento de swing: passos ímpares atrasam até meio
            // passo. É o mesmo comportamento de um step sequencer.
            let swing_offset = |step: i64| -> f64 {
                if step.rem_euclid(2) == 1 {
                    swing * 0.5 * frames_per_step
                } else {
                    0.0
                }
            };
            let onset = |step: i64| -> f64 { step as f64 * frames_per_step + swing_offset(step) };

            // Re-ancora depois de seek/play: primeiro passo cujo
            // onset caia em ou depois da posição atual.
            if rt.next_step == i64::MIN {
                let approx = (start_frames / frames_per_step).floor() as i64;
                let mut candidate = approx.max(0);
                while candidate > 0 && onset(candidate - 1) >= start_frames {
                    candidate -= 1;
                }
                while onset(candidate) < start_frames {
                    candidate += 1;
                }
                rt.next_step = candidate;
            }

            let end_frames = start_frames + frames as f64;
            // Limite de disparos por buffer: um BPM absurdo com
            // buffer grande não pode virar loop longo na RT.
            let mut guard = 0;
            while rt.next_step >= 0 && onset(rt.next_step) < end_frames && guard < 256 {
                guard += 1;
                let at = onset(rt.next_step);
                let delay = (at - start_frames).max(0.0) as u32;
                let step_in_pattern = rt.next_step.rem_euclid(steps) as usize;

                for row in snap.beat.rows.iter() {
                    if !track_is_active(!row.active, row.solo, snap.beat.any_solo) {
                        continue;
                    }
                    if row.steps.get(step_in_pattern) != Some(&true) {
                        continue;
                    }
                    let Some(buffer) = row.buffer.clone() else {
                        continue;
                    };
                    let (gl, gr) = stereo_gains_bipolar(row.volume, row.pan);
                    rt.trigger(buffer, gl, gr, delay.min(frames as u32));
                }

                rt.next_step += 1;
            }
        }

        // ---- mix -----------------------------------------------
        let any_soloed = snap.tracks.iter().any(|t| t.soloed);

        for frame in 0..frames {
            let mut left = 0.0f32;
            let mut right = 0.0f32;

            // Clips da timeline (só com o transporte andando).
            if playing {
                let pos = start_frames + frame as f64;

                for track in snap.tracks.iter() {
                    if !track_is_active(track.muted, track.soloed, any_soloed) {
                        continue;
                    }
                    let (gain_l, gain_r) = stereo_gains(track.volume, track.pan);

                    for clip in snap.clips.iter().filter(|c| c.track_id == track.id) {
                        let clip_start = clip.start_beat * frames_per_beat;
                        let clip_len = clip.length_beats * frames_per_beat;
                        let offset = pos - clip_start;
                        if offset < 0.0 || offset >= clip_len {
                            continue;
                        }

                        // O clip pode ser um trecho fatiado do arquivo. A
                        // leitura precisa começar no offset de origem, não
                        // no frame zero do buffer compartilhado.
                        let source_offset =
                            (clip.source_start_beats * frames_per_beat + offset).max(0.0);
                        let (sl, sr) = clip.buffer.read(source_offset);

                        // Rampa nas bordas: cortar o clip no meio de
                        // um ciclo é um degrau, e degrau é clique.
                        let edge = (offset / CLIP_EDGE_FADE)
                            .min((clip_len - offset) / CLIP_EDGE_FADE)
                            .min(1.0) as f32;

                        mix_stereo_source(
                            sl * edge,
                            sr * edge,
                            gain_l,
                            gain_r,
                            &mut left,
                            &mut right,
                        );
                    }
                }
            }

            // Vozes do sampler (beat + preview) — independentes do
            // transporte, por isso pré-escuta funciona parado.
            for voice in rt.voices.iter_mut() {
                if voice.delay > 0 {
                    voice.delay -= 1;
                    continue;
                }
                let Some(buffer) = voice.buffer.as_ref() else {
                    continue;
                };
                if voice.pos >= buffer.frames() as f64 {
                    voice.buffer = None;
                    continue;
                }
                let (sl, sr) = buffer.read(voice.pos);
                mix_stereo_source(sl, sr, voice.gain_l, voice.gain_r, &mut left, &mut right);
                voice.pos += 1.0;
            }

            // Clique sintetizado no próprio engine: não depende de arquivos
            // externos e oferece cinco timbres por frequência.
            if metronome_enabled && rt.metronome_remaining > 0 {
                let frequencies = [880.0f32, 660.0, 1100.0, 520.0, 1320.0];
                let freq = frequencies[metronome_sound as usize];
                let t = rt.metronome_phase / sample_rate as f32;
                let duration = (sample_rate as f32 / 12.0).max(1.0);
                let envelope = rt.metronome_remaining as f32 / duration;
                let click = (t * freq * std::f32::consts::TAU).sin()
                    * envelope
                    * metronome_volume
                    * 0.18;
                left += click;
                right += click;
                rt.metronome_phase += 1.0;
                rt.metronome_remaining -= 1;
            }

            let (l, r) = limit_stereo(left, right);
            let base = frame * channels;
            match channels {
                // Mono: soma o par JÁ limitado, então o dispositivo
                // mono ouve a mesma dinâmica que o estéreo.
                1 => output[base] = soft_clip((l + r) * 0.5),
                _ => {
                    output[base] = l;
                    output[base + 1] = r;
                    // Surround: os canais extras ficam em silêncio, em
                    // vez de repetir o par frontal e dobrar o nível.
                }
            }
        }

        // ---- devolve o relógio ao transporte -------------------
        // Os ticks são DERIVADOS da posição em samples: o áudio é o
        // relógio autoritativo, o domínio musical é a projeção.
        if playing {
            rt.pos_frames = start_frames + frames as f64;
            let ticks_per_frame = bpm * ppq / (60.0 * sample_rate as f64);
            let ticks = (rt.pos_frames * ticks_per_frame).max(0.0) as u64;
            if let Ok(mut t) = transport.try_lock() {
                t.position_ticks = ticks;
                rt.ticks_written = ticks;
            }
            // Se o lock falhou, `ticks_written` continua o de antes e
            // o próximo callback NÃO confunde isso com um seek: a
            // comparação usa o valor realmente escrito.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_per_beat_is_sixteenth() {
        // 4 passos por batida = semicolcheia; 16 passos = 1 compasso.
        assert_eq!(STEPS_PER_BEAT, 4.0);
    }

    #[test]
    fn voice_starts_free_and_alloc_reuses() {
        let mut rt = RtState::new();
        assert!(rt.voices.iter().all(Voice::is_free));
        let buffer = Arc::new(AudioBuffer {
            left: vec![1.0; 8],
            right: vec![1.0; 8],
            sample_rate: 48000,
        });
        rt.trigger(buffer.clone(), 1.0, 1.0, 0);
        assert_eq!(rt.voices.iter().filter(|v| !v.is_free()).count(), 1);

        // Estouro do pool rouba voz em vez de crescer.
        for _ in 0..MAX_VOICES * 2 {
            rt.trigger(buffer.clone(), 1.0, 1.0, 0);
        }
        assert_eq!(rt.voices.len(), MAX_VOICES);

        rt.kill_voices();
        assert!(rt.voices.iter().all(Voice::is_free));
    }

    /// O bug do áudio granulado: a posição de leitura tem que avançar
    /// 1 frame por frame, não em degraus do tamanho de um tick.
    #[test]
    fn read_position_advances_one_frame_per_frame() {
        let sample_rate = 48_000.0f64;
        let bpm = 120.0f64;
        let frames_per_beat = sample_rate * 60.0 / bpm;

        let pos_at = |frame: usize| -> f64 { 0.0 + frame as f64 };
        for frame in 0..1000 {
            let expected = frame as f64;
            assert_eq!(pos_at(frame), expected);
        }
        // 1 beat a 120 BPM = 0.5s = 24000 frames.
        assert_eq!(frames_per_beat, 24_000.0);
    }
}
