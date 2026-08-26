//! Drag & drop de arquivos do Sistema Operacional (Explorer → DAW).
//!
//! Fluxo com feedback idêntico ao drag interno do ClipPool:
//! 1. `HoveredFile` (cursor com arquivo entra na janela) → ativa o
//!    mesmo overlay visual do drag interno E dispara a decodificação
//!    em background (thread) preenchendo o cache de samples.
//! 2. Durante o hover, o ghost/snap/preview de drop aparecem como se
//!    fosse um drag do ClipPool (`drag-active` etc.).
//! 3. `DroppedFile` → o sample já está em RAM; a inserção é imediata.
//! 4. `HoveredFileCancelled` → desliga o overlay e descarta o job.

use crate::clip_cache;
use crate::importer::{import_audio_file_with_cache, is_audio_path};
use crate::waveform::WaveformCache;
use crate::ClipData;
use crate::MainWindow;
use daw_core::project::Project;
use std::cell::RefCell;
use std::rc::Rc;

pub fn install(
    ui: &MainWindow,
    project: Rc<RefCell<Project>>,
    waveforms: Rc<RefCell<WaveformCache>>,
    clips_model: Rc<slint::VecModel<ClipData>>,
    track_colors_model: Rc<slint::VecModel<i32>>,
    sample_cache: Rc<RefCell<clip_cache::SampleCache>>,
) {
        // Suporte a arrastar e soltar arquivos do Sistema Operacional (OS Drag & Drop)
        {
            use i_slint_backend_winit::WinitWindowAccessor;
            let ui_weak = ui.as_weak();
            let project = project.clone();
            let waveforms = waveforms.clone();
            let clips_model = clips_model.clone();
            let track_colors_model = track_colors_model.clone();
            let sample_cache = sample_cache.clone();

            ui.window().on_winit_window_event(move |_win, event| {
                use i_slint_backend_winit::winit::event::WindowEvent;
                match event {
                    // Janela voltou a ter foco do SO → devolve o foco
                    // de teclado ao FocusScope global. Sem isso os
                    // atalhos (Delete, Ctrl+Z...) morriam após o
                    // usuário alt-tab ou interagir com outra janela.
                    WindowEvent::Focused(true) => {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.invoke_restore_keyboard_focus();
                        }
                    }

                    // Cursor com arquivo ENTROU na janela: mesmo sinal
                    // visual do drag do ClipPool + pré-decodificação.
                    //
                    // O decode roda numa thread separada porque pode
                    // levar centenas de ms (MP3 grande); a UI continua
                    // fluida mostrando o ghost. O resultado vai para o
                    // SampleCache via channel, lido sem bloqueio.
                    WindowEvent::HoveredFile(path) => {
                        if !is_audio_path(&path) {
                            return i_slint_backend_winit::EventResult::Propagate;
                        }
                        if let Some(ui) = ui_weak.upgrade() {
                            let name = path
                                .file_stem()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Audio")
                                .to_string();
                            let path_str = path.to_string_lossy().to_string();
                            // Feedback visual IDÊNTICO ao drag interno:
                            // ghost do clip + preview de drop na timeline.
                            ui.invoke_drag_begin(
                                name.into(),
                                path_str.clone().into(),
                                "".into(),
                                "".into(),
                                slint::Color::from_rgb_u32(0x00C853),
                                false,
                            );
                            // Pré-decode em background (cacheia).
                            clip_cache::spawn_prefetch(
                                &sample_cache,
                                &path,
                                ui.get_bpm() as f32,
                            );
                        }
                    }

                    // Cursor saiu da janela sem soltar: encerra o
                    // overlay exatamente como o fim de um drag interno.
                    WindowEvent::HoveredFileCancelled => {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.invoke_drag_end();
                        }
                    }

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

                                let coords =
                                    ui.invoke_map_window_coords_to_timeline(logical_x, logical_y);

                                import_audio_file_with_cache(
                                    &path,
                                    coords.track,
                                    coords.beat,
                                    ui.get_bpm() as f32,
                                    &project,
                                    &waveforms,
                                    &clips_model,
                                    &track_colors_model,
                                    &sample_cache,
                                );

                                // Fecha o overlay do drag (o drop
                                // consumiu o gesto visual).
                                ui.invoke_drag_end();
                            }
                        }
                    }
                    _ => {}
                }
                i_slint_backend_winit::EventResult::Propagate
            });
        }
}
