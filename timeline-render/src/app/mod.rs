//! Módulo `app`: estado do aplicativo (struct `App`), construtor, métodos
//! base e o laço de eventos ((re)exporta `run`).

pub(crate) mod audio;
pub(crate) mod interaction;
pub(crate) mod render;
pub(crate) mod window;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

use wgpu::util::DeviceExt;
use daw_audio::AudioEngine;
use daw_core::Transport;
use winit::window::Window;

use crate::clips;
use crate::fades;
use crate::shctx::SHADER_WGSL;
use crate::tools::{init_tool_manager, set_dark_mode};
use crate::waveform;

// px-per-beat FIXO com que o buffer da waveform é gerado (zoom base).
// Qualquer mudança estrutural nos clips (slice/glue/erase/copy/paste) deve
// regenerar o buffer EM CONJUNTO com `wave_segments` — os dois andam juntos.
pub const WAVE_PPB: f32 = 40.0;

/// Snapshot do projeto para undo/redo: clips + fades (estado editável).
#[derive(Clone)]
pub struct TimelineSnapshot {
    pub clips: Vec<clips::ClipDemo>,
    pub fades: Vec<fades::ClipFade>,
}

/// Estrutura temporária para exibir o preview de um áudio arrastado antes da inserção.
pub struct DropPreview {
    pub beat: f32,
    pub track: usize,
    pub len_beats: f32,
}

/// Modo de arrasto do seletor de loop na régua.
#[derive(Clone, Copy, PartialEq)]
pub enum LoopDrag {
    Start, // arrasta a borda esquerda (ajusta largura)
    End,   // arrasta a borda direita (ajusta largura)
    Move,  // arrasta o centro (move o range inteiro)
}

/// Obtém a posição atual do cursor em coordenadas lógicas da janela, usando
/// `GetCursorPos` (Win32) porque durante o arrasto de um arquivo o sistema
/// operacional não envia eventos `CursorMoved` para a janela.
pub(crate) fn current_cursor_pos(window: &Window) -> Option<(f32, f32)> {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) != 0 {
            let window_pos = window.outer_position().ok()?;
            let scale = window.scale_factor();
            let x = (pt.x - window_pos.x) as f32 / scale as f32;
            let y = (pt.y - window_pos.y) as f32 / scale as f32;
            Some((x, y))
        } else {
            None
        }
    }
}

pub struct App {
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub unit_quad: wgpu::Buffer,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,

    pub theme: usize,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub px_per_beat: f32,
    pub zoom_y: f32,
    pub ruler_px: f32,
    pub playing: bool,
    pub transport_seconds: f32,
    pub play_start_seconds: f32,
    pub last_play_head_update: Instant,
    pub panning: bool,
    pub scrubbing: bool,
    pub last_cursor: (f32, f32),

    pub transport: Arc<Mutex<Transport>>,
    pub audio_engine: Option<AudioEngine>,

    pub wave_bars: Vec<f32>,
    /// (offset, count) fixos por id de clip no buffer `wave_bars`.
    pub wave_segments: Vec<(u32, u32)>,
    pub clips: Vec<clips::ClipDemo>,
    pub clip_interaction: clips::ClipInteraction,
    pub fades: Vec<fades::ClipFade>,
    pub fade_interaction: fades::FadeInteraction,

    pub loop_start_beat: f32,
    pub loop_end_beat: f32,
    pub loop_enabled: bool,
    pub loop_drag: Option<LoopDrag>,
    pub loop_drag_start_x: f32,
    pub loop_drag_orig_start: f32,
    pub loop_drag_orig_end: f32,

    pub focus_saved: Option<(f32, f32, f32)>,
    pub clipboard: Vec<clips::ClipDemo>,

    pub undo_stack: Vec<TimelineSnapshot>,
    pub redo_stack: Vec<TimelineSnapshot>,

    /// Paths dos arquivos em preview de drag & drop (acumulados; winit envia um
    /// `DroppedFile` por arquivo). Importados em lote após o fim do arrasto.
    pub drop_paths: Vec<String>,
    /// Aguarda período de silêncio (sem novos drops) antes de importar o lote.
    pub pending_drop_import: bool,
    /// Momento do último `DroppedFile` (para detectar fim do lote).
    pub last_drop_instant: Instant,

