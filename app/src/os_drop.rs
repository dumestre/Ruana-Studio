//! Drag & drop de arquivos direto do Sistema Operacional.
use crate::MainWindow;
use slint::ComponentHandle;

use std::cell::RefCell;
use std::rc::Rc;

use crate::importer::{import_audio_file, is_audio_path};
use crate::waveform::WaveformCache;
use daw_core::project::Project;
use crate::ClipData;

pub fn install(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    track_colors_model: Rc<slint::VecModel<i32>>,
) {
        // Suporte a arrastar e soltar arquivos do Sistema Operacional (OS Drag & Drop)
        {
            use i_slint_backend_winit::WinitWindowAccessor;
            let ui_weak = ui.as_weak();
            let project = project.clone();
            let waveforms = waveforms.clone();
            let clips_model = clips_model.clone();
            let track_colors_model = track_colors_model.clone();

            ui.window().on_winit_window_event(move |_win, event| {
                use i_slint_backend_winit::winit::event::WindowEvent;
                match event {
                    WindowEvent::DroppedFile(path) => {
                        if let Some(ui) = ui_weak.upgrade() {
                            if is_audio_path(&path) {
                                // Pergunta ao sistema onde o cursor ESTÁ
                                // agora. Durante um drag OLE do Windows os
                                // eventos CursorMoved não chegam, então a
                                // última posição rastreada ficava velha e o
                                // clip caía em outra track.
                                #[cfg(windows)]
                                let (phys_x, phys_y) = {
                                    use windows::Win32::Foundation::POINT;
                                    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
                                    let mut pt = POINT { x: 0, y: 0 };
                                    unsafe { let _ = GetCursorPos(&mut pt); }
                                    (pt.x as f64, pt.y as f64)
                                };
                                #[cfg(not(windows))]
                                let (phys_x, phys_y) = (0.0, 0.0);

                                // Coordenadas físicas de TELA → lógicas da
                                // janela: desconta a origem da janela antes
                                // do scale factor.
                                let scale_factor = ui.window().scale_factor();
                                let win_pos = ui.window().position();
                                let logical_x =
                                    ((phys_x - win_pos.x as f64) / scale_factor as f64) as f32;
                                let logical_y =
                                    ((phys_y - win_pos.y as f64) / scale_factor as f64) as f32;

                                let coords = ui.invoke_map_window_coords_to_timeline(logical_x, logical_y);

                                import_audio_file(
                                    &path,
                                    coords.track,
                                    coords.beat,
                                    ui.get_bpm() as f32,
                                    &project,
                                    &waveforms,
                                    &clips_model,
                                    &track_colors_model,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                i_slint_backend_winit::EventResult::Propagate
            });
        }
}
