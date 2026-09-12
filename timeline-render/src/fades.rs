//! fades.rs — Fade In, Fade Out e Gain/Volume para clips de áudio.
//!
//! Fornece estruturas de dados para fades e volume, avaliação de curvas Bézier 2D
//! controladas livremente pelo cursor do mouse (arrasto em X para esquerda/direita
//! e Y para cima/baixo), detecção de handles interativos (bolinhas) e renderização
//! analítica no shader WGSL com SDF contínuo e anti-aliasing de subpixel.

use super::RectInstance;

/// Tipos de handles (bolinhas) interativas de um clip.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FadeHandleKind {
    /// Bolinha superior esquerda: define a duração do Fade In.
    FadeInLen,
    /// Bolinha da curva: ajusta a posição e curvatura Bézier 2D do Fade In.
    FadeInCurve,
    /// Bolinha superior direita: define a duração do Fade Out.
    FadeOutLen,
    /// Bolinha da curva: ajusta a posição e curvatura Bézier 2D do Fade Out.
    FadeOutCurve,
    /// Bolinha na linha de volume/ganho: ajusta o nível em dB.
    GainLevel,
}

/// Eixo de trava durante o arrasto de um handle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DragAxisLock {
    /// Arrasto responde apenas ao movimento horizontal (fades in/out).
    Horizontal,
    /// Arrasto responde apenas ao movimento vertical (ganho/volume).
    Vertical,
}

impl FadeHandleKind {
    /// Eixo travado durante o arrasto de cada tipo de handle.
    fn drag_axis(self) -> Option<DragAxisLock> {
        match self {
            FadeHandleKind::GainLevel => Some(DragAxisLock::Vertical),
            FadeHandleKind::FadeInLen
            | FadeHandleKind::FadeInCurve
            | FadeHandleKind::FadeOutLen
            | FadeHandleKind::FadeOutCurve => Some(DragAxisLock::Horizontal),
        }
    }
}

/// Parâmetros de Fade e Ganho de um clip de áudio.
#[derive(Clone, Debug)]
pub struct ClipFade {
    /// Duração do fade in em beats (>= 0.0).
    pub in_len: f32,
    /// Ponto de controle Bézier do Fade In: X normalizado [0.0..1.0] dentro do fade in. Padrão 0.5.
    pub in_curve_x: f32,
    /// Ponto de controle Bézier do Fade In: Y normalizado [0.0..1.0] de amplitude. Padrão 0.5.
    pub in_curve_y: f32,
    /// Duração do fade out em beats (>= 0.0).
    pub out_len: f32,
    /// Ponto de controle Bézier do Fade Out: X normalizado [0.0..1.0] dentro do fade out. Padrão 0.5.
    pub out_curve_x: f32,
    /// Ponto de controle Bézier do Fade Out: Y normalizado [0.0..1.0] de amplitude. Padrão 0.5.
    pub out_curve_y: f32,
    /// Nível de ganho em decibéis (-24.0 dB a +12.0 dB, 0.0 dB = ganho unitário).
    pub gain_db: f32,
}