        // Auto-scroll nas bordas durante drag de clips.
        // Quando o cursor se aproxima de uma borda durante um arrasto ativo,
        // a viewport rola suavemente nessa direção para permitir arrastar
        // clips para fora da área visível.
        pub edge_scroll_x: f32, // velocidade px/frame (0 = desativado)
        pub edge_scroll_y: f32, // velocidade px/frame (0 = desativado)
        pub edge_scroll_dir: i32, // -1 = esquerda/cima, +1 = direita/baixo, 0 = nenhuma
        pub bpm: f32,
        pub time_ruler_h: f32,
        pub follow_playhead: bool,
        // ---------- estado para preview de drop de áudio ----------
        pub drop_path: Option<String>,            // caminho do arquivo arrastado
        pub drop_preview: Option<DropPreview>,   // preview visual enquanto ainda não inserido
        pub is_dragging_file: bool,              // true enquanto um arquivo está sendo arrastado sobre a janela
        // ------------------------------------------------------------
}

impl App {
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::empty(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: Default::default(),
            display: None,
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("wgpu: create_surface failed: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("wgpu: no adapter available: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ruana-timeline-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: Default::default(),
                experimental_features: Default::default(),
            },
        ))
        .map_err(|e| format!("wgpu: request_device failed: {e}"))?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("timeline-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("timeline-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("timeline-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let unit_quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("timeline-unit-quad"),
            contents: bytemuck::cast_slice(&[
                0.0f32, 0.0f32, 1.0f32, 0.0f32, 0.0f32, 1.0f32, 1.0f32, 1.0f32,
            ]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            view_formats: vec![],
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            desired_maximum_frame_latency: 1,
            color_space: Default::default(),
        };
        surface.configure(&device, &config);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("timeline-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 52,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 0,
                                shader_location: 1,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 16,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 32,
                                shader_location: 3,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 48,
                                shader_location: 4,
                            },
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // === Dados demo ===
        // Abre com apenas UMA track padrão, sem clips. O usuário importa o
        // áudio (por drag & drop ou menu) e a timeline cria as tracks.
        let mut clips: Vec<clips::ClipDemo> = Vec::new();

        // Gera as ondas dos clips demo e armazena cada uma em `clip.wave`
        // (a fonte da verdade). Buffer e segmentação derivam disso.
        let (wave_bars, wave_segments) = waveform::build_wave_bars_and_segments(&mut clips, WAVE_PPB);
        // Cria um vetor de fades padrão para esses clips (não‑midi).
        let fades = clips.iter().map(|c| {
            if !c.is_midi {
                fades::ClipFade::default()
            } else {
                fades::ClipFade::none()
            }
        }).collect::<Vec<_>>();

        let transport = Arc::new(Mutex::new(Transport::default()));
        let audio_engine = AudioEngine::start(transport.clone()).ok();

        Ok(Self {
            window,
            surface,
            device,
            queue,
            pipeline,
            bind_group_layout,
            unit_quad,
            config,
            size,

            theme: 6,
            scroll_x: 0.0,
            scroll_y: 0.0,
            px_per_beat: 40.0,
            zoom_y: 1.0,
            ruler_px: 28.0,
            playing: false,
            transport_seconds: 0.0,
            play_start_seconds: 0.0,
            last_play_head_update: Instant::now(),
            panning: false,
            scrubbing: false,
            last_cursor: (0.0, 0.0),

            transport,
            audio_engine,

            wave_bars,
            wave_segments,
            clips,
            clip_interaction: clips::ClipInteraction::new(),
            fades,
            fade_interaction: fades::FadeInteraction::new(),

            loop_start_beat: 0.0,
            loop_end_beat: 8.0,
            loop_enabled: true,
            loop_drag: None,
            loop_drag_start_x: 0.0,
            loop_drag_orig_start: 0.0,
            loop_drag_orig_end: 0.0,

            focus_saved: None,
            clipboard: Vec::new(),

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),

            drop_paths: Vec::new(),
            pending_drop_import: false,
            last_drop_instant: Instant::now(),

            edge_scroll_x: 0.0,
            edge_scroll_y: 0.0,
            edge_scroll_dir: 0,
            bpm: 120.0,
            time_ruler_h: 30.0,
            follow_playhead: false,
            drop_path: None,
            drop_preview: None,
            is_dragging_file: false,
        })
    }

    pub fn init_tools(&self) {
        let _ = init_tool_manager();
        let _ = set_dark_mode(self.theme == 6);
    }

    pub fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.size = size;
        if size.width > 0 && size.height > 0 {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
        self.clamp_scroll_y(size.height as f32);
    }

    pub fn set_playhead_from_x(&mut self, mx: f32) {
        let bpm = self.bpm;
        let pps = self.px_per_beat * bpm / 60.0;
        self.transport_seconds = ((mx + self.scroll_x) / pps).max(0.0);
    }

    /// Converte um beat (tempo musical) para a coordenada x de janela.
    pub fn beat_to_x(&self, beat: f32) -> f32 {
        beat * self.px_per_beat - self.scroll_x
    }

    pub fn clip_metrics(&self) -> clips::ClipMetrics {
        clips::ClipMetrics {
            px_per_beat: self.px_per_beat,
            wave_segments: self.wave_segments.clone(),
            row_height: 64.0 * self.zoom_y.max(0.001),
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
            ruler_px: self.ruler_px,
            theme: self.theme.clamp(0, 6) as usize,
            max_track: self.clips.iter().map(|c| c.track_index).max().unwrap_or(0),
        }
    }

    /// Trava a rolagem vertical para que todos os clips fiquem visíveis:
    /// se o conteúdo cabe na viewport, trava em 0 (grudado no topo); senão,
    /// limita o scroll ao máximo necessário para ver o último clip.
    pub fn clamp_scroll_y(&mut self, height: f32) {
        let row_height = 64.0 * self.zoom_y.max(0.001);
        let max_track = self.clips.iter().map(|c| c.track_index).max().unwrap_or(0);
        // Sempre mostra ao menos 1 track (uma track padrão visível).
        let content_h = (max_track as f32 + 1.0) * row_height;
        let view_h = (height - self.ruler_px - self.time_ruler_h).max(0.0);
        if content_h <= view_h {
            self.scroll_y = 0.0;
        } else {
            self.scroll_y = self.scroll_y.clamp(0.0, content_h - view_h);
        }
    }

    /// Reconcatena as ondas ARMAZENADAS nos clips (`clip.wave`) no buffer e
    /// recalcula a segmentação por id. Chamar SEMPRE que a estrutura mudar
    /// (slice, glue, erase, copy/paste, duplicate). NÃO regenera as ondas — os
    /// cortes feitos por slice são preservados nas peças.
    pub fn rebuild_wave_data(&mut self) {
        let (bars, segments) = waveform::concat_wave_data(&self.clips);
        self.wave_bars = waveform::apply_fade_envelope(bars, &segments, &self.clips, &self.fades);
        self.wave_segments = segments;
    }

    pub fn update_playhead(&mut self) {
        if !self.playing { return; }

        // Lê a posição do transporte compartilhado (avançado pelo AudioEngine).
        let transport_seconds = self.transport.lock()
            .map(|t| t.position_seconds() as f32)
            .unwrap_or(self.transport_seconds);

        self.transport_seconds = transport_seconds;
        self.last_play_head_update = Instant::now();

        // Follow playhead: mantém o playhead no centro da tela rolando a timeline.
        // Só ativa quando o playhead atinge o centro — antes disso, o usuário
        // pode posicionar o scroll manualmente para trás.
        if self.follow_playhead {
            let pps = self.px_per_beat * self.bpm / 60.0;
            if pps > 0.0 {
                let head_x = self.transport_seconds * pps - self.scroll_x;
                let center = self.size.width as f32 / 2.0;
                // Se o playhead passou do centro, rola a timeline para segui-lo
                if head_x >= center {
                    let target_scroll = self.transport_seconds * pps - center;
                    self.scroll_x = target_scroll.max(0.0);
                }
            }
        }
    }
}

/// Ponto de entrada do binário/lib: cria a `EventLoop` e roda o handler.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    window::run()
}