//! fades.rs — Fade In, Fade Out e Gain/Volume para clips de áudio.
//!
//! Fornece estruturas de dados para fades e volume, avaliação de curvas Bézier
//! (linear, côncava, convexa, s-curve), detecção de handles interativos (bolinhas)
//! e geração dos retângulos de renderização (`RectInstance`) para:
//!   - Preenchimento sombreado da área atenuada do fade
//!   - Traçado das curvas Bézier com espessura suave
//!   - Linha de nível de ganho/volume
//!   - Bolinhas/handles redondos com feedback de hover e arrasto ativo

use super::RectInstance;

/// Tipos de handles (bolinhas) interativas de um clip.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FadeHandleKind {
    /// Bolinha superior esquerda: define a duração do Fade In.
    FadeInLen,
    /// Bolinha central da curva: ajusta a tensão Bézier do Fade In.
    FadeInCurve,
    /// Bolinha superior direita: define a duração do Fade Out.
    FadeOutLen,
    /// Bolinha central da curva: ajusta a tensão Bézier do Fade Out.
    FadeOutCurve,
    /// Bolinha na linha de volume/ganho: ajusta o nível em dB.
    GainLevel,
}

/// Parâmetros de Fade e Ganho de um clip de áudio.
#[derive(Clone, Debug)]
pub struct ClipFade {
    /// Duração do fade in em beats (>= 0.0).
    pub in_len: f32,
    /// Tensão Bézier do fade in: -1.0 (exponencial) a +1.0 (logarítmica), 0.0 = linear.
    pub in_curve: f32,
    /// Duração do fade out em beats (>= 0.0).
    pub out_len: f32,
    /// Tensão Bézier do fade out: -1.0 a +1.0, 0.0 = linear.
    pub out_curve: f32,
    /// Nível de ganho em decibéis (-24.0 dB a +12.0 dB, 0.0 dB = ganho unitário).
    pub gain_db: f32,
}

impl Default for ClipFade {
    fn default() -> Self {
        Self {
            in_len: 1.0,      // 1 beat de fade in demo
            in_curve: 0.0,    // linear por padrão
            out_len: 1.0,     // 1 beat de fade out demo
            out_curve: 0.0,   // linear por padrão
            gain_db: 0.0,     // 0 dB (100% volume)
        }
    }
}

#[allow(dead_code)]
impl ClipFade {
    /// Cria uma nova configuração padrão de fades.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cria fades vazios (sem fade in/out e ganho 0dB).
    pub fn none() -> Self {
        Self {
            in_len: 0.0,
            in_curve: 0.0,
            out_len: 0.0,
            out_curve: 0.0,
            gain_db: 0.0,
        }
    }

    /// Fator de amplitude linear correspondente ao ganho em dB.
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        10.0f32.powf(self.gain_db / 20.0)
    }

    /// Converte o ganho dB em posição Y relativa [0.0 = topo (+12dB), 1.0 = base (-24dB)].
    /// 0 dB fica exatamente no centro vertical da área de áudio (y = 0.5).
    pub fn gain_to_norm_y(&self) -> f32 {
        // Range: +12 dB (topo) a -24 dB (base)
        let min_db = -24.0;
        let max_db = 12.0;
        let clamped = self.gain_db.clamp(min_db, max_db);
        1.0 - (clamped - min_db) / (max_db - min_db)
    }

    /// Converte uma posição normalizada Y [0.0..1.0] de volta para dB.
    pub fn norm_y_to_gain(norm_y: f32) -> f32 {
        let min_db = -24.0;
        let max_db = 12.0;
        let y_clamped = norm_y.clamp(0.0, 1.0);
        min_db + (1.0 - y_clamped) * (max_db - min_db)
    }
}

// ============================================================================
// Curvas Bézier
// ============================================================================

