//! Motor de zoom/scroll da Timeline.
//!
//! Arquitetura (CQRS):
//!   UI (Slint)  — captura o input cru (roda/touchpad) e repassa via
//!                 callback `scroll-input(dx,dy,ctrl,shift,alt,mx,my)`.
//!   Este motor — decide: acumula TARGETS (zoom/scroll exponenciais,
//!                 âncora no cursor) e, num Timer por frame, faz o
//!                 SMOOTHING com `t = 1 - exp(-speed*dt)` e escreve de
//!                 volta nas propriedades visuais (zoom/zoom-y/scroll).
//!
//! Nenhuma alocação por frame, frame-independente, sem passos artificiais.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;

use crate::MainWindow;

// Quão rápido cada valor persegue o seu alvo (maior = mais rígido).
const SPEED: f32 = 18.0;

// Ganho de zoom: sensibilidade por pixel de delta (roda ~60px/notch).
const ZOOM_GAIN_H: f32 = 0.022;
const ZOOM_GAIN_V: f32 = 0.022;

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 64.0;
const MIN_ZOOM_Y: f32 = 0.25;
const MAX_ZOOM_Y: f32 = 6.0;

// Ganho de scroll (reproduz o multiplicador do handle antigo 2.5x).
// SEM piso: no trackpad os deltas são pequenos e contínuos — um piso
// amplificava a inércia residual do dedo (±30px por micro-delta) e a
// timeline continuava rolando sozinha em vez de parar onde o usuário
// a deixou. A roda de mouse (delta ~120/notch) não é afetada pelo piso.
const SCROLL_GAIN: f32 = 2.5;

// Quando tudo chega ao alvo, para o timer (evita tick eterno).
const EPS: f32 = 0.05;

struct State {
    // Targets (px em coordenadas de conteúdo/timeline).
    tz: f32,
    tzy: f32,
    tsx: f32,
    tsy: f32,
    // Âncora (px de janela sob o cursor) para o zoom horizontal.
    anchor_mx: f32,
    // Verdadeiro enquanto há alvo a perseguir; falso → ressync dos alvos.
    active: bool,
    // Instant do último tick (para o dt do smoothing).
    last: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tz: 0.0,
            tzy: 0.0,
            tsx: 0.0,
            tsy: 0.0,
            anchor_mx: 0.0,
            active: false,
            last: Instant::now(),
        }
    }
}

/// Passo de scroll por evento (mesma semântica do handle antigo: ganho,
/// mas o resultado vai ao ALVO e é suavizado — nada de salto no visual).
fn step(v: f32) -> f32 {
    v * SCROLL_GAIN
}

