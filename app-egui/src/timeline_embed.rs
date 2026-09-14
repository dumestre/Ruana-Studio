//! RuanaStudio — Timeline Embedded (wgpu)
//!
//! Integração real: timeline-render roda no mesmo device wgpu do egui,
//! renderiza em textura offscreen (readback) e egui mostra via `egui::Image`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;

use daw_core::transport::Transport;
use timeline_render::{App as TimelineApp, WindowCommand};

// ============================================================================
// TIMELINE EMBED STATE
// ============================================================================

/// Estado da timeline embarcada
pub struct TimelineEmbed {
    /// App da timeline (engine pura, sem janela)
    timeline: Option<TimelineApp>,
    /// Device/Queue wgpu compartilhados
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// Handle egui para exibir a textura
    egui_texture: Option<egui::TextureHandle>,
    /// Tamanho atual
    size: (u32, u32),
    /// Último frame time
    last_frame: Instant,
    /// Canal para comandos UI -> timeline
    command_tx: std::sync::mpsc::Sender<WindowCommand>,
    /// Transport compartilhado
    transport: Arc<Mutex<Transport>>,
}

impl TimelineEmbed {
    /// Cria nova instância com device/queue compartilhados
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        transport: Arc<Mutex<Transport>>,
    ) -> Self {
        eprintln!("DEBUG: TimelineEmbed::new called");
        let (command_tx, _command_rx) = std::sync::mpsc::channel::<WindowCommand>();

        // Cria timeline app em modo "engine pura" (window = None)
        eprintln!("DEBUG: Calling TimelineApp::new");
        let timeline = TimelineApp::new(
            None, 
            (1280, 720), 
            Some(transport.clone()), 
            Some(device.clone()), 
            Some(queue.clone())
        )
            .expect("Failed to create timeline app");
        eprintln!("DEBUG: TimelineApp::new returned successfully");

        Self {
            timeline: Some(timeline),
            device,
            queue,
            egui_texture: None,
            size: (1280, 720),
            last_frame: Instant::now(),
            command_tx,
            transport,
        }
    }

    /// Renderiza via readback e atualiza textura egui
    pub fn render_readback(&mut self, width: u32, height: u32, ctx: &egui::Context) {
        let new_size = (width.max(1), height.max(1));
        if self.size != new_size {
            self.size = new_size;
            self.egui_texture = None; // Recria no próximo frame
        }
        
        let Some(timeline) = self.timeline.as_mut() else { return };
        
        // Atualiza playhead
        if let Ok(transport) = self.transport.try_lock() {
            timeline.update_playhead_from_transport(&transport);
        }
        
        // Renderiza e lê de volta (bloqueante, mas simples)
        if let Some(rgba) = timeline.render_to_rgba(width, height) {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &rgba,
            );
            
            self.egui_texture = Some(ctx.load_texture(
                "timeline-texture",
                color_image,
                egui::TextureOptions::LINEAR,
            ));
        }
    }

    /// Mostra a timeline no egui
    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let available = ui.available_size();
        let width = available.x.max(320.0) as u32;
        let height = available.y.max(180.0) as u32;
        
        // Renderiza (readback por enquanto)
        self.render_readback(width, height, ctx);
        
        // Mostra textura
        if let Some(texture) = &self.egui_texture {
            let rect = ui.available_rect_before_wrap();
            let response = ui.image(texture);
            // Fit to available rect
            if response.rect != rect {
                // Image will scale to fit rect automatically
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Inicializando timeline...");
            });
        }
        
        ctx.request_repaint();
    }

    /// Envia comando para timeline
    pub fn send_command(&self, cmd: WindowCommand) {
        let _ = self.command_tx.send(cmd);
    }
}

// ============================================================================
// EXTENSION TRAIT PARA TIMELINE APP
// ============================================================================

/// Estende TimelineApp com método para atualizar playhead do transport compartilhado
trait TimelineAppExt {
    fn update_playhead_from_transport(&mut self, transport: &Transport);
}

impl TimelineAppExt for TimelineApp {
    fn update_playhead_from_transport(&mut self, transport: &Transport) {
        // Sincroniza estado de play/position
        self.playing = transport.playing;
        self.transport_seconds = transport.position_ticks as f32 / (transport.ppq as f32 * transport.bpm as f32 / 60.0);
        self.bpm = transport.bpm as f32;
        self.loop_enabled = transport.loop_enabled;
    }
}