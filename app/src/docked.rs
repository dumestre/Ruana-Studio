//! Timeline dockada (arquitetura do AGENTS.md, abordagem B).
//!
//! Janela principal do app = Slint nativo (skia/femtovg, GPU). A timeline
//! roda numa thread separada via `timeline_render::run_docked_window`:
//! uma janela winit borderless que rastreia a janela "Ruana Studio" e se
//! encaixa exatamente sobre o retângulo da timeline (margens = layout real do
//! `.slint`, lidas via out-props), desenhando direto na surface (sem readback).

use std::rc::Rc;
use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use daw_core::transport::Transport;
use daw_core::Project;
use slint::{Timer, ComponentHandle};
use timeline_render::DockSpec;
use timeline_render::WindowCommand;

use crate::MainWindow;
use crate::timeline_slint::build_clip_demos;

/// Sobe a thread da janela da timeline dockada após a primeira pintura.
///
/// Devolve o `Sender` para o app enviar comandos (ex.: tema) ao engine.
pub fn install(
    ui: &MainWindow,
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
) -> mpsc::Sender<WindowCommand> {
    let (tx, rx) = mpsc::channel::<WindowCommand>();

    // Estado inicial + re-espelho a cada mudança de documento.
    let push = |p: &Project, tx: &mpsc::Sender<WindowCommand>| {
        let _ = tx.send(WindowCommand::SetClips(build_clip_demos(p)));
        let _ = tx.send(WindowCommand::SetBpm(p.tempo_map.bpm_at(0.0)));
    };

    // Envia estado inicial (project já existe aqui).
    push(&project.borrow(), &tx);

    // Subscreve mudanças do documento para re-espelhar clips/bpm.
    {
        let project_rc = project.clone();
        let tx_mirror = tx.clone();
        project.borrow().subscribe(move |_event, _kind| {
            push(&project_rc.borrow(), &tx_mirror);
        });
    }

    // Agenda spawn da janela dockada após layout/máxima estabilizarem.
    // Usa Timer::single_shot (não precisa reter o handle — callback dispara mesmo se dropar).
    let ui_weak = ui.as_weak();
    let transport_thread = transport.clone();
    Timer::single_shot(Duration::from_millis(600), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Propriedades do layout (já em px lógicos, vindas do .slint).
        let abs_x = ui.get_timeline_canvas_abs_x();
        let abs_y = ui.get_timeline_canvas_abs_y();
        let rect_w = ui.get_timeline_canvas_rect_width();
        let rect_h = ui.get_timeline_canvas_rect_height();

        // Tamanho da janela principal (PhysicalSize) → converte p/ lógico.
        let phys = ui.window().size();
        let scale = ui.window().scale_factor().max(0.001);
        let client_w_logical = phys.width as f32 / scale;
        let client_h_logical = phys.height as f32 / scale;

        // Margens lógicas p/ o dock tracker (o tracker reconverte c/ DPI cada tick).
        let top = abs_y.max(0.0).round() as i32;
        let left = abs_x.max(0.0).round() as i32;
        let right = (client_w_logical - (abs_x + rect_w)).max(0.0).round() as i32;
        let bottom = (client_h_logical - (abs_y + rect_h)).max(0.0).round() as i32;

        let spec = DockSpec {
            parent_title: "Ruana Studio".to_string(),
            top,
            left,
            right,
            bottom,
        };

        eprintln!(
            "[timeline_dock] spawn spec top={} left={} right={} bottom={} client_logical={:.0}x{:.0} (phys={}x{} scale={:.2})",
            top, left, right, bottom, client_w_logical, client_h_logical, phys.width, phys.height, scale
        );

        let _ = std::thread::Builder::new()
            .name("timeline-docked".into())
            .spawn(move || {
                if let Err(e) = timeline_render::run_docked_window(transport_thread, rx, spec) {
                    eprintln!("[timeline_dock] erro ao subir janela dockada: {e}");
                }
            });
    });

    tx
}

/// Envia mudança de tema para a janela dockada.
pub fn push_theme(tx: &mpsc::Sender<WindowCommand>, idx: i32) {
    let _ = tx.send(WindowCommand::SetTheme(idx as usize));
}