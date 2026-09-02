//! Sistema de zoom/scroll da Timeline.
//!
//! Zoom HORIZONTAL — suave e responsivo SEM perder a âncora:
//!   - No início de um gesto (Ctrl+roda / Ctrl+arrasto 2 dedos) capturamos o
//!     ponto sob o cursor (`anchor_px`) UMA vez. Ele é fixo durante o gesto.
//!   - Acumulamos o delta (`dz_accum`) e derivamos um alvo de zoom contínuo
//!     exponencial (`base * exp(dz*gain)`).
//!   - Um Timer interpola o zoom exibido em direção ao alvo (amortecimento
//!     frame-independente).
//!   - O scroll de compensação é SEMPRE derivado do zoom exibido:
//!         sx = pad + anchor_px * zoom - mx
//!     Como `anchor_px` é constante, o ponto sob o cursor não se move NUNCA —
//!     não importa o amortecimento. Zero drift, zero "ímã".
//!
//! Scroll e zoom vertical continuam instantâneos (previsíveis e diretos).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;

use crate::MainWindow;

// Ganho de zoom: sensibilidade por pixel de delta.
// Um pouco maior que o antigo p/ o trackpad responder bem (responsivo),
// e o acumulador cuida de não dar saltos (suave).
const ZOOM_GAIN: f32 = 0.02;

// Velocidade de amortecimento do zoom (maior = mais rígido/responsivo).
const ZOOM_SPEED: f32 = 30.0;

// Faixas de zoom.
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 64.0;
const MIN_ZOOM_Y: f32 = 0.25;
const MAX_ZOOM_Y: f32 = 6.0;

// row-height = 64 * zoom_y (UI Slint).
const BASE_ROW_HEIGHT: f32 = 64.0;

// Ganho de scroll (reproduz o multiplicador do handle antigo).
const SCROLL_GAIN: f32 = 2.5;

/// Estado do gesto de zoom horizontal.
struct HZoom {
    active: bool,
    // Valores capturados no início do gesto (estado exibido daquele momento).
    base_zoom: f32,
    pad: f32,
    mx: f32,
    // Ponto (em "unid. de beat*40") sob o cursor — FIXO durante o gesto.
    anchor_px: f32,
    // Delta acumulado e alvo de zoom derivado.
    dz_accum: f32,
    zoom_target: f32,
    last: Instant,
}

impl Default for HZoom {
    fn default() -> Self {
        Self {
            active: false,
            base_zoom: 1.0,
            pad: 0.0,
            mx: 0.0,
            anchor_px: 0.0,
            dz_accum: 0.0,
            zoom_target: 0.0,
            last: Instant::now(),
        }
    }
}

fn smooth_alpha(speed: f32, dt: f32) -> f32 {
    1.0 - (-speed * dt).exp()
}

