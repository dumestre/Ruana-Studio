//! Tipos e geometria do piano roll: notas, métricas e helpers de conversão.
//! Dados sincronizados do `daw_midi::NoteEditor` (fonte de verdade compartilhada
//! com o Slint UI).

use crate::clips::ClipMetrics;
use crate::{palette, rgba, RectInstance};

/// Uma nota exibida no piano roll. Espelha `daw_midi::Note` para desacoplar
/// o estado de render da fonte de verdade (`NoteEditor`).
#[derive(Clone, Debug)]
pub struct PianoRollNote {
    pub id: i32,
    pub pitch: i32,
    pub start: f32,
    pub length: f32,
    pub velocity: i32,
    pub selected: bool,
}

/// Uma tecla do piano (vertical) — representa uma nota MIDI (0..127).
#[derive(Clone, Debug)]
pub struct PianoKey {
    pub pitch: i32,
    pub is_white: bool,
    pub label: String,
}

/// Conversões cursor ↔ conteúdo do piano roll, espelhando o clip selecionado.
#[derive(Clone)]
pub struct PianoRollMetrics {
    pub px_per_beat: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub ruler_px: f32,
    pub row_height: f32,
    pub theme: usize,
    /// Pitch mais alto visível (topo da viewport).
    pub top_pitch: i32,
    /// Pitch mais baixo visível (base da viewport).
    pub bottom_pitch: i32,
    /// Altura de cada tecla em pixels.
    pub key_height: f32,
    /// Altura do piano roll (área inferior) em pixels.
    pub roll_height: f32,
}

impl PianoRollMetrics {
    /// Cria métricas espelhando o `ClipMetrics` do clip MIDI selecionado.
    pub fn from_clip_metrics(cm: &ClipMetrics, roll_height: f32) -> Self {
        PianoRollMetrics {
            px_per_beat: cm.px_per_beat,
            scroll_x: cm.scroll_x,
            scroll_y: cm.scroll_y,
            ruler_px: cm.ruler_px,
            row_height: cm.row_height,
            theme: cm.theme,
            top_pitch: 96,
            bottom_pitch: 0,
            key_height: 6.0,
            roll_height,
        }
    }

    /// Converte posição do cursor em coordenadas de janela para (beat, pitch).
    pub fn cursor_to_beat_pitch(&self, cursor: (f32, f32)) -> (f32, i32) {
        let content_x = cursor.0 + self.scroll_x;
        let beat = if self.px_per_beat > 0.0 {
            content_x / self.px_per_beat
        } else {
            0.0
        };

        let piano_top = self.ruler_px;
        let rel_y = cursor.1 - piano_top + self.scroll_y;
        let pitch = if self.key_height > 0.0 {
            (self.top_pitch as f32 - rel_y / self.key_height)
                .round()
                .clamp(self.bottom_pitch as f32, self.top_pitch as f32) as i32
        } else {
            self.top_pitch
        };

        (beat, pitch)
    }

    /// Converte (beat, pitch) para coordenadas de janela (x, y).
    pub fn beat_pitch_to_xy(&self, beat: f32, pitch: i32) -> (f32, f32) {
        let x = beat * self.px_per_beat - self.scroll_x;
        let piano_top = self.ruler_px;
        let rel_y = (self.top_pitch - pitch) as f32 * self.key_height;
        let y = piano_top + rel_y - self.scroll_y;
        (x, y)
    }

    /// Converte um beat em coordenada x de janela.
    pub fn beat_to_x(&self, beat: f32) -> f32 {
        beat * self.px_per_beat - self.scroll_x
    }

    /// Retorna as teclas do piano para o range visível.
    pub fn piano_keys(&self) -> Vec<PianoKey> {
        const BLACK_INDICES: [i32; 5] = [1, 3, 6, 8, 11];
        const NOTE_NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];