/// Avalia a curva Bézier normalizada para um progresso t em [0.0, 1.0].
/// - `t`: progresso de 0.0 (início) a 1.0 (fim).
/// - `tension`: fator de curvatura entre -1.0 e +1.0.
///   - 0.0: linear
///   - > 0.0: curva convexa / logarítmica (sobe rápido no fade in)
///   - < 0.0: curva côncava / exponencial (sobe lento no fade in)
pub fn eval_bezier(t: f32, tension: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let tension = tension.clamp(-1.0, 1.0);

    if tension.abs() < 0.001 {
        return t;
    }

    // Ponto de controle central da Bézier quadrática:
    // P0 = (0, 0), P1 = (0.5 - 0.35 * tension, 0.5 + 0.35 * tension), P2 = (1, 1)
    let _p1_x = 0.5 - 0.35 * tension;
    let p1_y = 0.5 + 0.35 * tension;

    // Aproximação inversa da parametrização de tempo para o valor X dado t:
    // Como queremos Y para um dado X=t, usamos a relação de curvatura de Bernstein:
    let u = t;
    let one_minus_u = 1.0 - u;
    // B_y(u) = 2*(1-u)*u*p1_y + u^2
    let y = 2.0 * one_minus_u * u * p1_y + u * u;

    // Mistura com expoente para maior fidelidade em extremos
    let exp_factor = if tension > 0.0 {
        t.powf(1.0 - 0.65 * tension)
    } else {
        1.0 - (1.0 - t).powf(1.0 + 0.65 * tension)
    };

    (0.5 * y + 0.5 * exp_factor).clamp(0.0, 1.0)
}

/// Avalia o valor do Fade In [0.0..1.0] dado um progresso t em [0.0..1.0].
#[inline]
pub fn eval_fade_in(t: f32, tension: f32) -> f32 {
    eval_bezier(t, tension)
}

/// Avalia o valor do Fade Out [1.0..0.0] dado um progresso t em [0.0..1.0].
#[inline]
pub fn eval_fade_out(t: f32, tension: f32) -> f32 {
    1.0 - eval_bezier(t, -tension)
}

// ============================================================================
// Estado de Interação com os Handles
// ============================================================================

/// Armazena o estado de arrasto ativo de um handle de fade/ganho.
#[derive(Clone, Debug)]
pub struct ActiveFadeDrag {
    pub clip_index: usize,
    pub kind: FadeHandleKind,
    pub start_cursor: (f32, f32),
    pub orig_in_len: f32,
    pub orig_in_curve: f32,
    pub orig_out_len: f32,
    pub orig_out_curve: f32,
    pub orig_gain_db: f32,
}

/// Gerenciador de interação (hover e arrasto) para fades e ganho.
#[derive(Clone, Debug, Default)]
pub struct FadeInteraction {
    pub hovered: Option<(usize, FadeHandleKind)>,
    pub active_drag: Option<ActiveFadeDrag>,
}

impl FadeInteraction {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dragging(&self) -> bool {
        self.active_drag.is_some()
    }

    /// Inicia o arrasto se um handle estiver com hover.
    pub fn begin_drag(&mut self, cursor: (f32, f32), fades: &[ClipFade]) -> bool {
        if let Some((clip_idx, kind)) = self.hovered {
            if let Some(fade) = fades.get(clip_idx) {
                self.active_drag = Some(ActiveFadeDrag {
                    clip_index: clip_idx,
                    kind,
                    start_cursor: cursor,
                    orig_in_len: fade.in_len,
                    orig_in_curve: fade.in_curve,
                    orig_out_len: fade.out_len,
                    orig_out_curve: fade.out_curve,
                    orig_gain_db: fade.gain_db,
                });
                return true;
            }
        }
        false
    }

    /// Atualiza o valor do fade ou ganho durante o arrasto do mouse.
    pub fn update_drag(
        &mut self,
        cursor: (f32, f32),
        fades: &mut [ClipFade],
        px_per_beat: f32,
        clip_len_beats: f32,
        body_h: f32,
    ) {
        let Some(ref drag) = self.active_drag else { return };
        let Some(fade) = fades.get_mut(drag.clip_index) else { return };

        let dx = cursor.0 - drag.start_cursor.0;
        let dy = cursor.1 - drag.start_cursor.1;

        match drag.kind {
            FadeHandleKind::FadeInLen => {
                let delta_beats = dx / px_per_beat.max(1.0);
                let max_len = (clip_len_beats - fade.out_len).max(0.0);
                fade.in_len = (drag.orig_in_len + delta_beats).clamp(0.0, max_len);
            }
            FadeHandleKind::FadeInCurve => {
                let delta_tension = -dy / 50.0 + dx / 50.0;
                fade.in_curve = (drag.orig_in_curve + delta_tension).clamp(-1.0, 1.0);
            }
            FadeHandleKind::FadeOutLen => {
                let delta_beats = -dx / px_per_beat.max(1.0);
                let max_len = (clip_len_beats - fade.in_len).max(0.0);
                fade.out_len = (drag.orig_out_len + delta_beats).clamp(0.0, max_len);
            }
            FadeHandleKind::FadeOutCurve => {
                let delta_tension = dy / 50.0 + dx / 50.0;
                fade.out_curve = (drag.orig_out_curve + delta_tension).clamp(-1.0, 1.0);
            }
            FadeHandleKind::GainLevel => {
                let delta_norm = dy / body_h.max(1.0);
                let min_db = -24.0;
                let max_db = 12.0;
                let delta_db = -delta_norm * (max_db - min_db);
                fade.gain_db = (drag.orig_gain_db + delta_db).clamp(min_db, max_db);
            }
        }
    }

