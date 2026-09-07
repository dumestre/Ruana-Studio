//! Layout: posiciona as áreas da janela.
//!
//! Quando o piano roll está visível (`show_piano_roll`), ele expande de baixo,
//! **empurrando a timeline e a time_ruler para cima**. A splitter separa a
//! timeline content do piano roll.
//!
//! Layout com piano roll (de cima para baixo):
//!   ruler → timeline → SPLITTER → piano_ruler → piano_roll → time_ruler

use crate::App;
use crate::app::piano_roll::PIANO_RULER_H;

#[derive(Clone, Debug)]
pub struct Layout {
    pub ruler: Rect,
    pub timeline: Rect,
    pub splitter: Rect,
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
                x: 0.0, y: ruler_h, width,
                height: (tr_y - ruler_h).max(0.0),
            };
            return Layout {
                ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
                timeline,
                splitter: Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
                piano_ruler: None,
                piano_roll: None,
                time_ruler: Rect { x: 0.0, y: tr_y, width, height: tr_h },
            };
        }

        // Com piano roll:
        // y=0 ruler → timeline → time_ruler → SPLITTER → piano_ruler → piano_roll → height

        let pr_ruler_h = PIANO_RULER_H;

        // Altura total do bloco do piano roll (régua + conteúdo)
        let pr_total = (self.piano_roll_height + PIANO_RULER_H)
            .clamp(
                self.piano_roll_min_h + PIANO_RULER_H,
                (height - ruler_h - 4.0).max(self.piano_roll_min_h + PIANO_RULER_H),
            );

        // A splitter fica ENTRE a time_ruler e a régua do piano roll.
        // time_ruler é empurrada para cima pelo piano roll.
        let tr_y = height - tr_h - pr_total - 4.0;

        let timeline = Rect {
            x: 0.0, y: ruler_h, width,
            height: (tr_y - ruler_h).max(0.0),
        };

        let time_ruler = Rect { x: 0.0, y: tr_y, width, height: tr_h };

        let splitter = Rect { x: 0.0, y: tr_y + tr_h, width, height: 4.0 };

        let pr_ruler_y = tr_y + tr_h + 4.0;
        let pr_roll_y = pr_ruler_y + pr_ruler_h;

        Layout {
            ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
            timeline,
            time_ruler,
            splitter,
            piano_ruler: Some(Rect { x: 0.0, y: pr_ruler_y, width, height: pr_ruler_h }),
            piano_roll: Some(Rect { x: 0.0, y: pr_roll_y, width, height: (height - pr_roll_y).max(0.0) }),
        }
    }

    pub fn splitter_hit_test(&self, cursor_y: f32, height: f32) -> bool {
        let layout = self.layout(self.size.width as f32, height);
        layout.splitter.contains_y(cursor_y)
    }
}
