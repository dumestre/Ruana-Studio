//! Layout: posiciona as áreas da janela — timeline em cima, piano roll
//! embaixo, com a **splitter entre a time_ruler e a régua do piano roll**.
//!
//! Layout final (de cima para baixo):
//!   ruler → timeline → time_ruler → SPLITTER → piano_ruler → piano_roll

use crate::App;
use crate::app::piano_roll::PIANO_RULER_H;

#[derive(Clone, Debug)]
pub struct Layout {
    pub ruler: Rect,
    pub timeline: Rect,
    pub time_ruler: Rect,
    pub splitter: Rect,
    pub piano_ruler: Option<Rect>,
    pub piano_roll: Option<Rect>,
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
        let splitter_h = 4.0;

        if !self.show_piano_roll {
            let time_ruler_y = height - tr_h;
            let timeline = Rect {
                x: 0.0, y: ruler_h, width,
                height: (time_ruler_y - ruler_h).max(0.0),
            };
            return Layout {
                ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
                timeline,
                time_ruler: Rect { x: 0.0, y: time_ruler_y, width, height: tr_h },
                splitter: Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
                piano_ruler: None,
                piano_roll: None,
            };
        }

        // Com piano roll:
        // y=0 ruler → timeline → time_ruler → SPLITTER → piano_ruler → piano_roll → height

        let pr_ruler_h = PIANO_RULER_H;

        // time_ruler fixo no bottom da janela (acima do piano roll)
        let time_ruler_y = height - tr_h;

        // Altura total do bloco do piano roll (régua + conteúdo)
        let pr_total = (self.piano_roll_height + pr_ruler_h)
            .clamp(
                self.piano_roll_min_h + pr_ruler_h,
                (time_ruler_y - ruler_h - splitter_h).max(self.piano_roll_min_h + pr_ruler_h),
            );

        // A splitter fica ENTRE a time_ruler e a régua do piano roll.
        let splitter_y = time_ruler_y - pr_total;
        let pr_ruler_y = splitter_y + splitter_h;
        let pr_roll_y = pr_ruler_y + pr_ruler_h;

        let timeline = Rect {
            x: 0.0, y: ruler_h, width,
            height: (time_ruler_y - ruler_h).max(0.0),
        };

        let time_ruler = Rect { x: 0.0, y: time_ruler_y, width, height: tr_h };

        let splitter = Rect { x: 0.0, y: splitter_y, width, height: splitter_h };

        let piano_ruler = Rect { x: 0.0, y: pr_ruler_y, width, height: pr_ruler_h };

        let piano_roll = Rect {
            x: 0.0, y: pr_roll_y, width,
            height: (height - pr_roll_y).max(0.0),
        };

        Layout {
            ruler: Rect { x: 0.0, y: 0.0, width, height: ruler_h },
            timeline,
            time_ruler,
            splitter,
            piano_ruler: Some(piano_ruler),
            piano_roll: Some(piano_roll),
        }
    }

    pub fn splitter_hit_test(&self, cursor_y: f32, height: f32) -> bool {
        let layout = self.layout(self.size.width as f32, height);
        layout.splitter.contains_y(cursor_y)
    }
}