pub fn install(ui: &MainWindow) {
    let ui_weak = ui.as_weak();
    let h: Rc<RefCell<HZoom>> = Rc::new(RefCell::new(HZoom::default()));
    let timer: Rc<RefCell<slint::Timer>> = Rc::new(RefCell::new(slint::Timer::default()));

    ui.on_scroll_input(move |raw_dx, raw_dy, ctrl, shift, alt, mx, my| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        let dz = raw_dx + raw_dy;

        if ctrl && dz.abs() >= 0.05 {
            // -------------------------------------------------------------
            // ZOOM HORIZONTAL (Ctrl+roda / Ctrl+arrasto 2 dedos)
            // -------------------------------------------------------------
            let mut state = h.borrow_mut();

            if !state.active {
                // Início do gesto: fixa o ponto sob o cursor e o zoom base.
                let zoom = ui.get_zoom_level();
                let sx = ui.get_timeline_scroll_x();
                let pad = ui.get_timeline_left_padding();
                state.base_zoom = zoom;
                state.pad = pad;
                state.mx = mx;
                state.anchor_px = (mx + sx - pad).max(0.0) / zoom.max(0.01);
                state.dz_accum = 0.0;
                state.zoom_target = zoom;
                state.active = true;
                state.last = Instant::now();
            }

            // Acumula o delta mantendo a direção (dx+dy do gesto).
            state.dz_accum += dz;
            state.zoom_target = (state.base_zoom * (state.dz_accum * ZOOM_GAIN).exp())
                .clamp(MIN_ZOOM, MAX_ZOOM);

            // Liga o timer para interpolar (amortecimento contínuo).
            if !timer.borrow().running() {
                state.last = Instant::now();
                timer
                    .borrow()
                    .start(slint::TimerMode::Repeated, Duration::from_millis(8), {
                        let h = h.clone();
                        let timer = timer.clone();
                        let ui_weak = ui_weak.clone();
                        move || {
                            let Some(ui) = ui_weak.upgrade() else {
                                return;
                            };
                            tick_h(&ui, &h, &timer);
                        }
                    });
            }
        } else if alt && dz.abs() >= 0.05 {
            // -------------------------------------------------------------
            // ZOOM VERTICAL (Alt+roda / Alt+arrasto 2 dedos) — instantâneo.
            // -------------------------------------------------------------
            let sy = ui.get_timeline_scroll_y();
            let zy = ui.get_zoom_y_level();
            let ruler = ui.get_timeline_ruler_height();

            let new_zy = (zy * (dz * ZOOM_GAIN).exp()).clamp(MIN_ZOOM_Y, MAX_ZOOM_Y);

            let row = (my + sy - ruler).max(0.0) / (BASE_ROW_HEIGHT * zy.max(0.01));
            let t = row.floor().max(0.0);
            let new_sy = (sy + t * BASE_ROW_HEIGHT * (new_zy - zy))
                .clamp(0.0, ui.get_timeline_max_scroll_y().max(0.0));

            ui.set_zoom_y_level(new_zy);
            ui.set_timeline_scroll_y(new_sy);
        } else if shift {
            // SCROLL HORIZONTAL (Shift+roda) — instantâneo.
            let sx = ui.get_timeline_scroll_x();
            let max_sx = ui.get_timeline_max_scroll_x().max(0.0);
            ui.set_timeline_scroll_x((sx - raw_dy * SCROLL_GAIN).clamp(0.0, max_sx));
        } else {
            // SCROLL NORMAL (duas dimensões) — instantâneo.
            let sx = ui.get_timeline_scroll_x();
            let sy = ui.get_timeline_scroll_y();
            let max_sx = ui.get_timeline_max_scroll_x().max(0.0);
            let max_sy = ui.get_timeline_max_scroll_y().max(0.0);
            ui.set_timeline_scroll_x((sx - raw_dx * SCROLL_GAIN).clamp(0.0, max_sx));
            ui.set_timeline_scroll_y((sy - raw_dy * SCROLL_GAIN).clamp(0.0, max_sy));
        }
    });
}

/// Um tick do amortecimento do zoom horizontal.
fn tick_h(ui: &MainWindow, h: &Rc<RefCell<HZoom>>, timer: &Rc<RefCell<slint::Timer>>) {
    let mut state = h.borrow_mut();
    if !state.active {
        timer.borrow().stop();
        return;
    }

    let now = Instant::now();
    let dt = (now - state.last).as_secs_f32().min(0.05);
    state.last = now;

    let cur_zoom = ui.get_zoom_level();
    let a = smooth_alpha(ZOOM_SPEED, dt);
    // Interpola o zoom exibido em direção ao alvo (amortecido e suave).
    let new_zoom = cur_zoom + (state.zoom_target - cur_zoom) * a;

    // Scroll de compensação SEMPRE derivado do zoom exibido + âncora fixa.
    // anchor_px é constante → o ponto sob o cursor não se move.
    let max_sx = ui.get_timeline_max_scroll_x().max(0.0);
    let new_sx = (state.pad + state.anchor_px * new_zoom - state.mx).clamp(0.0, max_sx);

    ui.set_zoom_level(new_zoom);
    ui.set_timeline_scroll_x(new_sx);

    // Terminou quando chega perto do alvo.
    if (new_zoom - state.zoom_target).abs() < 0.001 {
        state.active = false;
        timer.borrow().stop();
    }
}
