// Cache e publicação de waveform "Bin Visual" para a camada Slint.
//
// Em vez de imagem RGBA pré-renderizada, cada clip guarda um LOD
// multinível de BARRAS min→max por coluna:
//
//   - Cada nível é um conjunto de colunas; a coluna `i` cobre o chunk
//     `[i*chunk, (i+1)*chunk)` de amostras mono e guarda MIN e MAX.
//   - `chunk = frames / largura` (divisão inteira, no mínimo 1); a
//     ÚLTIMA coluna absorve a sobra ("último grupo com as sobras").
//   - LOD multinível: `lods[0]` é o mais fino (maior nº de colunas),
//     os seguintes são progressivamente mais grossos (zoom-out).
//   - A UI escolhe o nível cuja contagem de colunas cobre a largura de
//     exibição pedida e desenha uma barra vertical por coluna.
//
// O cache é um LRU simples (clip_id → CachedWave) — o decode acontece
// uma única vez na worker thread e o LOD inteiro fica em RAM.
use crate::MainWindow;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::ClipData;
use daw_core::clip::Clip;

/// Barras min/max de um nível de zoom: arrays paralelos, `mins[i]` e
/// `maxs[i]` são o menor/maior sample do i-ésimo chunk. `mins <= maxs`.
#[derive(Clone, Debug, Default)]
pub struct LodBars {
    /// Menor valor de cada coluna (negativo/zero).
    pub mins: Vec<f32>,
    /// Maior valor de cada coluna (positivo/zero).
    pub maxs: Vec<f32>,
}

impl LodBars {
    pub fn cols(&self) -> usize {
        self.mins.len()
    }
}

/// Waveform em cache por clip: LOD multinível pronto para publicar.
#[derive(Clone)]
pub struct CachedWave {
    /// Níveis do mais fino (lods[0]) ao mais grosso. Cada nível tem
    /// sua própria contagem de colunas (largura onde ele "cabe").
    pub lods: Vec<LodBars>,
    /// Total de quadros (samples mono) que este cache cobre. Usado
    /// para manter bordas alinhadas em split/merge/trim proporcionais.
    pub frames: usize,
}

/// Constante de build: largura (colunas) do nível mais fino.
const FINEST_COLS: usize = 16384;

/// Largura "exibida" padrão quando ainda não há pedido de resolução.
const DEFAULT_COLS: usize = 512;

/// Constrói o LOD multinível a partir das amostras mono.
///
/// Algoritmo por nível: `chunk = frames / width` (no mínimo 1),
/// `cols = ceil(frames / chunk)` e a última coluna leva a sobra. Os
/// níveis vão afinando/engrossando em potências de 2; quando o nº de
/// colunas repetir (arquivo curto), o nível é descartado.
pub fn build_lods(samples: &[f32]) -> Vec<LodBars> {
    let total = samples.len();
    if total == 0 {
        return Vec::new();
    }
    let mut lods = Vec::new();
    let mut prev_cols: Option<usize> = None;
    let mut width = FINEST_COLS;
    loop {
        let chunk = (total / width).max(1);
        let cols = total.div_ceil(chunk);
        if prev_cols == Some(cols) {
            if width <= 2 {
                break;
            }
            width /= 2;
            continue;
        }
        prev_cols = Some(cols);
        let mut mins = Vec::with_capacity(cols);
        let mut maxs = Vec::with_capacity(cols);
        for c in 0..cols {
            let start = c * chunk;
            let end = (start + chunk).min(total);
            let mut mn = f32::INFINITY;
            let mut mx = f32::NEG_INFINITY;
            for &v in &samples[start..end] {
                if v < mn {
                    mn = v;
                }
                if v > mx {
                    mx = v;
                }
            }
            mins.push(if mn.is_finite() { mn } else { 0.0 });
            maxs.push(if mx.is_finite() { mx } else { 0.0 });
        }
        lods.push(LodBars { mins, maxs });
        if width <= 2 || cols <= 2 {
            break;
        }
        width /= 2;
    }
    lods
}

/// Índice do nível mais grosso que ainda cobre `width_px` colunas.
/// Se nenhum cobre (zoom fundo → quer mais colunas que o mais fino),
/// devolve o mais fino (lods[0]).
pub fn pick_level(lods: &[LodBars], width_px: usize) -> &LodBars {
    match lods {
        [] => EMPTY_LOD.get_or_init(Default::default),
        [one] => one,
        many => {
            for l in many.iter().rev() {
                if l.cols() >= width_px {
                    return l;
                }
            }
            &many[0]
        }
    }
}