impl Default for ClipFade {
    fn default() -> Self {
        Self {
            in_len: 1.0,      // 1 beat de fade in demo
            in_curve_x: 0.5,  // centro horizontal (curva linear/neutra)
            in_curve_y: 0.5,  // centro vertical
            out_len: 1.0,     // 1 beat de fade out demo
            out_curve_x: 0.5, // centro horizontal
            out_curve_y: 0.5, // centro vertical
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
            in_curve_x: 0.5,
            in_curve_y: 0.5,
            out_len: 0.0,
            out_curve_x: 0.5,
            out_curve_y: 0.5,
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
// Avaliação de Curva Bézier 2D Analítica (CPU)
// ============================================================================

/// Calcula o Y normalizado [0.0..1.0] de uma curva Bézier quadrática 2D para um dado X [0.0..1.0].
/// P0 = (0, 0), P1 = (cx, cy), P2 = (1, 1).
#[allow(dead_code)]
pub fn eval_bezier_2d_in(x: f32, cx: f32, cy: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let cx = cx.clamp(0.02, 0.98);
    let cy = cy.clamp(0.0, 1.0);

    let denom = 1.0 - 2.0 * cx;
    let u = if denom.abs() < 0.001 {
        x
    } else {
        let discr = (cx * cx + denom * x).max(0.0);
        ((-cx + discr.sqrt()) / denom).clamp(0.0, 1.0)
    };

    2.0 * (1.0 - u) * u * cy + u * u
}

/// Calcula o Y normalizado [1.0..0.0] de uma curva Bézier quadrática 2D de Fade Out para um dado X [0.0..1.0].
/// P0 = (0, 1), P1 = (cx, cy), P2 = (1, 0).
#[allow(dead_code)]
pub fn eval_bezier_2d_out(x: f32, cx: f32, cy: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let cx = cx.clamp(0.02, 0.98);
    let cy = cy.clamp(0.0, 1.0);

    let denom = 1.0 - 2.0 * cx;
    let u = if denom.abs() < 0.001 {
        x
    } else {
        let discr = (cx * cx + denom * x).max(0.0);
        ((-cx + discr.sqrt()) / denom).clamp(0.0, 1.0)
    };

    let one_minus_u = 1.0 - u;
    one_minus_u * one_minus_u + 2.0 * one_minus_u * u * cy
}

// ============================================================================
// Estado de Interação com os Handles
// ============================================================================

/// Armazena o estado de arrasto ativo de um handle de fade/ganho.
#[derive(Clone, Debug)]
pub struct ActiveFadeDrag {
    pub clip_index: usize,
    pub kind: FadeHandleKind,
    /// Eixo travado durante o arrasto (vazio = livre em 2D).
    pub axis_lock: Option<DragAxisLock>,
    pub start_cursor: (f32, f32),
    pub orig_in_len: f32,
    pub orig_in_curve_x: f32,
    pub orig_in_curve_y: f32,
    pub orig_out_len: f32,
    pub orig_out_curve_x: f32,
    pub orig_out_curve_y: f32,
    pub orig_gain_db: f32,
}

/// Gerenciador de interação (hover e arrasto) para fades e ganho.
#[derive(Clone, Debug, Default)]
pub struct FadeInteraction {
    pub hovered: Option<(usize, FadeHandleKind)>,
    pub active_drag: Option<ActiveFadeDrag>,
    /// True após o snap magnético disparar uma vez — evita "resistência" de
    /// puxar o cursor de volta ao handle a cada frame. Resetado quando o
    /// hover se perde.
    pub snap_active: bool,
}

impl FadeInteraction {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dragging(&self) -> bool {
        self.active_drag.is_some()
    }

    /// Inicia o arrasto se um handle estiver com hover.
    ///
    /// O cursor é *snapado* para a posição exata do handle para que o arrasto
    /// comece do centro da bolinha, garantindo movimento suave e preciso.
    pub fn begin_drag(
        &mut self,
        cursor: (f32, f32),
        fades: &[ClipFade],
        px_per_beat: f32,
        body_x: f32,
        body_y: f32,
        body_w: f32,
        body_h: f32,
    ) -> bool {
        if let Some((clip_idx, kind)) = self.hovered {
            if let Some(fade) = fades.get(clip_idx) {
                // Snap: ajusta o cursor inicial para a posição exata do handle.
                let snapped_cursor = match kind {
                    FadeHandleKind::GainLevel => {
                        let gain_y = body_y + body_h * fade.gain_to_norm_y();
                        (cursor.0, gain_y)
                    }
                    _ => handle_position(kind, fade, body_x, body_y, body_w, body_h, px_per_beat)
                        .unwrap_or(cursor),
                };
                self.active_drag = Some(ActiveFadeDrag {
                    clip_index: clip_idx,
                    kind,
                    axis_lock: kind.drag_axis(),
                    start_cursor: snapped_cursor,
                    orig_in_len: fade.in_len,
                    orig_in_curve_x: fade.in_curve_x,
                    orig_in_curve_y: fade.in_curve_y,
                    orig_out_len: fade.out_len,
                    orig_out_curve_x: fade.out_curve_x,
                    orig_out_curve_y: fade.out_curve_y,
                    orig_gain_db: fade.gain_db,
                });
                return true;
            }
        }
        false
    }

    /// Atualiza o valor do fade ou ganho durante o arrasto do mouse.
    ///
    /// O arrasto em X e Y move o ponto de controle Bézier diretamente, fazendo a curva
    /// seguir exatamente a direção do cursor do mouse.
    pub fn update_drag(
        &mut self,
        cursor: (f32, f32),
        fades: &mut [ClipFade],
        px_per_beat: f32,
        clip_len_beats: f32,
        body_w: f32,
        body_h: f32,
    ) {
        let Some(ref drag) = self.active_drag else {
            return;
        };
        let Some(fade) = fades.get_mut(drag.clip_index) else {
            return;
        };

        let mut dx = cursor.0 - drag.start_cursor.0;
        let mut dy = cursor.1 - drag.start_cursor.1;

        // Travamento de eixo: handle de fade (in/out) responde apenas a X;
        // handle de ganho/volume responde apenas a Y. Ao soltar, destrava.
        match drag.axis_lock {
            Some(DragAxisLock::Horizontal) => dy = 0.0,
            Some(DragAxisLock::Vertical) => dx = 0.0,
            None => {}
        }

        match drag.kind {
            FadeHandleKind::FadeInLen => {
                let delta_beats = dx / px_per_beat.max(1.0);
                let max_len = (clip_len_beats - fade.out_len).max(0.0);
                fade.in_len = (drag.orig_in_len + delta_beats).clamp(0.0, max_len);
            }
            FadeHandleKind::FadeInCurve => {
                // Arrasto horizontal (dx) controla X do ponto de controle Bézier
                // Arrasto vertical (dy) controla Y do ponto de controle Bézier
                // Usa a MESMA largura clampada do render (min(body_w)) para o delta
                // normalizado não "desgrudar" da curva visual.
                let in_px = (fade.in_len * px_per_beat).min(body_w).max(10.0);
                let delta_norm_x = dx / in_px;
                let delta_norm_y = -dy / body_h.max(10.0); // arrastar pra cima aumenta Y
                fade.in_curve_x = (drag.orig_in_curve_x + delta_norm_x).clamp(0.05, 0.95);
                fade.in_curve_y = (drag.orig_in_curve_y + delta_norm_y).clamp(0.0, 1.0);
            }
            FadeHandleKind::FadeOutLen => {
                let delta_beats = -dx / px_per_beat.max(1.0);
                let max_len = (clip_len_beats - fade.in_len).max(0.0);
                fade.out_len = (drag.orig_out_len + delta_beats).clamp(0.0, max_len);
            }
            FadeHandleKind::FadeOutCurve => {
                // Mesma largura clampada do render para consistência visual
                let out_px = (fade.out_len * px_per_beat).min(body_w).max(10.0);
                let delta_norm_x = dx / out_px;
                let delta_norm_y = -dy / body_h.max(10.0);
                fade.out_curve_x = (drag.orig_out_curve_x + delta_norm_x).clamp(0.05, 0.95);
                fade.out_curve_y = (drag.orig_out_curve_y + delta_norm_y).clamp(0.0, 1.0);
            }
            FadeHandleKind::GainLevel => {
                // Área de ganho: ajusta ganho com base na posição vertical.
                // O divisor é a altura útil do corpo (x2): para varrer todo o
                // range de dB é preciso arrastar bastante, então cada pixel
                // mexe pouco no ganho (arrastar mais, mover menos).
                let min_db = -24.0;
                let max_db = 12.0;
                let gain_drag_range = (body_h * 2.0).max(48.0);
                let delta_norm = dy / gain_drag_range.max(1.0);
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
// Renderização: Bolinhas e Shaders SDF
// ============================================================================

/// Cria uma instância de círculo perfeito (bolinha) com anel de borda anti-aliased.
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
    // Borda exterior (círculo com raio total)
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

    // Miolo interno preenchido
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
/// O shader WGSL calcula o SDF da curva Bézier 2D e anti-aliasing analítico de subpixel.
/// As bolinhas (handles) acompanham a curva e a direção do mouse.
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
            || inter
                .active_drag
                .as_ref()
                .map_or(false, |d| d.clip_index == clip_index && d.kind == kind)
    };

    // Cores profissionais estilo DAW
    let _gain_line_color = [255, 215, 0, 160]; // Linha dourada suave para ganho
    let _handle_fill_normal = [250, 250, 255, 245];
    let handle_fill_hover = [0, 210, 90, 255]; // Accent esmeralda no hover
    let handle_border_color = [25, 25, 30, 240];

    // ========================================================================
    // 1. FADE IN — SHADER ANALÍTICO SDF (overlay visível apenas no hover)
    // ========================================================================
    let in_px = (fade.in_len * px_per_beat).min(body_w).max(0.0);
    let hover_fade_in =
        is_hovered(FadeHandleKind::FadeInLen) || is_hovered(FadeHandleKind::FadeInCurve);
    if in_px > 1.0 && hover_fade_in {
        let cx_code = (fade.in_curve_x.clamp(0.0, 1.0) * 1000.0).round() as u32;
        let cy_code = (fade.in_curve_y.clamp(0.0, 1.0) * 1000.0).round() as u32;
        instances.push(RectInstance {
            x: body_x,
            y: body_y,
            w: in_px,
            h: body_h,
            color: [0, 0, 0, 0],
            wave_offset: cx_code,
            wave_count: cy_code,
            clip_w: -2.0, // sinaliza Fade In SDF
            clip_h: body_h,
            radius: 0.0,
        });

        // Handles (apenas no hover)
        let handle_in_x = body_x + in_px;
        let handle_in_y = body_y + 4.0;
        push_handle_circle(
            instances,
            handle_in_x,
            handle_in_y,
            5.5,
            handle_fill_hover,
            handle_border_color,
        );

        let handle_curve_x = body_x + fade.in_curve_x * in_px;
        let handle_curve_y = body_y + body_h - fade.in_curve_y * body_h;
        push_handle_circle(
            instances,
            handle_curve_x,
            handle_curve_y,
            5.0,
            handle_fill_hover,
            handle_border_color,
        );
    } else {
        if is_hovered(FadeHandleKind::FadeInLen) {
            push_handle_circle(
                instances,
                body_x + 4.0,
                body_y + 4.0,
                4.0,
                handle_fill_hover,
                handle_border_color,
            );
        }
    }

    // ========================================================================
    // 2. FADE OUT — SHADER ANALÍTICO SDF (overlay visível apenas no hover)
    // ========================================================================
    let out_px = (fade.out_len * px_per_beat).min(body_w).max(0.0);
    let hover_fade_out =
        is_hovered(FadeHandleKind::FadeOutLen) || is_hovered(FadeHandleKind::FadeOutCurve);
    if out_px > 1.0 && hover_fade_out {
        let start_x = body_x + body_w - out_px;
        let cx_code = (fade.out_curve_x.clamp(0.0, 1.0) * 1000.0).round() as u32;
        let cy_code = (fade.out_curve_y.clamp(0.0, 1.0) * 1000.0).round() as u32;
        instances.push(RectInstance {
            x: start_x,
            y: body_y,
            w: out_px,
            h: body_h,
            color: [0, 0, 0, 0],
            wave_offset: cx_code,
            wave_count: cy_code,
            clip_w: -3.0, // sinaliza Fade Out SDF
            clip_h: body_h,
            radius: 0.0,
        });

        // Handles (apenas no hover)
        let handle_out_x = start_x;
        let handle_out_y = body_y + 4.0;
        push_handle_circle(
            instances,
            handle_out_x,
            handle_out_y,
            5.5,
            handle_fill_hover,
            handle_border_color,
        );

        let handle_curve_x = start_x + fade.out_curve_x * out_px;
        let handle_curve_y = body_y + body_h - fade.out_curve_y * body_h;
        push_handle_circle(
            instances,
            handle_curve_x,
            handle_curve_y,
            5.0,
            handle_fill_hover,
            handle_border_color,
        );
    } else {
        if is_hovered(FadeHandleKind::FadeOutLen) {
            push_handle_circle(
                instances,
                body_x + body_w - 4.0,
                body_y + 4.0,
                4.0,
                handle_fill_hover,
                handle_border_color,
            );
        }
    }

    // ========================================================================
    // 3. GAIN / VOLUME — Área no topo do corpo (abaixo do header)
    // ========================================================================
    // Área de ganho: faixa de 14px no topo do corpo do clip.
    // Sempre visível com alpha sutil; no hover fica mais opaca.
    let gain_area_h = 14.0;
    let gain_area_y = body_y;
    let gain_hovered = is_hovered(FadeHandleKind::GainLevel);
    let gradient_strips: i32 = 7;
    let base_alpha: i32 = if gain_hovered { 110 } else { 40 };
    for i in 0..gradient_strips {
        let frac = i as f32 / gradient_strips as f32;
        let strip_alpha = (base_alpha as f32 * (1.0 - frac * 0.85)).round() as u8;
        instances.push(RectInstance {
            x: body_x,
            y: gain_area_y + i as f32 * gain_area_h / gradient_strips as f32,
            w: body_w,
            h: gain_area_h / gradient_strips as f32 + 0.5,
            color: [255, 215, 0, strip_alpha],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });
    }
}
/// Calcula a posição (x, y) na tela de um handle de fade (bolinha).
pub fn handle_position(
    kind: FadeHandleKind,
    fade: &ClipFade,
    body_x: f32,
    body_y: f32,
    body_w: f32,
    body_h: f32,
    px_per_beat: f32,
) -> Option<(f32, f32)> {
    match kind {
        FadeHandleKind::FadeInLen => {
            let in_px = (fade.in_len * px_per_beat).min(body_w).max(0.0);
            let x = if in_px > 1.0 {
                body_x + in_px
            } else {
                body_x + 4.0
            };
            Some((x, body_y + 4.0))
        }
        FadeHandleKind::FadeInCurve => {
            let in_px = (fade.in_len * px_per_beat).min(body_w).max(0.0);
            if in_px > 1.0 {
                let cx = body_x + fade.in_curve_x * in_px;
                let cy = body_y + body_h - fade.in_curve_y * body_h;
                Some((cx, cy))
            } else {
                None
            }
        }
        FadeHandleKind::FadeOutLen => {
            let out_px = (fade.out_len * px_per_beat).min(body_w).max(0.0);
            let x = if out_px > 1.0 {
                body_x + body_w - out_px
            } else {
                body_x + body_w - 4.0
            };
            Some((x, body_y + 4.0))
        }
        FadeHandleKind::FadeOutCurve => {
            let out_px = (fade.out_len * px_per_beat).min(body_w).max(0.0);
            if out_px > 1.0 {
                let start_x = body_x + body_w - out_px;
                let cx = start_x + fade.out_curve_x * out_px;
                let cy = body_y + body_h - fade.out_curve_y * body_h;
                Some((cx, cy))
            } else {
                None
            }
        }
        FadeHandleKind::GainLevel => None,
    }
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
        dx * dx + dy * dy <= (r + 5.0) * (r + 5.0) // margem de 5px para clique confortável
    };

    for kind in [
        FadeHandleKind::FadeInLen,
        FadeHandleKind::FadeInCurve,
        FadeHandleKind::FadeOutLen,
        FadeHandleKind::FadeOutCurve,
    ] {
        if let Some((hx, hy)) =
            handle_position(kind, fade, body_x, body_y, body_w, body_h, px_per_beat)
        {
            if hit_dist_sq(hx, hy, 5.0) {
                return Some(kind);
            }
        }
    }

    // Handle de ganho/volume — área retangular no topo do corpo
    let gain_area_h = 14.0;
    let gain_area_y = body_y;
    if cursor.0 >= body_x
        && cursor.0 <= body_x + body_w
        && cursor.1 >= gain_area_y
        && cursor.1 <= gain_area_y + gain_area_h
    {
        return Some(FadeHandleKind::GainLevel);
    }

    None
}
