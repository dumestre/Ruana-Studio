//! Geometria do timewarp: editor de tempo estilo Cubase (eixo Y = BPM).
//!
//! Estrutura espelha o piano roll (régua idêntica em cima, corpo abaixo),
//! porém sem teclado e sem grid horizontal — só a grade vertical de
//! compassos/batidas + a curva de tempo. A coluna esquerda (onde o piano
//! roll tem o teclado) vira um mini-gutter com os valores de BPM.

use crate::{palette, rgba, RectInstance};

/// Altura da régua do timewarp (mesma da régua do piano roll).
pub const TIMEWARP_RULER_H: f32 = 24.0;

/// Faixa de BPM exibida (fixa, sem scroll vertical — como o editor de
/// tempo do Cubase). BPM alto desenhado no topo.
pub const BPM_MIN: f32 = 20.0;
pub const BPM_MAX: f32 = 300.0;

/// Largura (px) do mini-gutter com os valores de BPM na borda esquerda.
/// Ocupa o lugar do teclado do piano roll (mantém a régua alinhada).
pub const BPM_LABEL_W: f32 = 44.0;

/// Um evento de tempo exibido. Espelha `daw_core::timeline::TempoChange`.
#[derive(Clone, Debug)]
pub struct TempoPoint {
    pub beat: f32,
    pub bpm: f32,
}

/// Conversões cursor ↔ conteúdo do timewarp.
#[derive(Clone)]
pub struct TimewarpMetrics {
    pub px_per_beat: f32,
    pub scroll_x: f32,
    /// Topo da régua (coordenada Y global da janela).
    pub ruler_px: f32,
    /// Altura da régua.
    pub ruler_h: f32,
    /// Topo do corpo (abaixo da régua).
    pub body_top: f32,
    /// Altura do corpo (área da curva de tempo).
    pub body_h: f32,
    pub theme: usize,
}

impl TimewarpMetrics {
    /// Converte beat → coordenada de janela (a partir do gutter de BPM).
    pub fn beat_to_x(&self, beat: f32) -> f32 {
        BPM_LABEL_W + beat * self.px_per_beat - self.scroll_x
    }

    /// Converte x de janela → beat (considera o gutter de BPM).
    pub fn x_to_beat(&self, x: f32) -> f32 {
        (x - BPM_LABEL_W + self.scroll_x) / self.px_per_beat.max(0.001)
    }

    /// Converte BPM → coordenada de janela (BPM alto no topo do corpo).
    pub fn bpm_to_y(&self, bpm: f32) -> f32 {
        let norm = ((BPM_MAX - bpm) / (BPM_MAX - BPM_MIN)).clamp(0.0, 1.0);
        self.body_top + norm * self.body_h
    }

    /// Converte y de janela → BPM (clampado à faixa exibida).
    pub fn y_to_bpm(&self, y: f32) -> f32 {
        let rel = ((y - self.body_top) / self.body_h.max(1.0)).clamp(0.0, 1.0);
        (BPM_MAX - rel * (BPM_MAX - BPM_MIN)).clamp(BPM_MIN, BPM_MAX)
    }

    /// Converte cursor em (beat, bpm).
    pub fn cursor_to_beat_bpm(&self, cursor: (f32, f32)) -> (f32, f32) {
        (self.x_to_beat(cursor.0).max(0.0), self.y_to_bpm(cursor.1))
    }