    /// Finaliza o arrasto atual.
    pub fn end_drag(&mut self) {
        self.active_drag = None;
    }
}

// ============================================================================
// Renderização: Bolinhas e Curvas Bézier
// ============================================================================

/// Cria uma instância de círculo perfeito (bolinha) com borda.
#[inline]
pub fn push_handle_circle(
    instances: &mut Vec<RectInstance>,
    cx: f32,
    cy: f32,
    radius: f32,
    fill_color: [u8; 4],
    border_color: [u8; 4],
) {
    let d = radius * 2.0;
    // Borda exterior
    instances.push(RectInstance {
        x: cx - radius,
        y: cy - radius,
        w: d,
        h: d,
        color: border_color,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius,
    });

    // Miolo interno (1px menor para criar o anel de borda)
    let inner_r = (radius - 1.2).max(1.0);
    let inner_d = inner_r * 2.0;
    instances.push(RectInstance {
        x: cx - inner_r,
        y: cy - inner_r,
        w: inner_d,
        h: inner_d,
        color: fill_color,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: inner_r,
    });
}

/// Gera todas as instâncias visuais para os fades (in/out) e volume de um clip.
///
/// Desenha:
/// 1. Sombra preenchida sob a curva Bézier do Fade In e Fade Out
/// 2. Traçado contínuo da curva Bézier de Fade In
/// 3. Traçado contínuo da curva Bézier de Fade Out
/// 4. Linha horizontal de Ganho / Volume
/// 5. Bolinhas (handles) interativas no topo e nas curvas
pub fn build_clip_fade_instances(
    clip_index: usize,
    fade: &ClipFade,
    body_x: f32,
    body_y: f32,
    body_w: f32,
    body_h: f32,
    px_per_beat: f32,
    inter: &FadeInteraction,
    instances: &mut Vec<RectInstance>,
) {
    if body_w <= 2.0 || body_h <= 2.0 {
        return;
    }

    let is_hovered = |kind: FadeHandleKind| -> bool {
        inter.hovered == Some((clip_index, kind))
            || inter.active_drag.as_ref().map_or(false, |d| d.clip_index == clip_index && d.kind == kind)
    };

    // Cores do tema profissional
    let fade_shadow_color = [15, 15, 20, 90];     // Sombra translúcida sob o fade
    let curve_stroke_color = [255, 255, 255, 210]; // Linha branca da curva Bézier
    let gain_line_color = [255, 215, 0, 170];      // Linha dourada suave para ganho
    let handle_fill_normal = [245, 245, 250, 240];
    let handle_fill_hover = [0, 200, 83, 255];     // Accent verde esmeralda no hover
    let handle_border_color = [30, 30, 35, 230];

    // ========================================================================
    // 1. FADE IN
    // ========================================================================
    let in_px = (fade.in_len * px_per_beat).min(body_w).max(0.0);
    if in_px > 1.0 {
        // Amostragem da curva Bézier em segmentos verticais
        let steps = (in_px / 4.0).clamp(12.0, 64.0) as usize;
        let step_w = in_px / steps as f32;

        let mut prev_pt = (body_x, body_y + body_h);

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let gain = eval_fade_in(t, fade.in_curve);
            let cur_x = body_x + t * in_px;
            let cur_y = body_y + body_h - gain * body_h;

            if i > 0 {
                // Sombra da atenuação: do topo até a curva
                let seg_x = body_x + (i - 1) as f32 * step_w;
                let top_y = body_y;
                let fill_h = (cur_y - top_y).max(0.0);
                if fill_h > 0.5 {
                    instances.push(RectInstance {
                        x: seg_x,
                        y: top_y,
                        w: step_w + 0.5,
                        h: fill_h,
                        color: fade_shadow_color,
                        wave_offset: 0,
                        wave_count: 0,
                        clip_w: 0.0,
                        clip_h: 0.0,
                        radius: 0.0,
                    });
                }

                // Traço da linha Bézier (segmento fino conectando os pontos)
                let stroke_w = (cur_x - prev_pt.0).max(1.0);
                let stroke_h = (cur_y - prev_pt.1).abs().max(1.5);
                let stroke_y = cur_y.min(prev_pt.1);
                instances.push(RectInstance {
                    x: prev_pt.0,
                    y: stroke_y,
                    w: stroke_w,
                    h: stroke_h,
                    color: curve_stroke_color,
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });
            }
            prev_pt = (cur_x, cur_y);
        }

        // Bolinha Fade In Length (no canto superior do fade in)
        let handle_in_x = body_x + in_px;
        let handle_in_y = body_y + 4.0;
        let hover_len = is_hovered(FadeHandleKind::FadeInLen);
        let r_len = if hover_len { 5.5 } else { 4.5 };
        let fill_len = if hover_len { handle_fill_hover } else { handle_fill_normal };
        push_handle_circle(instances, handle_in_x, handle_in_y, r_len, fill_len, handle_border_color);

        // Bolinha Fade In Curve (no centro da curva de Bézier)
        let mid_t = 0.5;
        let mid_gain = eval_fade_in(mid_t, fade.in_curve);
        let handle_curve_x = body_x + mid_t * in_px;
        let handle_curve_y = body_y + body_h - mid_gain * body_h;
        let hover_curve = is_hovered(FadeHandleKind::FadeInCurve);
        let r_curve = if hover_curve { 4.5 } else { 3.5 };
        let fill_curve = if hover_curve { handle_fill_hover } else { [210, 210, 220, 220] };
        push_handle_circle(instances, handle_curve_x, handle_curve_y, r_curve, fill_curve, handle_border_color);
    } else {
        // Indicador pequeno de Fade In inativo quando hovering próximo ao topo esquerdo
        if is_hovered(FadeHandleKind::FadeInLen) {
            push_handle_circle(instances, body_x + 4.0, body_y + 4.0, 4.0, handle_fill_hover, handle_border_color);
        }
    }

    // ========================================================================
    // 2. FADE OUT
    // ========================================================================
    let out_px = (fade.out_len * px_per_beat).min(body_w).max(0.0);
    if out_px > 1.0 {
        let start_x = body_x + body_w - out_px;
        let steps = (out_px / 4.0).clamp(12.0, 64.0) as usize;
        let step_w = out_px / steps as f32;

        let mut prev_pt = (start_x, body_y);

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let gain = eval_fade_out(t, fade.out_curve);
            let cur_x = start_x + t * out_px;
            let cur_y = body_y + body_h - gain * body_h;

            if i > 0 {
                // Sombra da atenuação no fade out
                let seg_x = start_x + (i - 1) as f32 * step_w;
                let top_y = body_y;
                let fill_h = (cur_y - top_y).max(0.0);
                if fill_h > 0.5 {
                    instances.push(RectInstance {
                        x: seg_x,
                        y: top_y,
                        w: step_w + 0.5,
                        h: fill_h,
                        color: fade_shadow_color,
                        wave_offset: 0,
                        wave_count: 0,
                        clip_w: 0.0,
                        clip_h: 0.0,
                        radius: 0.0,
                    });
                }

                // Traço da linha Bézier
                let stroke_w = (cur_x - prev_pt.0).max(1.0);
                let stroke_h = (cur_y - prev_pt.1).abs().max(1.5);
                let stroke_y = cur_y.min(prev_pt.1);
                instances.push(RectInstance {
                    x: prev_pt.0,
                    y: stroke_y,
                    w: stroke_w,
                    h: stroke_h,
                    color: curve_stroke_color,
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });
            }
            prev_pt = (cur_x, cur_y);
        }

        // Bolinha Fade Out Length (no canto superior do fade out)
        let handle_out_x = start_x;
        let handle_out_y = body_y + 4.0;
        let hover_len = is_hovered(FadeHandleKind::FadeOutLen);
        let r_len = if hover_len { 5.5 } else { 4.5 };
        let fill_len = if hover_len { handle_fill_hover } else { handle_fill_normal };
        push_handle_circle(instances, handle_out_x, handle_out_y, r_len, fill_len, handle_border_color);

        // Bolinha Fade Out Curve (no centro da curva)
        let mid_t = 0.5;
        let mid_gain = eval_fade_out(mid_t, fade.out_curve);
        let handle_curve_x = start_x + mid_t * out_px;
        let handle_curve_y = body_y + body_h - mid_gain * body_h;
        let hover_curve = is_hovered(FadeHandleKind::FadeOutCurve);
        let r_curve = if hover_curve { 4.5 } else { 3.5 };
        let fill_curve = if hover_curve { handle_fill_hover } else { [210, 210, 220, 220] };
        push_handle_circle(instances, handle_curve_x, handle_curve_y, r_curve, fill_curve, handle_border_color);
    } else {
        if is_hovered(FadeHandleKind::FadeOutLen) {
            push_handle_circle(instances, body_x + body_w - 4.0, body_y + 4.0, 4.0, handle_fill_hover, handle_border_color);
        }
    }

    // ========================================================================
    // 3. GAIN / VOLUME (Linha e Bolinha Central)
    // ========================================================================
    let gain_norm_y = fade.gain_to_norm_y();
    let gain_y = body_y + gain_norm_y * body_h;

    // Linha horizontal de volume
    instances.push(RectInstance {
        x: body_x,
        y: gain_y - 0.75,
        w: body_w,
        h: 1.5,
        color: gain_line_color,
        wave_offset: 0,
        wave_count: 0,
        clip_w: 0.0,
        clip_h: 0.0,
        radius: 0.0,
    });

    // Bolinha de ajuste de ganho (no centro horizontal da linha de ganho)
    let gain_handle_x = body_x + body_w * 0.5;
    let hover_gain = is_hovered(FadeHandleKind::GainLevel);
    let r_gain = if hover_gain { 6.0 } else { 4.5 };
    let fill_gain = if hover_gain { [255, 230, 80, 255] } else { [255, 215, 0, 230] };
    push_handle_circle(instances, gain_handle_x, gain_y, r_gain, fill_gain, handle_border_color);
}

