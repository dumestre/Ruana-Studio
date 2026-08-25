//! WaveformLab — laboratório de renderização de waveform.
//!
//! Janela mínima com zoom, arrasto e A/B entre mono (downmix)
//! e estéreo (L/R em lanes). Carrega os WAVs da pasta `assets/`
//! além dos sinais sintéticos.

mod assets;
mod render;
mod synth;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

slint::include_modules!();

fn to_image(rgba: Vec<u8>, w: u32, h: u32) -> Image {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h);
    Image::from_rgba8(buf)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Sinais: sintéticos + WAVs da pasta assets/.
    let mut signals = synth::all_signals();
    let assets_dir = PathBuf::from("waveformlab/assets");
    let wav_signals = assets::load_assets(&assets_dir);
    let mut labels: Vec<slint::SharedString> =
        signals.iter().map(|s| s.label_name.clone().into()).collect();
    for w in &wav_signals {
        labels.push(w.label_name.clone().into());
    }
    signals.extend(wav_signals);
    let signals = Rc::new(signals);
    ui.set_signal_labels(Rc::new(slint::VecModel::from(labels)).into());

    if let Some(first) = signals.first() {
        ui.set_duration(first.duration());
    }

    // Estado do viewport: (px por segundo, início visível em segundos).
    let state: Rc<RefCell<(f32, f32)>> = Rc::new(RefCell::new((100.0, 0.0)));
    let drag_anchor: Rc<RefCell<Option<(f32, f32)>>> = Rc::new(RefCell::new(None));

    // Re-render completo das duas imagens.
    let redraw = {
        let ui_weak = ui.as_weak();
        let signals = signals.clone();
        let state = state.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let idx = ui.get_signal_index() as usize;
            let sig = &signals[idx.min(signals.len() - 1)];
            let (pps, view_start) = *state.borrow();

            let w = 1200u32.max(ui.window().size().width);
            let total = sig.left.len();
            let frac = (view_start / sig.duration()).clamp(0.0, 1.0);
            let start_sample = (frac * total as f32) as usize;
            let visible = ((w as f32 / pps) * sig.sample_rate) as usize;
            let end_sample = (start_sample + visible).min(total);
            let view = (start_sample.min(total.saturating_sub(1)), end_sample);

            // Modo automático: com menos de ~2 samples por pixel,
            // o envelope esconde os ciclos reais — troca para
            // osciloscópio (linha sample a sample).
            let samples_per_px = (view.1 - view.0) as f64 / w as f64;
            let scope = samples_per_px < 2.0;

            if scope {
                // Mesma cor do envelope: waveform sempre escura.
                let dark = [0x0A, 0x0A, 0x0A, 255];
                if let Some(rgba) =
                    render::render_oscilloscope(&sig.left, w, view, dark)
                {
                    ui.set_wave_mono(to_image(rgba, w, 512));
                }
                if let Some(rgba) =
                    render::render_oscilloscope(&sig.right, w, view, dark)
                {
                    ui.set_wave_stereo(to_image(rgba, w, 512));
                }
            } else {
                if let Some(rgba) = render::render_mono(&sig.left, &sig.right, w, view) {
                    ui.set_wave_mono(to_image(rgba, w, 512));
                }
                if let Some(rgba) = render::render_stereo(&sig.left, &sig.right, w, view) {
                    ui.set_wave_stereo(to_image(rgba, w, 512));
                }
            }
            ui.set_view_start(view_start);
        }
    };

    redraw();

    // Troca de sinal → reseta viewport e redesenha.
    {
        let state = state.clone();
        let redraw = redraw.clone();
        ui.on_signal_changed(move |_| {
            state.borrow_mut().1 = 0.0;
            redraw();
        });
    }

    // Zoom.
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        let redraw = redraw.clone();
        ui.on_zoom_changed(move |pps| {
            state.borrow_mut().0 = pps;
            let dur = ui_weak
                .upgrade()
                .map(|u| u.get_duration())
                .unwrap_or(4.0);
            let mut st = state.borrow_mut();
            let visible_s = 1200.0 / st.0;
            st.1 = st.1.min((dur - visible_s).max(0.0));
            drop(st);
            redraw();
        });
    }

    // Arrasto do viewport (incremental: cada move re-ancora).
    {
        let state = state.clone();
        let drag = drag_anchor.clone();
        ui.on_drag_start(move |x| {
            let pps = state.borrow().0;
            *drag.borrow_mut() = Some((x / 1.0, pps));
        });
    }
    {
        let state = state.clone();
        let drag = drag_anchor.clone();
        let ui_weak = ui.as_weak();
        let redraw = redraw.clone();
        ui.on_drag_move(move |x| {
            let anchor = *drag.borrow();
            let Some((ax, pps)) = anchor else { return };
            let dur = ui_weak
                .upgrade()
                .map(|u| u.get_duration())
                .unwrap_or(4.0);
            let cur_x = x / 1.0;
            let dx_seconds = (cur_x - ax) / pps;
            let mut st = state.borrow_mut();
            st.1 = (st.1 - dx_seconds).clamp(0.0, (dur - 1200.0 / st.0).max(0.0));
            drop(st);
            *drag.borrow_mut() = Some((cur_x, pps));
            redraw();
        });
    }
    {
        let drag = drag_anchor.clone();
        ui.on_drag_end(move || {
            *drag.borrow_mut() = None;
        });
    }

    ui.run()
}