        let mut keys = Vec::new();
        for pitch in (self.bottom_pitch..=self.top_pitch).rev() {
            let note_in_octave = pitch.rem_euclid(12);
            let is_black = BLACK_INDICES.contains(&note_in_octave);
            let label = format!("{}{}", NOTE_NAMES[note_in_octave as usize], pitch / 12);
            keys.push(PianoKey {
                pitch,
                is_white: !is_black,
                label,
            });
        }
        keys
    }

    /// Retorna as instâncias de fundo da grade (grid horizontal + vertical).
    /// Usa as MESMAS cores da timeline (SURFACE_HIGH/SURFACE_STEP alternados,
    /// mesmas opacidades de subdivisão).
    pub fn build_grid_instances(&self, width: f32, _height: f32) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6);
        let mut instances = Vec::new();

        let piano_top = self.ruler_px;
        let roll_bottom = piano_top + self.roll_height;
        let body_h = (roll_bottom - piano_top).max(0.0);

        // Fundo base do piano roll
        instances.push(RectInstance {
            x: 0.0,
            y: piano_top,
            w: width,
            h: body_h,
            color: rgba(palette::TIMELINE_BG[t], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        // Fundos alternados dos compassos (mesmo padrão da timeline)
        let px_per_bar = self.px_per_beat * 4.0;
        let total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let is_odd = i % 2 == 1;
            let bg = if is_odd { palette::SURFACE_HIGH[t] } else { palette::SURFACE_STEP[t] };
            let bw = px_per_bar;
            let bh = body_h;
            if bw > 0.0 && bh > 0.0 {
                instances.push(RectInstance {
                    x: bx, y: piano_top, w: bw, h: bh,
                    color: [bg.0, bg.1, bg.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Grid vertical — subdivisões (mesma opacidade da timeline)
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }

            // 1/64
            if px_per_bar >= 960.0 {
                let n = 64u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: piano_top, w: 1.0, h: body_h,
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
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: piano_top, w: 1.0, h: body_h,
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
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: piano_top, w: 1.0, h: body_h,
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
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: piano_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0,                         clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Batidas (1/4) — mais grossas, por cima das subdivisões
            if px_per_bar >= 40.0 {
                let n = 4u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: piano_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }

        // Grid horizontal (teclas do piano)
        for pitch in (self.bottom_pitch..=self.top_pitch).rev() {
            let y = piano_top + (self.top_pitch - pitch) as f32 * self.key_height - self.scroll_y;
            if y < piano_top || y > roll_bottom {
                continue;
            }
            let col = palette::GRID_LINE[t];
            instances.push(RectInstance {
                x: 0.0,
                y,
                w: width,
                h: 1.0,
                color: [col.0, col.1, col.2, 80],
                wave_offset: 0,
                wave_count: 0,
                clip_w: 0.0,
                clip_h: 0.0,
                radius: 0.0,
            });
        }

        instances
    }

    /// Retorna as instâncias do teclado vertical (à esquerda).
    pub fn build_keyboard_instances(
        &self,
        _width: f32,
        _height: f32,
    ) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6);
        let mut instances = Vec::new();

        let piano_top = self.ruler_px;
        let roll_bottom = piano_top + self.roll_height;
        let body_h = (roll_bottom - piano_top).max(0.0);
        let kb_w = 60.0;

        // Fundo do teclado
        instances.push(RectInstance {
            x: 0.0,
            y: piano_top,
            w: kb_w,
            h: body_h,
            color: rgba(palette::BASE[t], 255),
            wave_offset: 0,
            wave_count: 0,
            clip_w: 0.0,
            clip_h: 0.0,
            radius: 0.0,
        });

        // Teclas brancas e pretas
        for pitch in (self.bottom_pitch..=self.top_pitch).rev() {
            let y = piano_top + (self.top_pitch - pitch) as f32 * self.key_height - self.scroll_y;
            if y < piano_top || y > roll_bottom {
                continue;
            }

            let is_black = [1, 3, 6, 8, 11].contains(&(pitch.rem_euclid(12)));
            let key_h = self.key_height;
            let note_in_octave = pitch.rem_euclid(12);
            let is_alt = matches!(note_in_octave, 0 | 2 | 4 | 5 | 7 | 9 | 11);

            if is_black {
                let kb_x = kb_w - 14.0;
                let kb_w = 14.0;
                instances.push(RectInstance {
                    x: kb_x,
                    y,
                    w: kb_w,
                    h: key_h,
                    color: rgba(palette::BASE[t], 200),
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });
            } else {
                instances.push(RectInstance {
                    x: 0.0,
                    y,
                    w: kb_w,
                    h: key_h,
                    color: rgba(if is_alt { palette::SURFACE_HIGH[t] } else { palette::SURFACE_STEP[t] }, 255),
                    wave_offset: 0,
                    wave_count: 0,
                    clip_w: 0.0,
                    clip_h: 0.0,
                    radius: 0.0,
                });
            }
        }

        // Borda do teclado
        instances.push(RectInstance {
            x: kb_w - 1.0,
            y: piano_top,
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

