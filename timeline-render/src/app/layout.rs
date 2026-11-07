//! Layout: posiciona as áreas da janela.
//!
//! Quando o piano roll (`show_piano_roll`) OU o timewarp (`show_timewarp`)
//! está visível (áreas EXPANDIDAS e EXCLUSIVAS), o bloco inferior expande de
//! baixo, **empurrando a timeline e a time_ruler para cima**. A splitter
//! separa a timeline content do bloco inferior. A edit toolbar (ferramentas)
//! fica numa faixa entre a splitter e a ruler da área.
//!
//! Layout com área expandida (de cima para baixo):
//!   ruler → timeline → time_ruler → SPLITTER → toolbar → ruler → conteúdo
//!
//! Os campos `piano_*` se referem ao piano roll; `timewarp_*` ao timewarp.
//! A altura do conteúdo (`self.piano_roll_height`) é compartilhada — a splitter
//! arrasta o bloco inferior independente de qual área está aberta.

use crate::app::edit_toolbar::TOOLBAR_H;
use crate::App;
use crate::app::piano_roll::PIANO_RULER_H;
use crate::app::timewarp::TIMEWARP_RULER_H;

#[derive(Clone, Debug)]
pub struct Layout {
    pub ruler: Rect,
    pub timeline: Rect,
    pub splitter: Rect,
    /// Faixa da edit toolbar (ferramentas) — Some apenas com área expandida.
    pub edit_toolbar: Option<Rect>,
    pub piano_ruler: Option<Rect>,
    pub piano_roll: Option<Rect>,
    pub timewarp_ruler: Option<Rect>,
    pub timewarp: Option<Rect>,
    pub time_ruler: Rect,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    pub fn contains_y(&self, py: f32) -> bool {
        py >= self.y && py <= self.y + self.height
    }
}

impl App {
    pub fn layout(&self, width: f32, height: f32) -> Layout {
        let ruler_h = self.ruler_px;
        let tr_h = self.time_ruler_h;
        let _splitter_h = 4.0;

        if !self.show_piano_roll && !self.show_timewarp {
            // Sem área expandida: timeline ocupa tudo entre ruler e time_ruler.
            let tr_y = height - tr_h;
            let timeline = Rect {
                x: 0.0, y: ruler_h, width,
                height: (tr_y - ruler_h).max(0.0),
            };
            return Layout {
                ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
                timeline,
                splitter: Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
                edit_toolbar: None,
                piano_ruler: None,
                piano_roll: None,
                timewarp_ruler: None,
                timewarp: None,
                time_ruler: Rect { x: 0.0, y: tr_y, width, height: tr_h },
            };
        }

        // Com área expandida (piano roll OU timewarp):
        // y=0 ruler → timeline → time_ruler → toolbar → SPLITTER → ruler → conteúdo → height

        let bottom_ruler_h = if self.show_timewarp { TIMEWARP_RULER_H } else { PIANO_RULER_H };
        let toolbar_h = TOOLBAR_H;

        // Altura total do bloco inferior (régua + conteúdo) — compartilhada.
        let bottom_total = (self.piano_roll_height + bottom_ruler_h)
            .clamp(
                self.piano_roll_min_h + bottom_ruler_h,
                (height - ruler_h - toolbar_h - 4.0)
                    .max(self.piano_roll_min_h + bottom_ruler_h),
            );

        // A splitter fica ACIMA da toolbar (que por sua vez fica acima da
        // régua da área). time_ruler é empurrada para cima pelo bloco.
        let tr_y = height - tr_h - toolbar_h - bottom_total - 4.0;

        let timeline = Rect {
            x: 0.0, y: ruler_h, width,
            height: (tr_y - ruler_h).max(0.0),
        };

        let time_ruler = Rect { x: 0.0, y: tr_y, width, height: tr_h };

        let splitter = Rect { x: 0.0, y: tr_y + tr_h, width, height: 4.0 };

        let edit_toolbar = Rect { x: 0.0, y: tr_y + tr_h + 4.0, width, height: toolbar_h };

        let area_ruler_y = tr_y + tr_h + 4.0 + toolbar_h;
        let area_body_y = area_ruler_y + bottom_ruler_h;

        let ruler_rect = Rect { x: 0.0, y: area_ruler_y, width, height: bottom_ruler_h };
        let body_rect = Rect { x: 0.0, y: area_body_y, width, height: (height - area_body_y).max(0.0) };

        let (piano_ruler, piano_roll, timewarp_ruler, timewarp) = if self.show_timewarp {
            (None, None, Some(ruler_rect), Some(body_rect))
        } else {
            (Some(ruler_rect), Some(body_rect), None, None)
        };

        Layout {
            ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
            timeline,
            time_ruler,
            edit_toolbar: Some(edit_toolbar),
            splitter,
            piano_ruler,
            piano_roll,
            timewarp_ruler,
            timewarp,
        }
    }

    pub fn splitter_hit_test(&self, cursor_y: f32, height: f32) -> bool {
        let layout = self.layout(self.size.0 as f32, height);
        layout.splitter.contains_y(cursor_y)
    }
}