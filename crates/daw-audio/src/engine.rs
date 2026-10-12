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

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use daw_core::{PlayMode, Transport};

use crate::backend::{AudioConfig, AudioError, CpalBackend};
use crate::meter::{CLIP_L, CLIP_R, MeterBank};
use crate::mixer::{
    limit_stereo, mix_stereo_source, soft_clip, stereo_gains, stereo_gains_bipolar, track_is_active,
};
use crate::{AudioBufferKind, buffer::BufferCache};

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

/// Comprimento do crossfade no retorno do loop, em frames (~2 ms a
/// 48 kHz). No wrap, o transporte salta de fim-de-loop para o início
/// na fronteira do buffer — o degrau de amplitude é o clique. A cauda
/// pré-wrap fade-a-fora enquanto os primeiros frames pós-wrap
/// fade-a-dentro (equal-power); o ponto musical do loop não muda.
const LOOP_XFADE_FRAMES: usize = 96;

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
    buffer: AudioBufferKind,
    /// Fade de entrada em BEATS (multiplicamos por frames_per_beat no
    /// render; o snapshot não tem o bpm/rate, então guardamos em beats).
    fade_in_beats: f64,
    /// Fade de saída em BEATS.
    fade_out_beats: f64,
    /// Ganho do clip em linear (10^(dB/20)). 1.0 = 0 dB.
    gain_linear: f32,
    /// TIMEWARP: frames da fonte consumidos por frame de saída.
    ///
    /// `1.0` = tempo absoluto (clip LINEAR, ou buffer já esticado no alvo).
    /// `< 1.0` = a fonte avança mais devagar que o transporte (conteúdo
    /// esticado); `> 1.0` = mais rápido. Vem PRONTO do snapshot
    /// ([`crate::stretch::source_rate`]) para a thread RT só multiplicar —
    /// a conta depende apenas de campos já resolvidos, então recalculá-la
    /// por clip por frame seria desperdício no caminho mais quente.
    read_rate: f64,
}

/// Nota MIDI de um clip da timeline (somente os campos que a thread RT
/// precisa; sem histórico/edição).
#[derive(Clone)]
struct MidiNoteSlot {
    pitch: i32,
    start_beat: f64,
    velocity: i32,
}

/// Clip MIDI da timeline: NOTAS, não áudio. O motor dispara o sample da
/// linha do padrão correspondente ao pitch (sampler MIDI ao vivo).
#[derive(Clone)]
struct MidiClipSlot {
    track_id: u64,
    start_beat: f64,
    notes: Vec<MidiNoteSlot>,
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
    buffer: Option<AudioBufferKind>,
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
#[derive(Clone)]
pub struct ProjectSnapshot {
    tracks: Vec<TrackSlot>,
    clips: Vec<ClipSlot>,
    midi_clips: Vec<MidiClipSlot>,
    beat: BeatSlot,
    /// Ganho linear do bus master, ANTES do limitador.
    master_volume: f32,
    /// Master mutado: zera o bus sem mexer no fader.
    master_muted: bool,
}

/// O snapshot vazio precisa masterizar em unidade, não em silêncio.
///
/// `ProjectSnapshot::default()` é o que a RT usa antes do primeiro
/// `sync_project` — e o programa abre o dispositivo no startup, muito
/// antes do primeiro flush. Um default derivativo zeraria o master e o
/// primeiro play sairia mudo, que é o tipo de bug que só aparece com
/// o projeto já carregado.
impl Default for ProjectSnapshot {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            beat: BeatSlot::default(),
            master_volume: 1.0,
            master_muted: false,
        }
    }
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

                // TIMEWARP: o clip é musical (acompanha o tempo)?
                //
                // O comprimento NATURAL da região vem do próprio buffer (que
                // já está na taxa de saída): é a régua para saber quanto o
                // conteúdo precisa ser esticado para caber no span, e é o
                // que permite ao motor acompanhar o tempo sem depender de
                // nenhum campo derivado do documento.
                let output_rate = cache.output_sample_rate().max(1) as f64;
                let natural_seconds = buffer.frames() as f64 / output_rate;
                let target =
                    crate::stretch::stretch_target_for(c, &project.tempo_map, natural_seconds);

                // Já existe render do alvo ATUAL no cache? Então é ele que
                // toca (keylock, leitura 1:1). Se não, toca o ORIGINAL em
                // varispeed: o áudio acompanha o tempo já no primeiro frame
                // e o worker entrega a versão com pitch preservado logo.
                let (
                    buffer,
                    musical,
                    source_seconds,
                    span_seconds,
                    rendered_ratio,
                    source_start_beats,
                ) = match &target {
                    Some(t) => match cache.get(&t.key()) {
                        Some(stretched) => {
                            // O buffer renderizado começa no início do trecho
                            // selecionado; aplicar o offset original de novo
                            // faria a reprodução saltar para o meio do render.
                            (
                                stretched,
                                true,
                                t.source_seconds,
                                t.span_seconds,
                                t.ratio,
                                0.0,
                            )
                        }
                        None => (
                            buffer,
                            true,
                            t.source_seconds,
                            t.span_seconds,
                            1.0,
                            c.source_start_beats as f64,
                        ),
                    },
                    None => (buffer, false, 0.0, 0.0, 1.0, c.source_start_beats as f64),
                };

                let read_rate = crate::stretch::source_rate(
                    musical,
                    source_seconds,
                    span_seconds,
                    rendered_ratio,
                );

                Some(ClipSlot {
                    track_id: c.track_id,
                    start_beat: c.start_beat as f64,
                    length_beats: c.length_beats as f64,
                    source_start_beats,
                    buffer,
                    fade_in_beats: c.fade_in_beats as f64,
                    fade_out_beats: c.fade_out_beats as f64,
                    gain_linear: 10f32.powf(c.gain_db / 20.0),
                    read_rate,
                })
            })
            .collect();

        let midi_clips = project
            .clips
            .iter()
            .filter(|c| c.kind == daw_core::ClipKind::Midi)
            .map(|c| MidiClipSlot {
                track_id: c.track_id,
                start_beat: c.start_beat as f64,
                notes: c
                    .notes
                    .iter()
                    .map(|n| MidiNoteSlot {
                        pitch: n.pitch,
                        start_beat: n.start as f64,
                        velocity: n.velocity,
                    })
                    .collect(),
            })
            .filter(|c| !c.notes.is_empty())
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
            midi_clips,
            beat,
            master_volume: project.master_mixer.volume,
            master_muted: project.master_mixer.muted,
        }
    }
}

