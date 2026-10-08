//! Clips da DAW.

/// Tolerância para comparar posições em beats.
///
/// Beats são `f32`, então "encostado" nunca é igualdade exata: um clip
/// cortado em 3.7 e recolado precisa reconhecer o vizinho mesmo com o
/// resíduo do arredondamento. 1/1000 de beat é ~0.5 ms em 120 BPM —
/// abaixo de qualquer coisa audível ou clicável.
pub const BEAT_EPSILON: f32 = 0.001;

/// Cor primária de um sample (recurso de áudio).
///
/// Hash FNV-1a do caminho do arquivo (normalizado em lowercase — o
/// Windows é case-insensitive). É DETERMINÍSTICA: qualquer instância
/// do mesmo arquivo nasce com a mesma cor (1..=20, paleta da UI),
/// estável entre projetos. A única forma de divergir é o usuário
/// alterar manualmente a cor da instância (`Clip::color_override`).
pub fn sample_color(path: &str) -> u8 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_lowercase().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    1 + (hash % 20) as u8
}

/// Cor determinística para um padrão de bateria (MIDI clip vindo do pool).
///
/// Igual ao `sample_color`, mas aplicada ao NOME do pattern: padrões
/// com o mesmo nome nascem com a mesma cor, estável entre projetos.
/// Usado pelo drop do Clip Pool na timeline — sem isso, cada instância
/// do mesmo padrão nasce com cor aleatória (dependia do `id` do clip).
pub fn pattern_color(name: &str) -> u8 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.to_lowercase().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    1 + (hash % 20) as u8
}

/// Duas posições em beats são a mesma?
#[inline]
pub fn beats_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= BEAT_EPSILON
}

/// Tipo de conteúdo de um clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// Clip de áudio.
    Audio,
    /// Clip MIDI.
    Midi,
}

/// Como o clip reage ao tempo do projeto (Cubase: Musical × Linear).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeMode {
    /// MUSICAL (musical mode / timewarp): o conteúdo é esticado para caber
    /// no span do clip, preservando o pitch. Mudar o BPM (ou um ponto de
    /// tempo, ou um marcador de AudioWarp) muda a duração do áudio e ele
    /// continua casando com a grade.
    Musical,
    /// LINEAR (tempo absoluto): o áudio toca na velocidade natural, 1
    /// frame da fonte por frame de saída. O span do clip no projeto decide
    /// apenas QUANTO da fonte é ouvido. É o comportamento histórico do
    /// motor — e o default, para que ligar o musical mode seja sempre uma
    /// decisão explícita (o importer liga).
    #[default]
    Linear,
}

/// Marcador de AudioWarp ("free warp"): prende uma posição da FONTE a uma
/// posição do PROJETO, e os trechos entre marcadores ganham ratios próprios.
///
/// As duas coordenadas são LOCAIS ao clip, de propósito:
/// `source_seconds` conta a partir de onde o clip começa a ler o arquivo
/// (não do início do arquivo) e `project_offset_beats` a partir do início
/// do clip. Assim mover o clip na timeline não deforma o warp, e trim/offset
/// de origem não precisam de conversão de BPM para posicionar marcadores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpMarker {
    /// Offset dentro da região da fonte, em segundos (0 = início do clip).
    pub source_seconds: f32,
    /// Onde esse ponto deve soar, em beats locais ao clip (0 = início).
    pub project_offset_beats: f32,
}

impl WarpMarker {
    pub fn new(source_seconds: f32, project_offset_beats: f32) -> Self {
        Self {
            source_seconds: source_seconds.max(0.0),
            project_offset_beats: project_offset_beats.max(0.0),
        }
    }
}

/// Ordena e saneia marcadores (invariante do documento: sempre crescentes
/// em `source_seconds`, sem duplicatas exatas).
pub fn normalize_warp_markers(mut markers: Vec<WarpMarker>) -> Vec<WarpMarker> {
    markers.retain(|m| {
        m.source_seconds.is_finite() && m.project_offset_beats.is_finite()
    });
    markers.sort_by(|a, b| a.source_seconds.total_cmp(&b.source_seconds));
    markers.dedup_by(|a, b| (a.source_seconds - b.source_seconds).abs() < 1e-4);
    markers
}

/// Um clip posicionado na timeline.
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: u64,
    /// Trilha à qual o clip pertence.
    pub track_id: u64,
    pub name: String,
    pub kind: ClipKind,
    /// Posição inicial em beats na timeline.
    ///
    /// Fracionário: a duração de um arquivo de áudio quase nunca cai
    /// numa batida inteira, e forçar inteiro esticava ou achatava a
    /// waveform para caber.
    pub start_beat: f32,
    /// Duração em beats.
    pub length_beats: f32,
    /// Posição inicial dentro do arquivo de áudio, em beats do projeto.
    /// Clips MIDI ignoram este campo.
    pub source_start_beats: f32,
    /// Índice de cor (paleta na UI). 0 = padrão MIDI/Audio.
    pub color: u8,
    /// `true` = cor definida manualmente (override da instância, pelo
    /// swatch/recolor). `false` = cor herdada do sample (áudio) ou a
    /// padrão por id (MIDI) — novas instâncias do mesmo arquivo nascem
    /// com a MESMA cor vinda do ClipPool.
    pub color_override: bool,
    /// Caminho do arquivo de áudio (apenas clips de áudio).
    pub file_path: Option<String>,
    /// Fade de ENTRADA em beats (0 = sem fade além da rampa
    /// anti-clique de ~1 ms do motor).
    pub fade_in_beats: f32,
    /// Fade de SAÍDA em beats (0 = sem fade além da rampa
    /// anti-clique de ~1 ms do motor).
    pub fade_out_beats: f32,
    /// Ganho do clip em dB (-inf..+12). 0 = unidade (1.0 linear).
    /// Controlado pela barrinha central do clip e pelo campo GAIN do
    /// BarProperties quando o clip está selecionado.
    pub gain_db: f32,
    /// Musical × Linear (timewarp estilo Cubase). Clips MIDI ignoram.
    pub time_mode: TimeMode,
    /// Marcadores de AudioWarp (free warp), sempre ordenados por
    /// `source_seconds` — ver [`normalize_warp_markers`].
    pub warp_markers: Vec<WarpMarker>,
    /// Notas MIDI do clip (apenas clips MIDI). Vazio = clip sem conteúdo.
    pub notes: Vec<daw_midi::Note>,
}