// ============================================================================
// Hit Testing para detecção do cursor do mouse sobre as bolinhas
// ============================================================================

/// Testa se o cursor está sobre alguma das bolinhas de fade ou ganho de um clip.
pub fn hit_test_clip_handles(
    _clip_index: usize,
    fade: &ClipFade,
    body_x: f32,
    body_y: f32,
    body_w: f32,
    body_h: f32,
    px_per_beat: f32,
    cursor: (f32, f32),
) -> Option<FadeHandleKind> {
    let hit_dist_sq = |hx: f32, hy: f32, r: f32| -> bool {
        let dx = cursor.0 - hx;
        let dy = cursor.1 - hy;
        dx * dx + dy * dy <= (r + 4.0) * (r + 4.0) // margem de tolerância de 4px para clique fácil
    };

    // 1. Handle Fade In Length
    let in_px = (fade.in_len * px_per_beat).min(body_w).max(0.0);
    let handle_in_x = if in_px > 1.0 { body_x + in_px } else { body_x + 4.0 };
    let handle_in_y = body_y + 4.0;
    if hit_dist_sq(handle_in_x, handle_in_y, 5.0) {
        return Some(FadeHandleKind::FadeInLen);
    }

    // 2. Handle Fade In Curve
    if in_px > 1.0 {
        let mid_gain = eval_fade_in(0.5, fade.in_curve);
        let handle_curve_x = body_x + 0.5 * in_px;
        let handle_curve_y = body_y + body_h - mid_gain * body_h;
        if hit_dist_sq(handle_curve_x, handle_curve_y, 4.5) {
            return Some(FadeHandleKind::FadeInCurve);
        }
    }

    // 3. Handle Fade Out Length
    let out_px = (fade.out_len * px_per_beat).min(body_w).max(0.0);
    let handle_out_x = if out_px > 1.0 { body_x + body_w - out_px } else { body_x + body_w - 4.0 };
    let handle_out_y = body_y + 4.0;
    if hit_dist_sq(handle_out_x, handle_out_y, 5.0) {
        return Some(FadeHandleKind::FadeOutLen);
    }

    // 4. Handle Fade Out Curve
    if out_px > 1.0 {
        let start_x = body_x + body_w - out_px;
        let mid_gain = eval_fade_out(0.5, fade.out_curve);
        let handle_curve_x = start_x + 0.5 * out_px;
        let handle_curve_y = body_y + body_h - mid_gain * body_h;
        if hit_dist_sq(handle_curve_x, handle_curve_y, 4.5) {
            return Some(FadeHandleKind::FadeOutCurve);
        }
    }

    // 5. Handle Gain / Volume
    let gain_norm_y = fade.gain_to_norm_y();
    let gain_y = body_y + gain_norm_y * body_h;
    let gain_x = body_x + body_w * 0.5;
    if hit_dist_sq(gain_x, gain_y, 5.5) {
        return Some(FadeHandleKind::GainLevel);
    }

    None
}