// ============================================================
// VOZES
// ============================================================

/// Uma voz one-shot do sampler.
struct Voice {
    buffer: Option<AudioBufferKind>,
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
    buffer: AudioBufferKind,
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
    /// Ponte de medição: a RT escreve, a UI lê. Ver [`crate::meter`].
    meters: Arc<MeterBank>,
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

    /// Guarda no cache um buffer JÁ RENDERIZADO (timewarp) sob a chave do
    /// seu alvo — [`crate::stretch::StretchTarget::key`].
    ///
    /// NÃO publica o snapshot: chame [`Self::sync_project`] em seguida para
    /// a thread RT passar a enxergar o buffer. Os dois passos são separados
    /// de propósito (o worker pode inserir vários renders e publicar uma
    /// vez), e a publicação em si é só uma troca de `Arc` — a RT nunca vê
    /// estado parcial.
    ///
    /// Retorna `false` se o lock do cache estiver indisponível.
    pub fn insert_buffer(&self, key: &str, buffer: crate::mapped_buffer::AudioBufferKind) -> bool {
        let Ok(mut cache) = self.cache.lock() else {
            return false;
        };
        cache.insert(key, buffer);
        true
    }

    /// Já existe um buffer publicado sob esta chave?
    ///
    /// O worker de timewarp usa para não renderizar duas vezes o mesmo alvo.
    pub fn has_buffer(&self, key: &str) -> bool {
        let Ok(cache) = self.cache.lock() else {
            return false;
        };
        cache.get(key).is_some()
    }