impl Clip {
    /// Cria um novo clip.
    pub fn new(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        kind: ClipKind,
        start_beat: f32,
        length_beats: f32,
    ) -> Self {
        Self {
            id,
            track_id,
            name: name.into(),
            kind,
            start_beat,
            length_beats,
            source_start_beats: 0.0,
            color: ((id % 19) + 1) as u8,
            color_override: false,
            file_path: None,
            fade_in_beats: 0.0,
            fade_out_beats: 0.0,
            gain_db: 0.0,
            time_mode: TimeMode::Linear,
            warp_markers: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Cria um clip MIDI.
    pub fn midi(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        start_beat: f32,
        length_beats: f32,
    ) -> Self {
        Self::new(
            id,
            track_id,
            name,
            ClipKind::Midi,
            start_beat,
            length_beats,
        )
    }

    /// Cria um clip de áudio.
    pub fn audio(
        id: u64,
        track_id: u64,
        name: impl Into<String>,
        start_beat: f32,
        length_beats: f32,
    ) -> Self {
        Self::new(
            id,
            track_id,
            name,
            ClipKind::Audio,
            start_beat,
            length_beats,
        )
    }

    /// Beat onde o clip termina.
    #[inline]
    pub fn end_beat(&self) -> f32 {
        self.start_beat + self.length_beats
    }

    /// O clip cobre o intervalo `[start, end)`?
    ///
    /// Encostar não é sobrepor: um clip que termina exatamente onde o
    /// outro começa é adjacente, e adjacente é legal.
    pub fn overlaps(&self, start: f32, end: f32) -> bool {
        start < self.end_beat() - BEAT_EPSILON && end > self.start_beat + BEAT_EPSILON
    }

    /// True se o clip é MIDI.
    pub fn is_midi(&self) -> bool {
        self.kind == ClipKind::Midi
    }

    /// True se o áudio do clip acompanha o tempo (musical mode).
    ///
    /// MIDI nunca é "esticado" (o conteúdo já é musical): o modo só vale
    /// para áudio.
    pub fn is_musical(&self) -> bool {
        !self.is_midi() && self.time_mode == TimeMode::Musical
    }

    /// Define o caminho do arquivo de áudio (builder pattern).
    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Notas MIDI do clip (somente clips MIDI).
    pub fn notes(&self) -> &[daw_midi::Note] {
        &self.notes
    }

    /// Substitui as notas MIDI do clip.
    pub fn set_notes(&mut self, notes: Vec<daw_midi::Note>) {
        self.notes = notes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip_at(start: f32, len: f32) -> Clip {
        Clip::audio(1, 0, "x", start, len)
    }

    #[test]
    fn end_beat_is_start_plus_length() {
        assert_eq!(clip_at(2.5, 1.25).end_beat(), 3.75);
    }

    #[test]
    fn adjacent_clips_do_not_overlap() {
        let c = clip_at(4.0, 2.0);
        // Termina em 6.0: começar em 6.0 é encostar, não sobrepor.
        assert!(!c.overlaps(6.0, 8.0));
        assert!(!c.overlaps(0.0, 4.0));
    }

    #[test]
    fn partial_and_total_overlap_detected() {
        let c = clip_at(4.0, 2.0);
        assert!(c.overlaps(5.0, 7.0)); // entra pela direita
        assert!(c.overlaps(3.0, 5.0)); // entra pela esquerda
        assert!(c.overlaps(0.0, 16.0)); // engole
        assert!(c.overlaps(4.5, 5.5)); // dentro
    }

    #[test]
    fn beats_eq_tolerates_float_residue() {
        assert!(beats_eq(3.7, 3.7 + BEAT_EPSILON * 0.5));
        assert!(!beats_eq(3.7, 3.8));
    }

    #[test]
    fn sample_color_is_deterministic_and_case_insensitive() {
        // Mesmo caminho → mesma cor (estável entre projetos).
        assert_eq!(sample_color("C:/Samples/HIP_Kick.wav"), sample_color("C:/Samples/HIP_Kick.wav"));
        // Case-insensitive (Windows path).
        assert_eq!(sample_color("C:/samples/hip_kick.wav"), sample_color("C:/SAMPLES/HIP_Kick.WAV"));
        // Dentro da paleta (1..=20, 0 é reservado para "padrão por tipo").
        for p in ["A", "B", "C", "D", "E"] {
            let c = sample_color(&format!("X:/{p}.wav"));
            assert!((1..=20).contains(&c), "cor {c} fora da paleta");
        }
    }
}