static EMPTY_LOD: std::sync::OnceLock<LodBars> = std::sync::OnceLock::new();

/// Índice do nível de "prévia" (closer à largura padrão) usado quando
/// ainda não chegou o pedido de resolução da UI.
fn pick_default(lods: &[LodBars]) -> usize {
    let mut best = 0;
    let mut best_d = usize::MAX;
    for (i, l) in lods.iter().enumerate() {
        let d = l.cols().abs_diff(DEFAULT_COLS);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Aplica ganho linear e fades (fi/fo em px da largura pedida) numa
/// cópia das barras, prontas para publicar. Ganho/fade são "baked" nos
/// valores para a waveform mostrar a envoltória real (não só sombra).
pub fn bake_bars(
    level: &LodBars,
    width_px: u32,
    gain_linear: f32,
    fi_px: f32,
    fo_px: f32,
) -> (Vec<f32>, Vec<f32>) {
    let n = level.cols();
    let gain = gain_linear.clamp(0.0, 4.0);
    let w = width_px.max(1) as f32;
    let fi_cols = ((fi_px / w) * n as f32).round() as usize;
    let fo_cols = ((fo_px / w) * n as f32).round() as usize;
    let fo_lo = n.saturating_sub(fo_cols);
    let mut mins = Vec::with_capacity(n);
    let mut maxs = Vec::with_capacity(n);
    for i in 0..n {
        let mut factor = 1.0;
        if fi_cols > 0 && i < fi_cols {
            factor *= (i as f32 + 1.0) / fi_cols as f32;
        }
        if fo_cols > 0 && i >= fo_lo {
            factor *= (n - i) as f32 / fo_cols as f32;
        }
        let g = factor * gain;
        mins.push((level.mins[i] * g).clamp(-1.0, 1.0));
        maxs.push((level.maxs[i] * g).clamp(-1.0, 1.0));
    }
    (mins, maxs)
}

/// Cache de waveform com política LRU: clip_id → dados completos.
///
/// `get` marca recência (frente da fila = mais recente) via `RefCell`
/// interior, mantendo a assinatura `&self` — os call sites clonam o
/// `CachedWave` de qualquer forma. `insert` despeja o menos recente
/// quando a capacidade é excedida.
pub struct WaveformCache {
    map: HashMap<u64, CachedWave>,
    /// Frente = MRU, cauda = candidato a eviction.
    order: RefCell<VecDeque<u64>>,
    cap: usize,
}

impl WaveformCache {
    pub fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: RefCell::new(VecDeque::new()),
            cap: cap.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, id: &u64) -> Option<&CachedWave> {
        let hit = self.map.get(id)?;
        let mut order = self.order.borrow_mut();
        if let Some(pos) = order.iter().position(|k| *k == *id) {
            order.remove(pos);
        }
        order.push_front(*id);
        Some(hit)
    }

    pub fn insert(&mut self, id: u64, wave: CachedWave) -> Option<CachedWave> {
        if !self.map.contains_key(&id) && self.map.len() >= self.cap {
            if let Some(old) = self.order.borrow_mut().pop_back() {
                self.map.remove(&old);
            }
        }
        let old = self.map.insert(id, wave);
        let mut order = self.order.borrow_mut();
        if let Some(pos) = order.iter().position(|k| *k == id) {
            order.remove(pos);
        }
        order.push_front(id);
        old
    }

    pub fn remove(&mut self, id: &u64) -> Option<CachedWave> {
        let removed = self.map.remove(id);
        self.order.borrow_mut().retain(|k| *k != *id);
        removed
    }

    pub fn contains_key(&self, id: &u64) -> bool {
        self.map.contains_key(id)
    }
}

/// Publica os mins/maxes de um `CachedWave` (nível de prévia) num
/// `ClipData` existente, a partir do cache.
pub fn to_clip_data_from_id(id: u64, base: &ClipData, waveforms: &WaveformCache) -> ClipData {
    let (mins, maxs) = match waveforms.get(&id) {
        Some(c) => {
            let i = pick_default(&c.lods);
            (c.lods[i].mins.clone(), c.lods[i].maxs.clone())
        }
        None => (Vec::new(), Vec::new()),
    };
    ClipData {
        id: base.id,
        name: base.name.clone(),
        track_index: base.track_index,
        beat: base.beat,
        len: base.len,
        is_midi: base.is_midi,
        color: base.color,
        waveform_mins: Rc::new(slint::VecModel::from(mins)).into(),
        waveform_maxes: Rc::new(slint::VecModel::from(maxs)).into(),
        multi_selected: base.multi_selected,
        fade_in: base.fade_in,
        fade_out: base.fade_out,
        gain_db: base.gain_db,
    }
}

/// Atualiza apenas as barras de um `ClipData` existente a partir do cache.
pub fn reproject_clip(clips_model: &Rc<slint::VecModel<ClipData>>, waveforms: &WaveformCache, clip_id: u64) {
    let i = match crate::models::clip_row(clips_model, clip_id as i32) {
        Some(i) => i,
        None => return,
    };
    let Some(mut data) = clips_model.row_data(i) else {
        return;
    };
    let updated = to_clip_data_from_id(clip_id, &data, waveforms);
    data.waveform_mins = updated.waveform_mins;
    data.waveform_maxes = updated.waveform_maxes;
    clips_model.set_row_data(i, data);
}

// Re-publica as barras na resolução exata de exibição.
//
// COALESCING (anti-travamento): o pedido chega a cada frame de
// scroll/zoom (`changed wave-w1` no clip). Como agora a operação é
// escolher um nível de LOD (custo trivial, sem worker), o processamento
// cabe num timer de ~40 ms: se novos pedidos chegam enquanto o timer
// roda, apenas sobrescrevemos o pendente (coalesce).
//
// Ganho e fades são "baked" nas barras min→max neste passo — o waveform
// mostra a envoltória real (gain/fade visíveis), exatamente como o
// raster de antes fazia.

use crate::TimelineMetrics;
use crate::models::clip_row;
use slint::ComponentHandle;
use slint::Model;

/// Pedido mais recente de publicação de waveform (de-duplicado por clip).
#[derive(Clone, Copy)]
struct PendingWave {
    id: u64,
    w: u32,
    gain: f32,
    fi: f32,
    fo: f32,
}

pub fn wire_resolution_requests(
    ui: &MainWindow,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
) {
    // Pedidos pendentes por clip + flag de coalescing (timer ativo).
    let pending: Rc<RefCell<HashMap<u64, PendingWave>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let scheduled: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

    ui.global::<TimelineMetrics>()
        .on_waveform_resolution_requested(move |id, width_px, _wx0, _wx1, _hh, gain_linear, fi, fo| {
            let req = PendingWave {
                id: id.max(0) as u64,
                w: (width_px.round() as u32).clamp(16, FINEST_COLS as u32),
                gain: gain_linear,
                fi,
                fo,
            };
            // De-duplica por clip: cada clip mantém o pedido mais novo.
            pending.borrow_mut().insert(req.id, req);

            if *scheduled.borrow() {
                return;
            }
            *scheduled.borrow_mut() = true;

            let pending_shot = pending.clone();
            let scheduled_shot = scheduled.clone();
            let waves_shot = waveforms.clone();
            let clips_shot = clips_model.clone();

            slint::Timer::single_shot(
                std::time::Duration::from_millis(40),
                move || {
                    *scheduled_shot.borrow_mut() = false;
                    // Consome TODOS os clips pedidos (não só o último):
                    // ao parar o scroll, cada clip recebe sua waveform.
                    let batch = std::mem::take(&mut *pending_shot.borrow_mut());

                    for (req_id, req) in batch {
                        // Escolhe o nível cujas colunas cobrem a largura
                        // exata de exibição e publica as barras (gain/fade
                        // baked). Sem worker: só cópia + aritmética.
                        let Some(cw) = waves_shot.borrow().get(&req_id).cloned() else {
                            continue;
                        };
                        let Some(idx) = clip_row(&clips_shot, req_id as i32) else {
                            continue;
                        };
                        let level = pick_level(&cw.lods, req.w as usize);
                        let (mins, maxs) = bake_bars(level, req.w, req.gain, req.fi, req.fo);
                        if let Some(mut data) = clips_shot.row_data(idx) {
                            data.waveform_mins = Rc::new(slint::VecModel::from(mins)).into();
                            data.waveform_maxes = Rc::new(slint::VecModel::from(maxs)).into();
                            clips_shot.set_row_data(idx, data);
                        }
                    }
                },
            );
        });
}

/// Corta NÍVEIS pela fração de colunas [f0, f0+f1] (usado por trim).
fn slice_levels(lods: &[LodBars], f0: f64, f1: f64) -> Vec<LodBars> {
    lods.iter()
        .map(|l| {
            let n = l.cols();
            if n == 0 {
                return LodBars {
                    mins: Vec::new(),
                    maxs: Vec::new(),
                };
            }
            let lo = ((n as f64 * f0).floor() as usize).min(n.saturating_sub(1));
            let hi = ((n as f64 * (f0 + f1)).ceil() as usize)
                .max(lo + 1)
                .min(n);
            LodBars {
                mins: l.mins[lo..hi].to_vec(),
                maxs: l.maxs[lo..hi].to_vec(),
            }
        })
        .collect()
}

/// Divide a waveform cacheada de um clip em duas metades proporcionais
/// a `f` (fração de tempo que fica no clip da ESQUERDA após o corte).
///
/// Cada metade ganha sua própria parte dos níveis (as colunas repetem o
/// alinhamento de bordas da fonte). Não há re-decode: LOD já em RAM.
pub fn split_cached_wave(cw: &CachedWave, f: f64) -> (CachedWave, CachedWave) {
    let f = f.clamp(0.0, 1.0);
    let frames_l = ((cw.frames as f64 * f).round() as usize)
        .clamp(1, cw.frames.saturating_sub(1));
    let frames_r = cw.frames - frames_l;
    let mut left = Vec::with_capacity(cw.lods.len());
    let mut right = Vec::with_capacity(cw.lods.len());
    for l in &cw.lods {
        let n = l.cols();
        if n == 0 {
            left.push(l.clone());
            right.push(l.clone());
            continue;
        }
        let k = ((n as f64 * f + 0.5) as usize).clamp(1, n - 1);
        left.push(LodBars {
            mins: l.mins[..k].to_vec(),
            maxs: l.maxs[..k].to_vec(),
        });
        right.push(LodBars {
            mins: l.mins[k..].to_vec(),
            maxs: l.maxs[k..].to_vec(),
        });
    }
    (
        CachedWave {
            lods: left,
            frames: frames_l,
        },
        CachedWave {
            lods: right,
            frames: frames_r,
        },
    )
}

/// Reconstitue a waveform da esquerda unindo as duas metades de um
/// corte (glue): concatena os min/max de cada nível.
pub fn merge_cached_waves(a: &CachedWave, b: &CachedWave) -> CachedWave {
    let levels = a.lods.len().max(b.lods.len());
    let empty = LodBars {
        mins: Vec::new(),
        maxs: Vec::new(),
    };
    let mut lods = Vec::with_capacity(levels);
    for i in 0..levels {
        let la = a.lods.get(i).unwrap_or(&empty);
        let lb = b.lods.get(i).unwrap_or(&empty);
        let mut mins = la.mins.clone();
        mins.extend(lb.mins.iter().copied());
        let mut maxs = la.maxs.clone();
        maxs.extend(lb.maxs.iter().copied());
        lods.push(LodBars { mins, maxs });
    }
    CachedWave {
        lods,
        frames: a.frames + b.frames,
    }
}

/// Corta a waveform cacheada ao trecho visível após um RESIZE de borda
/// — sem re-escalar o conteúdo. As colunas/alinhamento de borda são
/// preservados pela fração [f0, f0+f1].
pub fn trim_cached_wave(cw: &CachedWave, f0: f64, f1: f64) -> CachedWave {
    let f0 = f0.clamp(0.0, 1.0);
    let f1 = f1.clamp(0.0, 1.0 - f0);
    let frames = ((cw.frames as f64 * f1).round() as usize).max(1);
    CachedWave {
        lods: slice_levels(&cw.lods, f0, f1),
        frames,
    }
}

/// Mapeia um clip do core para o DTO da camada Slint.
pub fn to_clip_data(clip: &Clip, waveforms: &WaveformCache) -> ClipData {
    let (mins, maxs) = match waveforms.get(&clip.id) {
        Some(c) => {
            let i = pick_default(&c.lods);
            (c.lods[i].mins.clone(), c.lods[i].maxs.clone())
        }
        None => (Vec::new(), Vec::new()),
    };
    ClipData {
        id: clip.id as i32,
        name: clip.name.clone().into(),
        track_index: clip.track_id as i32,
        beat: clip.start_beat,
        len: clip.length_beats,
        is_midi: clip.is_midi(),
        color: clip.color as i32,
        gain_db: clip.gain_db,
        waveform_mins: Rc::new(slint::VecModel::from(mins)).into(),
        waveform_maxes: Rc::new(slint::VecModel::from(maxs)).into(),
        multi_selected: false,
        fade_in: clip.fade_in_beats,
        fade_out: clip.fade_out_beats,
    }
}