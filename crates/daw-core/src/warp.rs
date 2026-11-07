//! Matematica do timewarp: QUANTO esticar cada trecho de um clip.
//!
//! Fica no core (e nao no motor) porque a mesma resposta serve a tres
//! consumidores diferentes:
//!
//! - o motor de audio, para tocar (rate) e para pedir o buffer esticado;
//! - o worker de render, que estica o arquivo por trecho;
//! - a UI, que precisa saber se o clip esta "esticado" para desenhar.
//!
//! Duas coordenadas, sempre LOCAIS ao clip:
//!   - `source_seconds`: offset dentro da regiao da fonte (0 = onde o clip
//!     comeca a ler o arquivo);
//!   - beats do projeto: `clip.start_beat + offset` no mapa de tempo.
//!
//! Regra do modo MUSICAL: o conteudo da fonte (comprimento natural
//! `region_len_seconds`) tem de caber no span do clip (`length_beats`),
//! medido em segundos pelo mapa de tempo. Os marcadores de AudioWarp
//! dividem esse mapeamento em trechos, cada um com o seu proprio ratio.
//!
//! No modo LINEAR (default) nada e esticado: um unico trecho, ratio 1.0.

use crate::clip::Clip;
use crate::timeline::TempoMap;

/// Espacamento minimo entre marcadores para o trecho existir (segundos).
const MIN_SEGMENT_SECONDS: f64 = 1e-4;

/// Um trecho contiguo da fonte e o ratio com que ele deve ser esticado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpSegment {
    /// Offset do trecho dentro da regiao da fonte, em segundos.
    pub source_offset_seconds: f64,
    /// Duracao do trecho na fonte, em segundos.
    pub source_seconds: f64,
    /// Offset do trecho dentro do clip, em beats locais.
    pub project_offset_beats: f64,
    /// Duracao do trecho no projeto, em beats locais.
    pub project_beats: f64,
    /// Ratio de stretch: `> 1.0` = saida mais longa (mais lenta).
    pub ratio: f64,
}

/// O plano de stretch de um clip inteiro.
#[derive(Debug, Clone, PartialEq)]
pub struct WarpPlan {
    pub segments: Vec<WarpSegment>,
    /// Ratio do clip INTEIRO (span do projeto / fonte). E o valor usado
    /// pelo caminho varispeed enquanto o buffer esticado nao esta pronto.
    pub total_ratio: f64,
}

impl WarpPlan {
    /// Plano de um clip que nao estica nada (Linear, MIDI ou fonte vazia).
    fn identity(clip: &Clip, region_len_seconds: f64) -> Self {
        let source_seconds = if region_len_seconds.is_finite() {
            region_len_seconds.max(0.0)
        } else {
            0.0
        };

        Self {
            segments: vec![WarpSegment {
                source_offset_seconds: 0.0,
                source_seconds,
                project_offset_beats: 0.0,
                project_beats: clip.length_beats.max(0.0) as f64,
                ratio: 1.0,
            }],
            total_ratio: 1.0,
        }
    }

    /// O clip estica alguma coisa?
    pub fn stretches(&self) -> bool {
        (self.total_ratio - 1.0).abs() > 1e-6
    }

    /// Ratio do trecho que contem `source_seconds` (offset na fonte).
    ///
    /// Fora da faixa, devolve o ratio do primeiro/ultimo trecho.
    pub fn ratio_at_source_seconds(&self, source_seconds: f64) -> f64 {
        for seg in &self.segments {
            let end = seg.source_offset_seconds + seg.source_seconds;

            if source_seconds < end {
                return seg.ratio;
            }
        }

        self.segments.last().map(|s| s.ratio).unwrap_or(1.0)
    }
}