    /// Esquece um buffer do cache (chave sintética de render, ao evictar o
    /// render mais antigo).
    pub fn evict_buffer(&self, key: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.evict(key);
        }
    }

    /// Buffer decodificado de um arquivo (carrega se ainda não estiver).
    ///
    /// O worker de timewarp renderiza a partir do MESMO buffer que a RT lê:
    /// sem reabrir o arquivo e sem decodificar de novo.
    pub fn buffer_for(&self, path: &str) -> Option<crate::mapped_buffer::AudioBufferKind> {
        let Ok(mut cache) = self.cache.lock() else {
            return None;
        };
        cache.load(path)
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

    /// Banco de medição para os VU meters.
    ///
    /// Só a engine escreve; a UI lê. O `&MeterBank` sai daqui justamente
    /// para não haver `Arc` a mais para manter vivo: quem já tem um
    /// `EngineHandle` tem o banco.
    pub fn meters(&self) -> &MeterBank {
        &self.meters
    }

    /// Declara quantos canais os medidores devem esperar (trilhas +
    /// master). Ao crescer, os canais novos começam em silêncio.
    pub fn set_meter_channels(&self, channels: usize) {
        self.meters.set_channel_count(channels);
    }

    /// Apaga o latch de clip de um canal (clique no medidor).
    pub fn clear_meter_clip(&self, channel: usize) {
        self.meters.clear_clip(channel);
    }

    /// Apaga todos os latches de clip.
    pub fn clear_meter_clips(&self) {
        self.meters.clear_all_clips();
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
    /// Modo visto no buffer anterior. A virada Timeline↔BeatOnly tem
    /// que re-ancorar o agendador de passos como se fosse um play.
    was_beat_only: bool,
    metronome_beat: i64,
    metronome_remaining: u32,
    metronome_phase: f32,
    /// Cauda dos últimos frames renderizados (estéreo), o material
    /// que "continuaria" a tocar se não houvesse wrap. Usada como
    /// rampa descendente do crossfade do retorno do loop.
    loop_tail: VecDeque<(f32, f32)>,
    /// Frames restantes do crossfade de loop ativo.
    xfade_remaining: usize,
    /// Comprimento do crossfade ativo (pode ser menor que
    /// `LOOP_XFADE_FRAMES` se o loop for curto ou a cauda incompleta).
    xfade_len: usize,
    /// Pico do buffer por canal (L, R), em [`crate::meter::FLOATS_PER_CHANNEL`]
    /// entradas por canal. Zera a cada callback — o que a UI mostra é o
    /// pico DESTE buffer, e segurar o máximo histórico aqui faria o
    /// medidor nunca cair.
    meter_peaks: Vec<f32>,
    /// Bitmask de clip por canal, acumulado no buffer. Vive na RT (que
    /// decide o que estourou) e é publicado a cada callback.
    meter_clips: Vec<u32>,
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
            was_beat_only: false,
            metronome_beat: -1,
            metronome_remaining: 0,
            metronome_phase: 0.0,
            loop_tail: VecDeque::with_capacity(LOOP_XFADE_FRAMES),
            xfade_remaining: 0,
            xfade_len: 0,
            // Capacidade fixa, igual ao `MeterBank`. Alocar aqui, no
            // construtor, é o que mantém `prepare_meters` livre de
            // alocação — a versão anterior crescia o `Vec` no callback,
            // e um `resize` na RT durante uma criação de faixa é uma
            // queda de áudio justamente no gesto que a causou.
            meter_peaks: vec![
                0.0;
                crate::meter::MAX_METER_CHANNELS * crate::meter::FLOATS_PER_CHANNEL
            ],
            meter_clips: vec![0u32; crate::meter::MAX_METER_CHANNELS],
        }
    }

    /// Prepara os acumuladores de medição para `channels` canais.
    ///
    /// Só zera — nunca redimensiona. Os `Vec` já vêm com
    /// [`crate::meter::MAX_METER_CHANNELS`] de capacidade, o mesmo teto do
    /// banco publicado, então nem criar nem apagar trilha aloca na RT.
    fn prepare_meters(&mut self, channels: usize) {
        let need = channels * crate::meter::FLOATS_PER_CHANNEL;
        if need > self.meter_peaks.len() {
            // Mais canais do que o banco aceita medir: o excedente é
            // ignorado pelo `publish`, e medir não pode custar um
            // estouro de índice no callback.
            return;
        }
        for v in self.meter_peaks[..need].iter_mut() {
            *v = 0.0;
        }
        for v in self.meter_clips[..channels].iter_mut() {
            *v = 0;
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

    fn trigger(&mut self, buffer: AudioBufferKind, gain_l: f32, gain_r: f32, delay: u32) {
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

    /// Aplica o crossfade de loop a um frame estéreo e mantém a cauda.
    ///
    /// Fora do crossfade só empurra o frame na cauda (sem tocar no
    /// sinal). Durante o crossfade, o frame da cauda pré-wrap é lido
    /// da ponta e misturado com o frame atual em equal-power: a rampa
    /// descendente vai de ~1 a 0, a ascendente de 0 a ~1 — a potência
    /// percebida fica constante, sem degrau na fronteira do wrap.
    #[inline]
    fn xfade_loop_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let out = if self.xfade_remaining > 0 {
            let idx = self.xfade_len - self.xfade_remaining;
            self.xfade_remaining -= 1;
            match self.loop_tail.front() {
                Some(&(hl, hr)) => {
                    let t = (idx as f32 + 0.5) / self.xfade_len as f32;
                    let g_out = (t * std::f32::consts::FRAC_PI_2).cos();
                    let g_in = (t * std::f32::consts::FRAC_PI_2).sin();
                    (hl * g_out + l * g_in, hr * g_out + r * g_in)
                }
                None => (l, r),
            }
        } else {
            (l, r)
        };
        self.loop_tail.push_back(out);
        if self.loop_tail.len() > LOOP_XFADE_FRAMES {
            self.loop_tail.pop_front();
        }
        out
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
        // Medidores: alocado antes da stream abrir, com capacidade fixa,
        // e publicado para a UI pelo handle. A RT só escreve por índice.
        let meters = Arc::new(MeterBank::new());

        let snapshot_cb = snapshot.clone();
        let preview_cb = preview.clone();
        let transport_cb = transport.clone();
        let meters_cb = meters.clone();

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
                &meters_cb,
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
                meters,
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
        meters: &MeterBank,
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
        let mut mode = PlayMode::Timeline;
        let mut bpm = 120.0f64;
        let mut ppq = 960.0f64;
        if let Ok(t) = transport.try_lock() {
            playing = t.playing;
            mode = t.mode;
            bpm = t.bpm as f64;
            ppq = t.ppq as f64;

            let frames_per_tick = sample_rate as f64 * 60.0 / (bpm * ppq);

            // A UI mexeu na posição (seek/stop-rewind)? Re-ancora.
            if t.position_ticks != rt.ticks_written {
                rt.pos_frames = t.position_ticks as f64 * frames_per_tick;
                rt.next_step = i64::MIN; // recalculado abaixo
                rt.kill_voices();
                // Seek não é wrap de loop: nada de crossfade, e a
                // cauda velha (pré-seek) não pode vazar pra ele.
                rt.xfade_remaining = 0;
                rt.loop_tail.clear();
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

        // Trocar o modo no meio da reprodução (a UI sempre passa por um
        // stop, mas o Transport é público) precisa do mesmo reset: o
        // agendador de passos não pode retomar de um cursor velho e
        // despejar uma rajada de passos atrasados.
        let beat_only = playing && mode == PlayMode::BeatOnly;
        if beat_only != rt.was_beat_only {
            if beat_only {
                rt.next_step = i64::MIN;
            } else {
                rt.kill_voices();
            }
            rt.was_beat_only = beat_only;
        }

        let start_frames = rt.pos_frames;

        let (metronome_enabled, metronome_sound, metronome_volume) = transport
            .try_lock()
            .map(|t| {
                (
                    t.metronome_enabled,
                    t.metronome_sound.min(4),
                    t.metronome_volume.clamp(0.0, 1.0),
                )
            })
            .unwrap_or((false, 0, 0.7));
        // Modo "só beat": o metrônomo não toca — só o pattern.
        if playing && mode == PlayMode::Timeline && metronome_enabled {
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
        //
        // O pattern ativo em loop só toca no modo isolado do header. O
        // play global usa os clips colocados na timeline, inclusive
        // clips MIDI criados a partir de patterns.
        if beat_only && !snap.beat.rows.is_empty() && snap.beat.steps > 0 {
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

        // Canais medidos: uma entrada por trilha + o master no fim.
        // É a MESMA ordem de `mixer_state()` no app, para o índice do
        // medidor e o índice do canal do mixer coincidirem sem mapa.
        let meter_channels = snap.tracks.len() + 1;
        let master_channel = snap.tracks.len();
        rt.prepare_meters(meter_channels);

        // ---- agendamento das notas MIDI (timeline) -------------
        // Cada nota de um clip MIDI mapeia pra uma linha do pattern
        // (pitch = DRUM_PITCH_BASE + índice) e dispara o sample daquela
        // linha — mesmo banco que o step sequencer usa, com o mesmo
        // respeito a active/solo. O ganho é o do TRACK da timeline
        // vezes o da linha vezes a velocidade da nota.
        if playing && mode == PlayMode::Timeline {
            let end_frames = start_frames + frames as f64;
            for clip in snap.midi_clips.iter() {
                let Some(track) = snap.tracks.iter().find(|t| t.id == clip.track_id) else {
                    continue;
                };
                if !track_is_active(track.muted, track.soloed, any_soloed) {
                    continue;
                }
                let (tg_l, tg_r) = stereo_gains(track.volume, track.pan);
                for note in clip.notes.iter() {
                    let onset = (clip.start_beat + note.start_beat) * frames_per_beat;
                    if onset < start_frames || onset >= end_frames {
                        continue;
                    }
                    let row_idx = (note.pitch - daw_midi::DRUM_PITCH_BASE) as usize;
                    let Some(row) = snap.beat.rows.get(row_idx) else {
                        continue;
                    };
                    if !track_is_active(!row.active, row.solo, snap.beat.any_solo) {
                        continue;
                    }
                    let Some(buffer) = row.buffer.clone() else {
                        continue;
                    };
                    let vel = (note.velocity as f32 / 127.0).clamp(0.0, 1.0);
                    let rg = row.volume * vel;
                    let (rg_l, rg_r) = if row.pan == 0.0 {
                        (rg, rg)
                    } else {
                        let (rl, rr) = stereo_gains_bipolar(1.0, row.pan);
                        (rg * rl, rg * rr)
                    };
                    let delay = (onset - start_frames).max(0.0) as u32;
                    rt.trigger(buffer, tg_l * rg_l, tg_r * rg_r, delay.min(frames as u32));
                }
            }
        }

        for frame in 0..frames {
            let mut left = 0.0f32;
            let mut right = 0.0f32;

            // Clips da timeline (só com o transporte andando e fora do
            // modo "só beat").
            if playing && mode == PlayMode::Timeline {
                let pos = start_frames + frame as f64;

                for (track_idx, track) in snap.tracks.iter().enumerate() {
                    if !track_is_active(track.muted, track.soloed, any_soloed) {
                        continue;
                    }
                    let (gain_l, gain_r) = stereo_gains(track.volume, track.pan);

                    // Sub-soma DA TRACK, separada do bus. O medidor de
                    // cada trilha precisa do que ELA entregou, não do
                    // total com as vizinhas: somar direto no bus dava a
                    // mesma barra para todas as trilhas.
                    let mut track_l = 0.0f32;
                    let mut track_r = 0.0f32;

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
                        //
                        // `read_rate` é o timewarp: em modo musical a fonte
                        // avança em ritmo diferente do transporte (ou o
                        // buffer já é a versão esticada, e o rate é 1.0).
                        // Em modo linear é 1.0, exatamente como antes.
                        let source_offset = (clip.source_start_beats * frames_per_beat
                            + offset * clip.read_rate)
                            .max(0.0);
                        let (sl, sr) = clip.buffer.read(source_offset);

                        // Rampa nas bordas: cortar o clip no meio de
                        // um ciclo é um degrau, e degrau é clique.
                        // Cada lado tem seu próprio fade (mínimo da
                        // rampa anti-clique de ~1 ms mesmo com fade=0).
                        // Gain do clip já vem linear (dB->linear).
                        let fade_in = (clip.fade_in_beats * frames_per_beat).max(CLIP_EDGE_FADE);
                        let fade_out = (clip.fade_out_beats * frames_per_beat).max(CLIP_EDGE_FADE);
                        let edge = (offset / fade_in)
                            .min((clip_len - offset) / fade_out)
                            .min(1.0) as f32;

                        let clip_gain = clip.gain_linear;
                        mix_stereo_source(
                            sl * edge * clip_gain,
                            sr * edge * clip_gain,
                            gain_l,
                            gain_r,
                            &mut track_l,
                            &mut track_r,
                        );
                    }

                    // Pico do frame para o canal desta trilha. Medido
                    // ANTES do master: baixar o master não pode fazer o
                    // medidor de uma trilha mentir.
                    //
                    // A guarda de range não é decorativa: os acumuladores
                    // têm capacidade fixa, e um projeto com mais faixas do
                    // que o banco mede não pode estourar índice aqui — o
                    // áudio continua passando, só esse canal não é medido.
                    if track_idx < crate::meter::MAX_METER_CHANNELS {
                        let base = track_idx * 2;
                        let mag_l = track_l.abs();
                        let mag_r = track_r.abs();
                        if mag_l > rt.meter_peaks[base] {
                            rt.meter_peaks[base] = mag_l;
                        }
                        if mag_r > rt.meter_peaks[base + 1] {
                            rt.meter_peaks[base + 1] = mag_r;
                        }
                        if mag_l >= 1.0 {
                            rt.meter_clips[track_idx] |= CLIP_L;
                        }
                        if mag_r >= 1.0 {
                            rt.meter_clips[track_idx] |= CLIP_R;
                        }
                    }

                    left += track_l;
                    right += track_r;
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
                let click =
                    (t * freq * std::f32::consts::TAU).sin() * envelope * metronome_volume * 0.18;
                left += click;
                right += click;
                rt.metronome_phase += 1.0;
                rt.metronome_remaining -= 1;
            }

            // ---- MASTER ---------------------------------------
            // O fader do master é aplicado AQUI, antes do limitador:
            // é o último controle de ganho da cadeia, então puxar o
            // master para baixo segura o material de verdade em vez de
            // pedir ao limitador que faça o trabalho do fader. Sem
            // isto, `Project.master_mixer` era lido só pela UI e o
            // fader não mudava o áudio.
            let master_gain = if snap.master_muted {
                0.0
            } else {
                snap.master_volume
            };
            let bus_l = left * master_gain;
            let bus_r = right * master_gain;

            // Medidor do master: o sinal que o limitador vai receber.
            // O clip é testado AQUI, e não na saída — `limit_stereo`
            // segura tudo em ±1, então medir depois nunca acusaria
            // clip. O que acende a luz é "o material passou do fundo de
            // escala", que é a informação que o usuário precisa.
            let master_base = master_channel * 2;
            let mag_l = bus_l.abs();
            let mag_r = bus_r.abs();
            if mag_l > rt.meter_peaks[master_base] {
                rt.meter_peaks[master_base] = mag_l;
            }
            if mag_r > rt.meter_peaks[master_base + 1] {
                rt.meter_peaks[master_base + 1] = mag_r;
            }
            if mag_l >= 1.0 {
                rt.meter_clips[master_channel] |= CLIP_L;
            }
            if mag_r >= 1.0 {
                rt.meter_clips[master_channel] |= CLIP_R;
            }

            let (raw_l, raw_r) = limit_stereo(bus_l, bus_r);
            let (l, r) = rt.xfade_loop_frame(raw_l, raw_r);
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

        // ---- publica os medidores -------------------------------
        // Uma vez por buffer, depois que o buffer inteiro foi medido:
        // publicar por frame daria ao VU 48 kHz de atualizações para
        // suavizar (e a UI lê a ~30 Hz de qualquer jeito). `Relaxed`
        // basta: é um número de medição, não um sinal de ordem — a UI
        // pode ler o pico do buffer anterior ao lado do atual sem que
        // isso signifique nada para o áudio.
        for channel in 0..meter_channels {
            let base = channel * 2;
            meters.publish(
                channel,
                rt.meter_peaks[base],
                rt.meter_peaks[base + 1],
                rt.meter_clips[channel],
            );
        }

        // ---- devolve o relógio ao transporte -------------------
        // Os ticks são DERIVADOS da posição em samples: o áudio é o
        // relógio autoritativo, o domínio musical é a projeção.

        if playing {
            rt.pos_frames = start_frames + frames as f64;
            let ticks_per_frame = bpm * ppq / (60.0 * sample_rate as f64);
            let ticks = (rt.pos_frames * ticks_per_frame).max(0.0) as u64;
            if let Ok(mut t) = transport.try_lock() {
                let written_ticks = if t.loop_enabled
                    && t.loop_end_ticks > t.loop_start_ticks
                    && ticks >= t.loop_end_ticks
                {
                    let span = t.loop_end_ticks - t.loop_start_ticks;
                    t.loop_start_ticks + (ticks - t.loop_start_ticks) % span
                } else {
                    ticks
                };
                t.position_ticks = written_ticks;
                rt.ticks_written = written_ticks;
                if written_ticks != ticks {
                    rt.pos_frames = written_ticks as f64 / ticks_per_frame;
                    // Wrap de loop: engatilha o crossfade para o fim da
                    // cauda pré-wrap encontrar o início da pós-wrap sem
                    // degrau. O comprimento é limitado a metade do loop
                    // (um loop mais curto que o crossfade não pode ter
                    // uma janela maior que o próprio span) e à cauda
                    // disponível. Um crossfade em andamento não é
                    // reiniciado: loops menores que 1 buffer voltam a
                    // cruzar antes de ~2 ms e dispensam duas rampas.
                    if rt.xfade_remaining == 0 {
                        let span_frames =
                            (t.loop_end_ticks - t.loop_start_ticks) as f64 / ticks_per_frame;
                        let len = LOOP_XFADE_FRAMES
                            .min((span_frames * 0.5).floor().max(1.0) as usize)
                            .min(rt.loop_tail.len())
                            .max(1);
                        rt.xfade_len = len;
                        rt.xfade_remaining = len;
                    }
                }
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
    use crate::AudioBuffer;
    use crate::meter::MeterReading;

    #[test]
    fn steps_per_beat_is_sixteenth() {
        // 4 passos por batida = semicolcheia; 16 passos = 1 compasso.
        assert_eq!(STEPS_PER_BEAT, 4.0);
    }

    #[test]
    fn meter_accumulators_are_allocated_once_and_never_grow() {
        // A RT não pode alocar. A versão anterior crescia os `Vec` com
        // `resize` no callback, o que punia a criação de uma faixa com
        // uma queda de áudio exatamente no gesto que a causou.
        let mut rt = RtState::new();
        let peaks = crate::meter::MAX_METER_CHANNELS * crate::meter::FLOATS_PER_CHANNEL;
        assert_eq!(rt.meter_peaks.len(), peaks);
        assert_eq!(rt.meter_clips.len(), crate::meter::MAX_METER_CHANNELS);

        rt.prepare_meters(2);
        rt.prepare_meters(crate::meter::MAX_METER_CHANNELS);
        // Mais canais do que a capacidade: ignora em vez de estourar.
        rt.prepare_meters(crate::meter::MAX_METER_CHANNELS + 10);

        assert_eq!(rt.meter_peaks.len(), peaks);
        assert_eq!(rt.meter_clips.len(), crate::meter::MAX_METER_CHANNELS);
    }

    #[test]
    fn prepare_meters_clears_only_the_channels_in_use() {
        let mut rt = RtState::new();
        rt.meter_peaks[0] = 0.9;
        rt.meter_peaks[4] = 0.9;
        rt.meter_clips[1] = CLIP_L;

        rt.prepare_meters(1);

        assert_eq!(rt.meter_peaks[0], 0.0);
        // Canal de fora do projeto em uso não é tocado — e, mais
        // importante, o clip latch dele também não é limpo: a UI ainda
        // pode estar mostrando aquela luz.
        assert_eq!(rt.meter_peaks[4], 0.9);
        assert_eq!(rt.meter_clips[1], CLIP_L);
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
        let buffer = AudioBufferKind::Memory(buffer);
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

    /// Grava um WAV s16le mono com um TOM de 1 kHz no início (0.25s) e
    /// silêncio depois. Ajuda a provar que o começo do arquivo ESCAPOU
    /// do buffer (não é um pulso que só "parece de baixo nível").
    fn write_marker_wav(path: &std::path::Path) {
        let sample_rate = 48_000u32;
        let marker = (sample_rate as usize / 4) as usize; // 0.25 s
        let total = sample_rate as usize * 3; // 3 s
        let mut samples = Vec::with_capacity(total);
        for i in 0..total {
            let v = if i < marker {
                let s = (0.9
                    * ((i as f32) * std::f32::consts::TAU * 1000.0 / sample_rate as f32).sin())
                .clamp(-1.0, 1.0);
                ((s * 32767.0).round() as i16).to_le_bytes()
            } else {
                0i16.to_le_bytes()
            };
            samples.extend_from_slice(&v);
        }
        let mut wav = Vec::with_capacity(44 + samples.len() * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate as u32 * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(path, wav).expect("grava wav de teste");
    }

    /// TIMEWARP ponta a ponta, ate onde da para verificar sem placa de som:
    /// o clip musical tem de sair do snapshot lendo a fonte em ritmo
    /// diferente do transporte (varispeed), e quando o render do worker
    /// entra no cache o snapshot passa a ler o buffer esticado 1:1.
    #[test]
    fn timewarp_musical_varispeed_e_depois_buffer_pronto() {
        let dir = std::env::temp_dir();
        let path = dir.join("ruana_timewarp_tone.wav");
        // 3 s mono a 48 kHz.
        write_sine_wav(&path, 440.0, 0.5);
        let path_str = path.to_string_lossy().into_owned();

        let rate = 48_000u32;
        let natural_seconds = 3.0;

        let mut project = daw_core::Project::startup();
        // 8 beats a 120 BPM = 4.0 s de span para 3.0 s de fonte: o conteudo
        // precisa ficar 4/3 mais longo.
        project.add_clip_with_source(
            0,
            daw_core::ClipKind::Audio,
            "tone",
            0.0,
            8.0,
            0.0,
            Some(path_str.clone()),
        );

        let mut cache = BufferCache::new(rate);

        // 1) Sem render: varispeed. O rate e o INVERSO do ratio pedido, e o
        // audio acompanha o tempo no primeiro frame (pitch acompanha).
        let snapshot = ProjectSnapshot::build(&project, &mut cache);
        assert_eq!(snapshot.clips.len(), 1);

        let needed = 4.0 / natural_seconds;
        let expected_varispeed = 1.0 / needed;

        assert!(
            (snapshot.clips[0].read_rate - expected_varispeed).abs() < 1e-3,
            "varispeed: read_rate {} != {expected_varispeed}",
            snapshot.clips[0].read_rate
        );

        // 2) Modo LINEAR volta a ser 1:1 (o comportamento historico).
        project.clips[0].time_mode = daw_core::TimeMode::Linear;
        let linear = ProjectSnapshot::build(&project, &mut cache);

        assert!(
            (linear.clips[0].read_rate - 1.0).abs() < 1e-9,
            "linear: read_rate {}",
            linear.clips[0].read_rate
        );

        project.clips[0].time_mode = daw_core::TimeMode::Musical;

        // 3) O worker renderiza o alvo e publica o buffer esticado.
        let clip = project.clips[0].clone();
        let target = crate::stretch::stretch_target_for(&clip, &project.tempo_map, natural_seconds)
            .expect("alvo de stretch");

        assert!(target.needs_render(), "ratio {}", target.ratio);

        let source = cache.load(&path_str).expect("buffer da fonte");
        let stretched = crate::stretch::render_stretch(&source, &target).expect("render");

        assert_eq!(
            stretched.frames(),
            target.frames_out(rate),
            "o buffer pronto tem de ter exatamente o span do clip"
        );

        cache.insert(
            target.key(),
            crate::mapped_buffer::AudioBufferKind::Memory(std::sync::Arc::new(stretched)),
        );

        // 4) Com o render pronto o snapshot le 1:1 - sem reamostragem.
        let ready = ProjectSnapshot::build(&project, &mut cache);

        assert!(
            (ready.clips[0].read_rate - 1.0).abs() < 1e-3,
            "buffer pronto: read_rate {}",
            ready.clips[0].read_rate
        );

        std::fs::remove_file(&path).ok();
    }

    /// Clip COLADO NO ZERO da timeline: o começo do arquivo precisa sair
    /// nos primeiros buffers. Foi o sintoma relatado — clip em `start_beat=0`
    /// "não reproduz o começo".
    #[test]
    fn clip_em_zero_reproduz_o_inicio() {
        let dir = std::env::temp_dir();
        let path = dir.join("ruana_clip_zero_marker.wav");
        write_marker_wav(&path);
        let path_str = path.to_string_lossy().into_owned();

        let rate = 48_000u32;

        let mut project = daw_core::Project::startup();
        // `add_clip_with_source`: clip de AUDIO no beat 0, comprimento 5
        // beats (cobre os 3s do arquivo), origem do arquivo no frame 0.
        project.add_clip_with_source(
            0,
            daw_core::ClipKind::Audio,
            "zero",
            0.0,
            5.0,
            0.0,
            Some(path_str.clone()),
        );
        // Master em unidade: este teste é sobre QUANDO o clip começa, e
        // o fader do master agora é real (o default 0.8 tiraria 2 dB e
        // o limiar de 0.5 do assert mediria o master, não o clip).
        project.master_mixer.volume = 1.0;

        let mut cache = BufferCache::new(rate);
        let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::build(
            &project, &mut cache,
        ))));
        let transport = Arc::new(Mutex::new(Transport::default()));
        transport.lock().unwrap().playing = true; // play do zero

        let mut rt = RtState::new();
        let preview = Arc::new(Mutex::new(Vec::<PreviewCmd>::new()));
        let meters = MeterBank::new();
        meters.set_channel_count(project.tracks.len() + 1);

        let frames = 960usize;
        let mut early_max = 0.0f32;
        for _ in 0..4 {
            let mut out = vec![0.0f32; frames * 2];
            AudioEngine::process(
                &mut out, 2, &mut rt, &transport, &snapshot, &preview, &meters, rate,
            );
            for i in 0..frames {
                early_max = early_max.max(out[i * 2].abs()).max(out[i * 2 + 1].abs());
            }
        }
        let track_reading = meters.read(0);
        let master_reading = meters.read(project.tracks.len());
        std::fs::remove_file(&path).ok();

        assert!(
            early_max > 0.5,
            "clip no beat 0 deve tocar o TOM do arquivo já nos primeiros buffers (max={})",
            early_max,
        );

        // O VU tem que ver o MESMO sinal que a saída, na trilha E no
        // master. Antes o meter era um PRNG na UI, então nada aqui
        // falhava mesmo com o áudio certo.
        assert!(
            track_reading.peak_left > 0.5 && track_reading.peak_right > 0.5,
            "medidor da trilha tem que acompanhar o áudio (l={} r={})",
            track_reading.peak_left,
            track_reading.peak_right,
        );
        assert!(
            master_reading.peak_left > 0.5,
            "medidor do master tem que acompanhar o áudio (l={})",
            master_reading.peak_left,
        );
        assert!(
            !track_reading.clipped_left && !master_reading.clipped_left,
            "um tom em 0.8 não pode acender o clip",
        );
    }

    #[test]
    fn pattern_toca_conforme_o_modo_e_a_presenca_na_timeline() {
        use daw_midi::{BeatPatterns, DrumPattern};

        let path = std::env::temp_dir().join("ruana_pool_pattern_mode.wav");
        write_marker_wav(&path);
        let path_str = path.to_string_lossy().into_owned();
        let rate = 48_000u32;
        let frames = 960usize;

        let render_peak = |mode, put_midi_clip_on_timeline| {
            let mut project = daw_core::Project::startup();
            project.master_mixer.volume = 1.0;
            let mut pattern = DrumPattern::new("POOL", 1, 16);
            pattern.grid[0][0] = true;
            pattern.track_params[0].sample_path = Some(path_str.clone());
            project.beat_patterns = BeatPatterns {
                patterns: vec![pattern],
                current: 0,
            };
            if put_midi_clip_on_timeline {
                let id = project
                    .add_clip(0, daw_core::ClipKind::Midi, "POOL", 0.0, 4.0)
                    .expect("cria clip MIDI na timeline");
                project.set_clip_notes(
                    id,
                    vec![daw_midi::Note {
                        id: 1,
                        pitch: daw_midi::DRUM_PITCH_BASE,
                        start: 0.0,
                        length: 0.25,
                        velocity: 127,
                    }],
                );
            }

            let mut cache = BufferCache::new(rate);
            let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::build(
                &project, &mut cache,
            ))));
            let transport = Arc::new(Mutex::new(Transport::default()));
            {
                let mut t = transport.lock().unwrap();
                t.playing = true;
                t.mode = mode;
            }
            let mut rt = RtState::new();
            let preview = Arc::new(Mutex::new(Vec::<PreviewCmd>::new()));
            let meters = MeterBank::new();
            meters.set_channel_count(project.tracks.len() + 1);

            let mut peak = 0.0f32;
            for _ in 0..4 {
                let mut out = vec![0.0f32; frames * 2];
                AudioEngine::process(
                    &mut out, 2, &mut rt, &transport, &snapshot, &preview, &meters, rate,
                );
                peak = out
                    .chunks_exact(2)
                    .map(|frame| frame[0].abs().max(frame[1].abs()))
                    .fold(peak, f32::max);
            }
            peak
        };

        let timeline_without_clip = render_peak(PlayMode::Timeline, false);
        let timeline_with_clip = render_peak(PlayMode::Timeline, true);
        let isolated_beat = render_peak(PlayMode::BeatOnly, false);
        std::fs::remove_file(&path).ok();

        assert!(
            timeline_without_clip < 1e-6,
            "o pattern selecionado no pool não pode tocar em loop sem um clip na timeline (peak={timeline_without_clip})"
        );
        assert!(
            timeline_with_clip > 0.5,
            "o play global deve reproduzir o clip MIDI de pattern que está na timeline (peak={timeline_with_clip})"
        );
        assert!(
            isolated_beat > 0.5,
            "o botão Play do header deve tocar o pattern isolado (peak={isolated_beat})"
        );
    }

    /// O fader do master tem que MUDAR O ÁUDIO, e o medidor de cada
    /// trilha tem que continuar mostrando o sinal da trilha — não o do
    /// barramento. Antes o master não existia na engine (`rg master` em
    /// `crates/daw-audio` não achava nada) e o VU era um PRNG.
    #[test]
    fn master_fader_attenua_e_a_trilha_nao_muda() {
        let dir = std::env::temp_dir();
        let path = dir.join("ruana_master_fader_marker.wav");
        write_marker_wav(&path);
        let path_str = path.to_string_lossy().into_owned();

        let rate = 48_000u32;
        let frames = 960usize;

        let render = |master: f32| -> (f32, MeterReading, MeterReading) {
            let mut project = daw_core::Project::startup();
            project.add_clip_with_source(
                0,
                daw_core::ClipKind::Audio,
                "zero",
                0.0,
                5.0,
                0.0,
                Some(path_str.clone()),
            );
            project.master_mixer.volume = master;

            let mut cache = BufferCache::new(rate);
            let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::build(
                &project, &mut cache,
            ))));
            let transport = Arc::new(Mutex::new(Transport::default()));
            transport.lock().unwrap().playing = true;
            let preview = Arc::new(Mutex::new(Vec::<PreviewCmd>::new()));
            let meters = MeterBank::new();
            let master_channel = project.tracks.len();
            meters.set_channel_count(master_channel + 1);

            let mut rt = RtState::new();
            let mut out = vec![0.0f32; frames * 2];
            AudioEngine::process(
                &mut out, 2, &mut rt, &transport, &snapshot, &preview, &meters, rate,
            );
            let peak = (0..frames)
                .map(|i| out[i * 2].abs().max(out[i * 2 + 1].abs()))
                .fold(0.0f32, f32::max);
            (peak, meters.read(0), meters.read(master_channel))
        };

        let (unity_peak, unity_track, unity_master) = render(1.0);
        let (half_peak, half_track, half_master) = render(0.5);

        assert!(
            unity_peak > 0.5,
            "setup: o clip tem que soar (peak={})",
            unity_peak
        );
        assert!(
            (half_peak / unity_peak - 0.5).abs() < 0.05,
            "master em 0.5 tem que cortar o áudio pela metade (unity={} half={})",
            unity_peak,
            half_peak,
        );
        // A trilha é medida ANTES do master: o fader do master muda o
        // que se ouve, não o que a trilha entregou ao bus.
        assert!(
            (half_track.peak_left - unity_track.peak_left).abs() < 1e-4,
            "medidor da trilha não pode depender do master (unity={} half={})",
            unity_track.peak_left,
            half_track.peak_left,
        );
        assert!(
            half_master.peak_left < unity_master.peak_left * 0.75,
            "medidor do master tem que cair junto com o áudio (unity={} half={})",
            unity_master.peak_left,
            half_master.peak_left,
        );

        std::fs::remove_file(&path).ok();
    }

    /// Grava um WAV s16le mono contínuo (seno puro) para cravar a
    /// não-descontinuidade do wrap de loop.
    fn write_sine_wav(path: &std::path::Path, freq: f32, amp: f32) {
        let sample_rate = 48_000u32;
        let total = sample_rate as usize * 3; // 3 s
        let mut samples = Vec::with_capacity(total);
        for i in 0..total {
            let v = (amp * (freq * std::f32::consts::TAU * i as f32 / sample_rate as f32).sin())
                .clamp(-1.0, 1.0);
            samples.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes());
        }
        let mut wav = Vec::with_capacity(44 + samples.len() * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate as u32 * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(path, wav).expect("grava wav de teste");
    }

    /// O retorno do loop não pode degrau: no wrap, a cauda pré-loop é
    /// cruzada com os primeiros frames pós-loop (crossfade), então a
    /// maior derivada perto do loop point fica bem abaixo do degrau que
    /// o sinal teria sem o fade.
    #[test]
    fn loop_wrap_nao_descontinuo() {
        let dir = std::env::temp_dir();
        let path = dir.join("ruana_loop_tone.wav");
        write_sine_wav(&path, 1_000.0, 0.5);
        let path_str = path.to_string_lossy().into_owned();

        let rate = 48_000u32;
        let bpm = 120u32;
        // 120 BPM / 960 PPQ → 1 tick = 25 frames a 48 kHz.
        let frames_per_tick = rate as f64 * 60.0 / (bpm as f64 * 960.0);
        let buf_frames = 960usize;
        // Loop de 2000 ticks (50_000 frames); o play começa perto do fim
        // para o primeiro wrap chegar rápido depois da cauda encher.
        let loop_start_ticks = 1_000u64; // frame 25_000
        let loop_end_ticks = 3_000u64; // frame 75_000
        assert!(
            ((loop_end_ticks - loop_start_ticks) as f64 * frames_per_tick)
                > buf_frames as f64 * 4.0,
            "loop longo o bastante para o crossfade inteiro caber num buffer"
        );

        let mut project = daw_core::Project::startup();
        project.add_clip_with_source(
            0,
            daw_core::ClipKind::Audio,
            "tone",
            0.0,
            5.0,
            0.0,
            Some(path_str.clone()),
        );
        // Este teste mede a DESCONTINUIDADE DO WRAP do loop, e para isso
        // pressupõe leitura 1:1 da fonte (frame de saída = frame da fonte).
        // Clip de áudio com arquivo nasce MUSICAL (timewarp), o que mudaria
        // a velocidade de leitura e deslocaria a fronteira medida — então o
        // tempo absoluto é fixado aqui de propósito.
        if let Some(clip) = project.clips.first_mut() {
            clip.time_mode = daw_core::TimeMode::Linear;
        }

        let mut cache = BufferCache::new(rate);
        let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::build(
            &project, &mut cache,
        ))));
        let transport = Arc::new(Mutex::new(Transport::default()));
        {
            let mut t = transport.lock().unwrap();
            t.playing = true;
            t.loop_enabled = true;
            t.loop_start_ticks = loop_start_ticks;
            t.loop_end_ticks = loop_end_ticks;
            t.position_ticks = 2_900; // dentro do loop, perto do fim
        }

        // Degrau que o wrap TERIA sem crossfade: o maior salto entre o
        // fim do material pré-loop (em qualquer posição de fronteira de
        // buffer depois do loop_end) e o início do loop, lido do buffer.
        let buffer = cache.load(&path_str).expect("carrega o tom");
        let start_f = loop_start_ticks as f64 * frames_per_tick;
        let end_f = loop_end_ticks as f64 * frames_per_tick;
        let raw_jump = (0..buf_frames)
            .map(|k| (buffer.read(end_f + k as f64).0 - buffer.read(start_f).0).abs())
            .fold(0.0f32, f32::max);
        assert!(
            raw_jump > 0.5,
            "setup do teste: o loop tem que ter degrau real (raw_jump={})",
            raw_jump
        );

        let mut rt = RtState::new();
        let preview = Arc::new(Mutex::new(Vec::<PreviewCmd>::new()));
        let meters = MeterBank::new();
        meters.set_channel_count(project.tracks.len() + 1);

        let mut out_all: Vec<f32> = Vec::new();
        let mut boundaries: Vec<usize> = Vec::new();
        // ~100 kframes = 2 voltas completas do loop.
        let nbuffers = (loop_end_ticks as usize * 2) * 25 / buf_frames + 8;
        for _ in 0..nbuffers {
            let pos_before = rt.pos_frames;
            let mut out = vec![0.0f32; buf_frames * 2];
            AudioEngine::process(
                &mut out, 2, &mut rt, &transport, &snapshot, &preview, &meters, rate,
            );
            let wrapped = rt.pos_frames + 0.5 < pos_before + buf_frames as f64;
            for f in 0..buf_frames {
                out_all.push(out[f * 2]);
            }
            if wrapped {
                // O wrap acontece no FIM deste buffer; o crossfade cobre
                // o começo do PRÓXIMO — o próximo índice de `out_all`.
                boundaries.push(out_all.len());
            }
        }
        std::fs::remove_file(&path).ok();

        assert!(
            boundaries.len() >= 3,
            "esperado várias voltas de loop (wraps={})",
            boundaries.len()
        );

        let max_dy_window = boundaries
            .iter()
            .filter(|&&b| b + LOOP_XFADE_FRAMES + 1 < out_all.len())
            .map(|&b| {
                (b..b + LOOP_XFADE_FRAMES)
                    .map(|j| (out_all[j + 1] - out_all[j]).abs())
                    .fold(0.0f32, f32::max)
            })
            .fold(0.0f32, f32::max);
        let max_dy_global = out_all
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);

        // ~1 kHz / 48 kHz com amp 0.5: passo natural do seno ≈ 0.065.
        // O crossfade empurra o degrau do wrap pra baixo de um quarto
        // dele; sem o fade o salto seria ~raw_jump (>>0.5).
        assert!(
            max_dy_window < raw_jump * 0.4,
            "crossfade do loop deixou degrau: max_dy janela={} raw_jump={}",
            max_dy_window,
            raw_jump,
        );
        assert!(
            max_dy_global < 0.25,
            "wrap introduziu descontinuidade no sinal: max_dy={}",
            max_dy_global,
        );
    }

    /// CONTROLE: o mesmo arquivo numa posição posterior não toca antes da hora.
    #[test]
    fn clip_posicionado_longe_fica_silencioso_no_inicio() {
        let dir = std::env::temp_dir();
        let path = dir.join("ruana_clip_longe_marker.wav");
        write_marker_wav(&path);
        let path_str = path.to_string_lossy().into_owned();

        let rate = 48_000u32;

        let mut project = daw_core::Project::startup();
        // Clip a 2 beats (1s) do zero; 3s de arquivo = ~6 beats.
        project.add_clip_with_source(
            0,
            daw_core::ClipKind::Audio,
            "zero",
            2.0,
            6.0,
            0.0,
            Some(path_str.clone()),
        );

        let mut cache = BufferCache::new(rate);
        let snapshot = Arc::new(Mutex::new(Arc::new(ProjectSnapshot::build(
            &project, &mut cache,
        ))));
        let transport = Arc::new(Mutex::new(Transport::default()));
        transport.lock().unwrap().playing = true;

        let mut rt = RtState::new();
        let preview = Arc::new(Mutex::new(Vec::<PreviewCmd>::new()));
        let meters = MeterBank::new();
        meters.set_channel_count(project.tracks.len() + 1);

        let frames = 960usize;
        let mut max = 0.0f32;
        for _ in 0..4 {
            let mut out = vec![0.0f32; frames * 2];
            AudioEngine::process(
                &mut out, 2, &mut rt, &transport, &snapshot, &preview, &meters, rate,
            );
            for i in 0..frames {
                max = max.max(out[i * 2].abs()).max(out[i * 2 + 1].abs());
            }
        }
        let reading = meters.read(0);
        std::fs::remove_file(&path).ok();

        assert!(
            max < 0.5,
            "clip a 1s do zero não pode soar nos primeiros 0.08s (max={})",
            max,
        );
        // Silêncio na saída é silêncio no VU. Um medidor que mostra
        // nível com o transporte em silêncio no clip é pior que
        // nenhum: faz o usuário caçar um phantom no gain.
        assert!(
            reading.peak_left == 0.0 && reading.peak_right == 0.0,
            "medidor tem que estar em silêncio antes do clip começar (l={} r={})",
            reading.peak_left,
            reading.peak_right,
        );
    }
}