/// Coeficiente exponencial frame-independente: `1 - exp(-speed*dt)`.
fn smooth_alpha(speed: f32, dt: f32) -> f32 {
    1.0 - (-speed * dt).exp()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn install(ui: &MainWindow) {
    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::default()));
    let timer: Rc<RefCell<slint::Timer>> = Rc::new(RefCell::new(slint::Timer::default()));

    let ui_weak_input = ui.as_weak();
    let state_input = state.clone();
    let timer_input = timer.clone();
    ui.on_scroll_input(move |dx, dy, ctrl, shift, alt, mx, _my| {
        let Some(ui) = ui_weak_input.upgrade() else {
            return;
        };
        let mut s = state_input.borrow_mut();

        // Se o timer parou (tudo "assentou"), ressincroniza os alvos com
        // a view atual — o próximo gesto recomeça dali, sem pular.
        if !s.active {
            s.tz = ui.get_zoom_level();
            s.tzy = ui.get_zoom_y_level();
            s.tsx = ui.get_timeline_scroll_x();
            s.tsy = ui.get_timeline_scroll_y();
            s.last = Instant::now();
            s.active = true;
        }

        if ctrl {
            // -----------------------------------------------------------------
            // ZOOM HORIZONTAL (Ctrl+roda ou botão do meio segurado)
            // -----------------------------------------------------------------
            // Zoom EXPONENCIAL contínuo: fator = exp(gain * delta). Direção
            // preservada do app atual: delta positivo → afastar (zoom out).
            let dz = dx + dy;
            let factor = (-dz * ZOOM_GAIN_H).exp();
            let new_tz = (s.tz * factor).clamp(MIN_ZOOM, MAX_ZOOM);

            // ÂNCORA NO CURSOR: o ponto de conteúdo sob o cursor NÃO move.
            // Usa a view ATUAL (que o usuário vê) para achar o conteúdo,
            // e deriva o scroll-alvo para a nova zoom-alvo.
            let pad = ui.get_timeline_left_padding();
            let cur_zoom = ui.get_zoom_level();
            let cur_sx = ui.get_timeline_scroll_x();
            let content_x = (mx + cur_sx - pad).max(0.0) / cur_zoom;
            let new_tsx = pad + content_x * new_tz - mx;

            s.tz = new_tz;
            s.tsx = new_tsx;
            s.anchor_mx = mx;
        } else if alt {
            // -----------------------------------------------------------------
            // ZOOM VERTICAL (Alt+roda) — ancorado no topo (régua).
            // -----------------------------------------------------------------
            let dz = dx + dy;
            let factor = (-dz * ZOOM_GAIN_V).exp();
            s.tzy = (s.tzy * factor).clamp(MIN_ZOOM_Y, MAX_ZOOM_Y);
        } else if shift {
            // -----------------------------------------------------------------
            // SCROLL HORIZONTAL (Shift+roda)
            // -----------------------------------------------------------------
            s.tsx -= step(dy);
        } else {
            // -----------------------------------------------------------------
            // SCROLL NORMAL (duas dimensões)
            // -----------------------------------------------------------------
            s.tsx -= step(dx);
            s.tsy -= step(dy);
        }

        // Re-liga o timer se estiver parado e há movimento para perseguir.
        if !timer_input.borrow().running() {
            s.last = Instant::now();
            timer_input
                .borrow()
                .start(slint::TimerMode::Repeated, Duration::from_millis(8), {
                    let state = state_input.clone();
                    let timer = timer_input.clone();
                    let ui_weak = ui_weak_input.clone();
                    move || {
                        let Some(ui) = ui_weak.upgrade() else {
                            return;
                        };
                        let mut s = state.borrow_mut();
                        if !s.active {
                            timer.borrow().stop();
                            return;
                        }

                        let now = Instant::now();
                        let dt = (now - s.last).as_secs_f32().min(0.05);
                        s.last = now;

                        let cur_sx = ui.get_timeline_scroll_x();
                        let cur_sy = ui.get_timeline_scroll_y();
                        let cur_zoom = ui.get_zoom_level();
                        let cur_tzy = ui.get_zoom_y_level();

                        let a = smooth_alpha(SPEED, dt);
                        let nsx = lerp(cur_sx, s.tsx, a);
                        let nsy = lerp(cur_sy, s.tsy, a);
                        let nz = lerp(cur_zoom, s.tz, a);
                        let nzy = lerp(cur_tzy, s.tzy, a);

                        // Clamp com os máximos ATUAIS da view.
                        let max_sx = ui.get_timeline_max_scroll_x().max(0.0);
                        let max_sy = ui.get_timeline_max_scroll_y().max(0.0);
                        let nsx = nsx.clamp(0.0, max_sx);
                        let nsy = nsy.clamp(0.0, max_sy);

                        ui.set_timeline_scroll_x(nsx);
                        ui.set_timeline_scroll_y(nsy);
                        ui.set_zoom_level(nz);
                        ui.set_zoom_y_level(nzy);

                        // Parou de chegar perto do alvo? Desliga o timer.
                        if (nsx - s.tsx).abs() < EPS
                            && (nsy - s.tsy).abs() < EPS
                            && (nz - s.tz).abs() < EPS
                            && (nzy - s.tzy).abs() < EPS
                        {
                            s.active = false;
                            timer.borrow().stop();
                        }
                    }
                });
        }
    });
}