/// Calcula o plano de stretch de um clip.
///
/// `region_len_seconds` e o comprimento NATURAL do conteudo que o clip le
/// (duracao do trecho do arquivo, em segundos). Ele nao vive no documento:
/// quem tem o arquivo decodificado (motor, worker ou import) o informa.
pub fn plan_for_clip(
    clip: &Clip,
    tempo_map: &TempoMap,
    region_len_seconds: f64,
) -> WarpPlan {
    if !clip.is_musical()
        || !region_len_seconds.is_finite()
        || region_len_seconds <= MIN_SEGMENT_SECONDS
        || clip.length_beats <= 0.0
    {
        return WarpPlan::identity(clip, region_len_seconds);
    }

    let span_beats = clip.length_beats as f64;
    let start_beat = clip.start_beat as f64;

    // Segundos que o clip ocupa no projeto, pelo mapa de tempo.
    let seconds_at = |offset_beats: f64| -> f64 {
        tempo_map.beats_to_seconds((start_beat + offset_beats) as f32) as f64
    };

    // Ancoras monotonas: comeco, marcadores, fim.
    let mut anchors: Vec<(f64, f64)> = Vec::with_capacity(clip.warp_markers.len() + 2);

    anchors.push((0.0, 0.0));

    for marker in &clip.warp_markers {
        let source = marker.source_seconds as f64;
        let project = (marker.project_offset_beats as f64).clamp(0.0, span_beats);

        // Descarta marcadores fora da regiao ou que cruzariam com o
        // anterior: o mapa tem de ser estritamente crescente nos dois
        // eixos, senao o trecho teria duracao negativa.
        if source <= MIN_SEGMENT_SECONDS || source >= region_len_seconds - MIN_SEGMENT_SECONDS {
            continue;
        }

        let Some(&(last_source, last_project)) = anchors.last() else {
            continue;
        };

        if source <= last_source + MIN_SEGMENT_SECONDS
            || project <= last_project + MIN_SEGMENT_SECONDS
        {
            continue;
        }

        anchors.push((source, project));
    }

    let Some(&(last_source, last_project)) = anchors.last() else {
        return WarpPlan::identity(clip, region_len_seconds);
    };

    if region_len_seconds > last_source + MIN_SEGMENT_SECONDS
        && span_beats > last_project + MIN_SEGMENT_SECONDS
    {
        anchors.push((region_len_seconds, span_beats));
    }

    let mut segments = Vec::with_capacity(anchors.len().saturating_sub(1));

    for pair in anchors.windows(2) {
        let (source_a, project_a) = pair[0];
        let (source_b, project_b) = pair[1];

        let source_seconds = source_b - source_a;
        let project_beats = project_b - project_a;

        if source_seconds <= MIN_SEGMENT_SECONDS {
            continue;
        }

        let project_seconds = seconds_at(project_b) - seconds_at(project_a);

        let ratio = if project_seconds > 0.0 {
            project_seconds / source_seconds
        } else {
            1.0
        };

        segments.push(WarpSegment {
            source_offset_seconds: source_a,
            source_seconds,
            project_offset_beats: project_a,
            project_beats,
            ratio,
        });
    }

    if segments.is_empty() {
        return WarpPlan::identity(clip, region_len_seconds);
    }

    let project_seconds = seconds_at(span_beats) - seconds_at(0.0);
    let total_ratio = if project_seconds > 0.0 {
        project_seconds / region_len_seconds
    } else {
        1.0
    };

    WarpPlan {
        segments,
        total_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{TimeMode, WarpMarker};

    const EPS: f64 = 1e-6;

    fn audio_clip(len_beats: f32, mode: TimeMode) -> Clip {
        let mut clip = Clip::audio(1, 0, "teste", 0.0, len_beats);
        clip.time_mode = mode;
        clip
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn linear_nao_estica_nada() {
        let clip = audio_clip(4.0, TimeMode::Linear);
        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 5.0);

        assert_eq!(plan.segments.len(), 1);
        assert!(close(plan.segments[0].ratio, 1.0));
        assert!(close(plan.total_ratio, 1.0));
        assert!(!plan.stretches());
    }

    #[test]
    fn midi_nunca_estica() {
        let mut clip = Clip::midi(1, 0, "padrao", 0.0, 4.0);
        clip.time_mode = TimeMode::Musical;

        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 2.0);

        assert!(close(plan.total_ratio, 1.0));
    }

    #[test]
    fn musical_sem_marcadores_cabe_no_span() {
        // 120 BPM: 2 beats = 1.0 s. Fonte de 0.5 s -> 2x mais longo.
        let clip = audio_clip(2.0, TimeMode::Musical);
        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 0.5);

        assert_eq!(plan.segments.len(), 1);
        assert!(
            close(plan.segments[0].ratio, 2.0),
            "ratio {0}",
            plan.segments[0].ratio
        );
        assert!(close(plan.total_ratio, 2.0));
        assert!(plan.stretches());
    }

    #[test]
    fn bpm_maior_encurta_o_conteudo() {
        // Mesmo clip musical (2 beats); a 240 BPM 2 beats = 0.5 s, entao a
        // fonte de 1.0 s tem de ser COMPRIMIDA para 0.5x.
        let clip = audio_clip(2.0, TimeMode::Musical);
        let map = TempoMap::from_bpm(240.0);
        let plan = plan_for_clip(&clip, &map, 1.0);

        assert!(close(plan.total_ratio, 0.5), "ratio {}", plan.total_ratio);
    }

    #[test]
    fn marcador_divide_em_dois_trechos_com_ratios_proprios() {
        // Regiao de 1.0 s, clip de 4 beats (= 2.0 s a 120 BPM).
        // Marcador: 0.5 s da fonte deve soar no beat local 1.0.
        //   trecho 1: 0.5 s -> 0.5 s  => 1.0x
        //   trecho 2: 0.5 s -> 1.5 s  => 3.0x
        let mut clip = audio_clip(4.0, TimeMode::Musical);
        clip.warp_markers = vec![WarpMarker::new(0.5, 1.0)];

        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 1.0);

        assert_eq!(plan.segments.len(), 2, "{plan:?}");
        assert!(close(plan.segments[0].ratio, 1.0));
        assert!(close(plan.segments[1].ratio, 3.0));
        assert!(close(plan.total_ratio, 2.0));

        // Lookup por posicao na fonte.
        assert!(close(plan.ratio_at_source_seconds(0.25), 1.0));
        assert!(close(plan.ratio_at_source_seconds(0.75), 3.0));
    }

    #[test]
    fn mapa_de_tempo_variavel_muda_o_ratio_do_trecho() {
        // Clip musical de 4 beats comecando no beat 0, fonte de 1.0 s.
        // Tempo: 120 BPM ate o beat 2 e 60 BPM depois.
        //   0..2 beats a 120 = 1.0 s ; 2..4 beats a 60 = 2.0 s.
        // Marcador em (0.5 s, beat local 2.0):
        //   trecho 1: 0.5 s -> 1.0 s => 2.0x
        //   trecho 2: 0.5 s -> 2.0 s => 4.0x
        let mut map = TempoMap::from_bpm(120.0);
        map.set_change(2.0, 60.0);

        let mut clip = audio_clip(4.0, TimeMode::Musical);
        clip.warp_markers = vec![WarpMarker::new(0.5, 2.0)];

        let plan = plan_for_clip(&clip, &map, 1.0);

        assert_eq!(plan.segments.len(), 2, "{plan:?}");
        assert!(close(plan.segments[0].ratio, 2.0), "{:?}", plan.segments[0]);
        assert!(close(plan.segments[1].ratio, 4.0), "{:?}", plan.segments[1]);
        assert!(close(plan.total_ratio, 3.0));
    }

    #[test]
    fn marcador_que_cruzaria_e_descartado() {
        // Dois marcadores com o MESMO offset de projeto: o segundo teria
        // duracao zero (ou negativa) no projeto e e descartado, mantendo o
        // mapa monotonico.
        let mut clip = audio_clip(4.0, TimeMode::Musical);
        clip.warp_markers = vec![WarpMarker::new(0.3, 2.0), WarpMarker::new(0.6, 2.0)];

        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 1.0);

        assert_eq!(plan.segments.len(), 2, "{plan:?}");
    }

    #[test]
    fn regiao_invalida_nao_divide_por_zero() {
        let clip = audio_clip(4.0, TimeMode::Musical);
        let map = TempoMap::from_bpm(120.0);

        for len in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let plan = plan_for_clip(&clip, &map, len);

            assert!(close(plan.total_ratio, 1.0), "len {len}");
            assert!(plan.segments.iter().all(|s| s.ratio.is_finite()));
        }
    }

    #[test]
    fn marcadores_nos_extremos_nao_criam_trecho_vazio() {
        let mut clip = audio_clip(4.0, TimeMode::Musical);
        clip.warp_markers = vec![
            WarpMarker::new(0.0, 0.0),
            WarpMarker::new(1.0, 4.0),
        ];

        let map = TempoMap::from_bpm(120.0);
        let plan = plan_for_clip(&clip, &map, 1.0);

        assert_eq!(plan.segments.len(), 1, "{plan:?}");
        assert!(close(plan.segments[0].ratio, 2.0));
    }
}