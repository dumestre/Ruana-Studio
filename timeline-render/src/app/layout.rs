//! Layout: posiciona as áreas da janela.
//!
//! Quando o piano roll está visível (`show_piano_roll`), ele expande de baixo,
//! **empurrando a timeline e a time_ruler para cima**. A splitter separa a
//! timeline content do bloco do piano roll. A edit toolbar (ferramentas) fica
//! numa faixa entre a splitter e a piano_ruler.
//!
//! Layout com piano roll (de cima para baixo):
//!   ruler → timeline → time_ruler → SPLITTER → toolbar → piano_ruler → piano_roll

use crate::App;
use crate::app::edit_toolbar::TOOLBAR_H;
use crate::app::piano_roll::PIANO_RULER_H;

#[derive(Clone, Debug)]
pub struct Layout {
    pub ruler: Rect,
    pub timeline: Rect,
    pub splitter: Rect,
    /// Faixa da edit toolbar (ferramentas) — Some apenas com piano roll.
    pub edit_toolbar: Option<Rect>,
    pub piano_ruler: Option<Rect>,
    pub piano_roll: Option<Rect>,
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
        let _pr_ruler_h = PIANO_RULER_H;

        if !self.show_piano_roll {
            // Sem piano roll: timeline ocupa tudo entre ruler e time_ruler.
            let tr_y = height - tr_h;
            let timeline = Rect {
                x: 0.0,
                y: ruler_h,
                width,
                height: (tr_y - ruler_h).max(0.0),
            };
            return Layout {
                ruler: Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: ruler_h,
                },
                timeline,
                splitter: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
                edit_toolbar: None,
                piano_ruler: None,
                piano_roll: None,
                time_ruler: Rect {
                    x: 0.0,
                    y: tr_y,
                    width,
                    height: tr_h,
                },
            };
        }

        // Com piano roll:
        // y=0 ruler → timeline → time_ruler → toolbar → SPLITTER → piano_ruler → piano_roll → height

        let pr_ruler_h = PIANO_RULER_H;
        let toolbar_h = TOOLBAR_H;

        // Altura total do bloco do piano roll (régua + conteúdo)
        let pr_total = (self.piano_roll_height + PIANO_RULER_H).clamp(
            self.piano_roll_min_h + PIANO_RULER_H,
            (height - ruler_h - toolbar_h - 4.0).max(self.piano_roll_min_h + PIANO_RULER_H),
        );

        // A splitter fica ACIMA da toolbar (que por sua vez fica acima da
        // régua do piano roll). time_ruler é empurrada para cima pelo bloco.
        let tr_y = height - tr_h - toolbar_h - pr_total - 4.0;

        let timeline = Rect {
            x: 0.0,
            y: ruler_h,
            width,
            height: (tr_y - ruler_h).max(0.0),
        };

        let time_ruler = Rect {
            x: 0.0,
            y: tr_y,
            width,
            height: tr_h,
        };

        let splitter = Rect {
            x: 0.0,
            y: tr_y + tr_h,
            width,
            height: 4.0,
        };

        let edit_toolbar = Rect {
            x: 0.0,
            y: tr_y + tr_h + 4.0,
            width,
            height: toolbar_h,
        };

        let pr_ruler_y = tr_y + tr_h + 4.0 + toolbar_h;
        let pr_roll_y = pr_ruler_y + pr_ruler_h;

        Layout {
            ruler: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: ruler_h,
            },
            timeline,
            time_ruler,
            edit_toolbar: Some(edit_toolbar),
            splitter,
            piano_ruler: Some(Rect {
                x: 0.0,
                y: pr_ruler_y,
                width,
                height: pr_ruler_h,
            }),
            piano_roll: Some(Rect {
                x: 0.0,
                y: pr_roll_y,
                width,
                height: (height - pr_roll_y).max(0.0),
            }),
        }
    }

    pub fn splitter_hit_test(&self, cursor_y: f32, height: f32) -> bool {
        let layout = self.layout(self.size.0 as f32, height);
        layout.splitter.contains_y(cursor_y)
    }
}