    /// Retorna as instâncias de fundo do corpo NEXT (grid SÓ vertical, sem
    /// linhas horizontais e sem teclado — ao contrário do piano roll).
    pub fn build_body_instances(&self, width: f32) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6);
        let mut instances = Vec::new();

        let body_top = self.body_top;
        let body_h = self.body_h.max(0.0);
        let body_bottom = body_top + body_h;

        // Fundo base do corpo
        instances.push(RectInstance {
            x: 0.0,
            y: body_top,
            w: width,
            h: body_h,
            color: rgba(palette::TIMELINE_BG[t], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        // Fundos alternados dos compassos (mesmo padrão da timeline) — só a
        // partir do gutter de BPM.
        let px_per_bar = self.px_per_beat * 4.0;
        let total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        for i in 0..total_bars {
            let bx = BPM_LABEL_W + i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let is_odd = i % 2 == 1;
            let bg = if is_odd { palette::SURFACE_HIGH[t] } else { palette::SURFACE_STEP[t] };
            let draw_x = bx.max(BPM_LABEL_W);
            let draw_w = (bx + px_per_bar).min(width) - draw_x;
            if draw_w > 0.0 && body_h > 0.0 {
                instances.push(RectInstance {
                    x: draw_x, y: body_top, w: draw_w, h: body_h,
                    color: [bg.0, bg.1, bg.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Grid VERTICAL — subdivisões (mesmas regras de zoom do piano roll).
        for i in 0..total_bars {
            let bx = BPM_LABEL_W + i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }

            // 1/64
            if px_per_bar >= 960.0 {
                let n = 64u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs < BPM_LABEL_W || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 70],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/32
            if px_per_bar >= 480.0 {
                let n = 32u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs < BPM_LABEL_W || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 90],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/16
            if px_per_bar >= 240.0 {
                let n = 16u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs < BPM_LABEL_W || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/8
            if px_per_bar >= 120.0 {
                let n = 8u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs < BPM_LABEL_W || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Batidas (1/4) — mais grossas, por cima das subdivisões
            if px_per_bar >= 40.0 {
                let n = 4u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs < BPM_LABEL_W || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }

        // Linha vertical na borda direita do gutter de BPM.
        instances.push(RectInstance {
            x: BPM_LABEL_W,
            y: body_top,
            w: 1.0,
            h: body_h,
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        let _ = body_bottom;
        instances
    }

    /// Instâncias do gutter esquerdo com os valores de BPM (sem grid
    /// horizontal — só o fundo escuro e os números da faixa).
    pub fn build_bpm_gutter_instances(&self, height: f32) -> Vec<RectInstance> {
        use crate::build_text_instances;
        let t = self.theme.clamp(0, 6);
        let mut instances = Vec::new();
        let body_top = self.body_top;
        let body_h = self.body_h.max(0.0);

        // Fundo do gutter (coluna "BPM" onde o PR tem o teclado).
        instances.push(RectInstance {
            x: 0.0,
            y: body_top,
            w: BPM_LABEL_W,
            h: body_h,
            color: rgba(palette::BASE[t], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        // Labels de BPM a cada 40 (300/240/.../40/20..). Desenhados sobre o
        // corpo (sem linhas horizontais), em alturas onde cabem no gutter.
        let label_col = rgba(palette::TEXT_PRIMARY[t], 200);
        let mut bpm = BPM_MAX;
        while bpm >= BPM_MIN {
            let y = self.bpm_to_y(bpm);
            if y >= body_top - 8.0 && y <= body_top + body_h + 8.0 {
                instances.extend(build_text_instances(&format!("{bpm:.0}"), 2.0, y - 7.0, label_col, 1.0));
            }
            bpm -= 40.0;
        }

        // Borda direita do gutter.
        instances.push(RectInstance {
            x: BPM_LABEL_W - 1.0,
            y: body_top,
            w: 1.0,
            h: body_h,
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        let _ = height;
        instances
    }

    /// Instâncias do gutter esquerdo no MODO WARP: fundo escuro (no lugar dos
    /// valores de BPM, que não fazem sentido no editor de warp).
    pub fn build_warp_gutter_instances(&self) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6);
        let mut instances = Vec::new();
        let body_top = self.body_top;
        let body_h = self.body_h.max(0.0);

        instances.push(RectInstance {
            x: 0.0,
            y: body_top,
            w: BPM_LABEL_W,
            h: body_h,
            color: rgba(palette::BASE[t], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        // Borda direita do gutter.
        instances.push(RectInstance {
            x: BPM_LABEL_W - 1.0,
            y: body_top,
            w: 1.0,
            h: body_h,
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        instances
    }
}