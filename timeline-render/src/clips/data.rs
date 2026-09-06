//! Tipos de dados e geometria dos clips: `ClipDemo`, `ClipMetrics` e
//! `clip_rect` (conversões cursor ↔ conteúdo e retângulos de render).

/// Um clip posicionado numa faixa (track) da timeline.
#[derive(Clone, Debug)]
pub struct ClipDemo {
    pub track_index: usize,
    pub beat: f32,
    pub len: f32,
    pub is_midi: bool,
    pub color: usize,
    pub name: Option<String>,
    pub seam_beat: Option<f32>,
    pub pre_glue_len: Option<f32>,
    pub pre_glue_color: Option<usize>,

    /// Amostras da waveform deste clip (min/max interleaved, 2 por coluna).
    pub wave: Vec<f32>,

    /// Caminho do arquivo de áudio original, se houver.
    pub file_path: Option<String>,

    /// Posição de início dentro do arquivo original, em beats do projeto.
    pub source_start_beats: f32,

    /// Duração do arquivo original em beats (antes de qualquer corte/trim).
    pub source_length_beats: f32,

    /// Número de canais de áudio: 1 = mono, 2 = stereo.
    pub channels: u16,
}

/// Métricas de conversão cursor ↔ conteúdo, espelhadas do App.
#[derive(Clone)]
pub struct ClipMetrics {
    pub px_per_beat: f32,

    /// Segmentos (offset, count) no buffer `wave_bars`, FIXOS por id do clip
    /// (calculados uma vez com os lens originais). Stretch/move nunca mudam
    /// o buffer, então NUNCA recomputar a partir dos lens vivos — senão o
    /// stretch de um clip corrompe a waveform dos clips seguintes.
    pub wave_segments: Vec<(u32, u32)>,

    pub row_height: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub ruler_px: f32,
    pub theme: usize,
}

impl ClipMetrics {
    /// Converte o cursor da janela para o (beat, track) do conteúdo.
    pub(super) fn cursor_to_beat_track(&self, cursor: (f32, f32)) -> (f32, usize) {
        let content_x = cursor.0 + self.scroll_x;
        let content_y = cursor.1 - self.ruler_px + self.scroll_y;

        let beat = if self.px_per_beat > 0.0 {
            content_x / self.px_per_beat
        } else {
            0.0
        };

        let track = if self.row_height > 0.0 {
            (content_y / self.row_height).floor().max(0.0) as usize
        } else {
            0
        };

        (beat, track)
    }
}

/// Retângulo de um clip em coordenadas de janela (já descontando scroll).
pub fn clip_rect(
    clip: &ClipDemo,
    m: &ClipMetrics,
) -> (f32, f32, f32, f32) {
    let x = clip.beat * m.px_per_beat - m.scroll_x;

    // SEM largura mínima artificial.
    // A borda direita deve espelhar exatamente `len * px_per_beat`.
    let w = clip.len * m.px_per_beat;

    let y =
        m.ruler_px
        + clip.track_index as f32 * m.row_height
        - m.scroll_y;

    (x, y, w, m.row_height)
}
