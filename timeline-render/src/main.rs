//! Timeline renderer standalone — wgpu windowed (canvas 2D style).
//!
//! Replica visualmente a timeline do app: régua, grid, clips, playhead e VU.
//! Usa o MESMO shader (app/src/timeline_gpu/wgpu.wgsl) e a mesma lógica de
//! instancing + storage buffer de waveform do `TimelineGpuRenderer` do app.
//!
//! Controles:
//!   - Roda do mouse / touchpad 2 dedos: scroll vertical (e lateral)
//!   - Click na roda + arrastar:          pan na timeline
//!   - Ctrl + PixelDelta vertical:        zoom horizontal (onde suportado)
//!   - Alt + PixelDelta vertical:         zoom vertical (altura das faixas)
//!   - Espaço:                            play/pause do playhead animado
//!   - ESC:                               fecha
//!   - Resize:                            redimensiona o canvas

mod shctx;

use std::sync::Arc;
use std::time::Instant;

use wgpu::util::DeviceExt;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};
use winit::window;

use shctx::SHADER_WGSL;

// Paleta e tipo ----------------------------------------------------------------

/// Paleta idêntica ao app (7 slots de tema, indexado 0..=6).
pub mod palette {
    pub type Rgb = (u8, u8, u8);

    // Theme.base — cor usada para o grid da régua e da timeline.
    pub const BASE: [Rgb; 7] = [
        (0x31, 0x31, 0x31), (0x16, 0x1A, 0x21), (0x19, 0x17, 0x22),
        (0x14, 0x16, 0x0E), (0x21, 0x1A, 0x12), (0x0F, 0x1E, 0x21),
        (0x64, 0x64, 0x64),
    ];

    pub const SURFACE_HIGH: [Rgb; 7] = [
        (0x55, 0x55, 0x55), (0x26, 0x30, 0x3D), (0x2D, 0x27, 0x40),
        (0x26, 0x2A, 0x1D), (0x36, 0x2A, 0x1B), (0x1E, 0x30, 0x35),
        (0x91, 0x91, 0x91),
    ];
    pub const SURFACE_STEP: [Rgb; 7] = [
        (0x42, 0x42, 0x42), (0x23, 0x2D, 0x38), (0x27, 0x23, 0x34),
        (0x20, 0x23, 0x1A), (0x30, 0x26, 0x1F), (0x19, 0x2C, 0x31),
        (0x7A, 0x7A, 0x7A),
    ];
    pub const GRID_LINE: [Rgb; 7] = [
        (0x75, 0x75, 0x75), (0x4A, 0x5A, 0x70), (0x4A, 0x44, 0x66),
        (0x4A, 0x4D, 0x3A), (0x5A, 0x4E, 0x3A), (0x3A, 0x5C, 0x66),
        (0xD6, 0xD6, 0xD6),
    ];
    pub const GRID_SUB: [Rgb; 7] = [
        (0x45, 0x45, 0x45), (0x2A, 0x36, 0x45), (0x2F, 0x2C, 0x42),
        (0x2A, 0x2D, 0x24), (0x3A, 0x30, 0x28), (0x21, 0x36, 0x3C),
        (0xEF, 0xEF, 0xEF),
    ];
    pub const TIMELINE_BG: [Rgb; 7] = [
        (0x69, 0x69, 0x69), (0x1E, 0x2C, 0x35), (0x1F, 0x1B, 0x28),
        (0x18, 0x1B, 0x11), (0x25, 0x21, 0x19), (0x12, 0x26, 0x28),
        (0x79, 0x79, 0x79),
    ];
    pub const CLIP_PALETTE: [Rgb; 20] = [
        (0x6F, 0xA8, 0x6B), (0x4F, 0x8A, 0xE0), (0xE0, 0xA9, 0x4F),
        (0xE0, 0x5E, 0x5E), (0xA0, 0x60, 0xD8), (0xE0, 0x5E, 0xAA),
        (0x3E, 0xC7, 0xB0), (0xC0, 0xC0, 0x50), (0xE0, 0x7A, 0x3E),
        (0x8F, 0xD1, 0x4F), (0x3F, 0xA3, 0x4D), (0x35, 0xB5, 0xE5),
        (0x5A, 0x8F, 0xE0), (0x55, 0x60, 0xD8), (0x9B, 0x5C, 0xE0),
        (0xD8, 0x4F, 0xB2), (0xE0, 0x4F, 0x6F), (0xA8, 0x74, 0x4A),
        (0xD8, 0xC0, 0x4F), (0x7A, 0x93, 0xA8),
    ];
    pub const CLIP_MIDI_DEFAULT: [Rgb; 7] = [
        (0x6F, 0xA8, 0x6B), (0x6F, 0xA8, 0x6B), (0x6F, 0xA8, 0x6B),
        (0x8F, 0xC1, 0x4F), (0x8F, 0xAA, 0x6B), (0x6F, 0xA8, 0x6B),
        (0x52, 0xBA, 0x46),
    ];
    pub const CLIP_AUDIO_DEFAULT: Rgb = (0x00, 0xC8, 0x53);
    pub const TEXT_PRIMARY: [Rgb; 7] = [
        (0xE0, 0xE0, 0xE0), (0xE2, 0xE6, 0xEE), (0xE0, 0xDA, 0xF0),
        (0xD5, 0xE8, 0xB0), (0xE8, 0xD8, 0xC0), (0xC8, 0xE8, 0xE0),
        (0x17, 0x17, 0x17),
    ];
    pub const TEXT_DIM: [Rgb; 7] = [
        (0x9A, 0x9A, 0x9A), (0x84, 0x9A, 0xB5), (0x8F, 0x8A, 0xB0),
        (0x7A, 0x85, 0x9A), (0x9A, 0x94, 0x82), (0x82, 0xC2, 0x92),
        (0x5A, 0x5A, 0x5A),
    ];
    pub const PLAYHEAD_COLOR: Rgb = (0xDC, 0x14, 0x3C);
}

fn rgba((r, g, b): palette::Rgb, a: u8) -> [u8; 4] { [r, g, b, a] }
fn darker((r, g, b): palette::Rgb, amount: f32) -> palette::Rgb {
    let f = (1.0 - amount).max(0.0);
    (
        (r as f32 * f).round() as u8,
        (g as f32 * f).round() as u8,
        (b as f32 * f).round() as u8,
    )
}
fn rgba_alpha((r, g, b): palette::Rgb, alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    [r, g, b, a]
}

// =====================================================
// Bitmap font 8x8 para renderizar números na régua.
// Cada caractere é 8px de largura, 8px de altura.
// Cada bit (1) = pixel ligado. Bitmap é índiceado [linha][coluna].
// =====================================================

/// Fonte bitmap 8x8 para dígitos 0-9.
const FONT_8X8: [[[u8; 8]; 8]; 10] = [
    // '0'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '1'
    [
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 0, 0, 0],
        [0, 1, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '2'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 1, 1, 1, 0],
        [0, 0, 0, 1, 1, 1, 0, 0],
        [0, 0, 1, 1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
    ],
    // '3'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '4'
    [
        [0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 1, 1, 1, 0, 0],
        [0, 0, 1, 0, 1, 1, 0, 0],
        [0, 1, 0, 0, 1, 1, 0, 0],
        [1, 0, 0, 0, 1, 1, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 1, 1, 0],
    ],
    // '5'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '6'
    [
        [0, 0, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '7'
    [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 0, 1, 1, 0],
        [0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
    ],
    // '8'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
    ],
    // '9'
    [
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 0, 0, 0, 0, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [0, 0, 1, 1, 1, 1, 0, 0],
    ],
];

/// Desenha um número (0-999) na régua usando a fonte bitmap 8x8.
/// Cada dígito ocupa 8px de largura. O texto é centrado no x informado.
fn build_text_instances(
    text: &str,
    center_x: f32,
    y: f32,
    color: [u8; 4],
) -> Vec<RectInstance> {
    let char_w = 8.0;
    let gap = 1.0;
    let total_w = (text.len() as f32 - 1.0) * gap + text.len() as f32 * char_w;
    let start_x = center_x - total_w / 2.0;

    let mut instances = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let digit = ch.to_digit(10);
        if let Some(d) = digit {
            let glyph = &FONT_8X8[d as usize];
            for row in 0..8 {
                for col in 0..8 {
                    if glyph[row][col] == 1 {
                        instances.push(RectInstance {
                            x: start_x + i as f32 * (char_w + gap) + col as f32,
                            y: y + row as f32,
                            w: 1.0,
                            h: 1.0,
                            color,
                            wave_offset: 0,
                            wave_count: 0,
                            clip_w: 0.0,
                            clip_h: 0.0,
                            radius: 0.0,
                        });
                    }
                }
            }
        }
    }
    instances
}

/// Instância de render (mesmo layout do shader).
#[derive(Clone, Copy, Debug)]
pub struct RectInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 4],
    pub wave_offset: u32,
    pub wave_count: u32,
    pub clip_w: f32,
    pub clip_h: f32,
    pub radius: f32,
}

impl RectInstance {
    fn to_f32(self) -> [f32; 13] {
        let [r, g, b, a] = self.color;
        [
            self.x, self.y, self.w, self.h,
            r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0,
            self.wave_offset as f32, self.wave_count as f32,
            self.clip_w, self.clip_h,
            self.radius,
        ]
    }
}

/// Câmera ortográfica 2D.
#[derive(Clone, Copy, Debug)]
pub struct OrthoCamera {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
}

impl OrthoCamera {
    pub fn proj(&self) -> [f32; 16] {
        let sx = self.scroll_x;
        let sy = self.scroll_y;
        let hw = (self.viewport_w / 2.0).max(1.0);
        let hh = (self.viewport_h / 2.0).max(1.0);
        [
            1.0 / hw, 0.0, 0.0, 0.0,
            0.0, -1.0 / hh, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            -sx / hw - 1.0, 1.0 + sy / hh, 0.0, 1.0,
        ]
    }
}

// ============================================================================
// Dados demo
// ============================================================================

struct ClipDemo {
    track_index: usize,
    beat: f32,
    len: f32,
    is_midi: bool,
    color: usize,
}

// ============================================================================
// App
// ============================================================================

struct App {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    unit_quad: wgpu::Buffer,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,

    theme: usize,
    scroll_x: f32,
    scroll_y: f32,
    px_per_beat: f32,
    zoom_y: f32,
    ruler_px: f32,
    playing: bool,
    transport_seconds: f32,
    last_play_head_update: Instant,
    panning: bool,
    last_cursor: (f32, f32),

    wave_bars: Vec<f32>,
    clips: Vec<ClipDemo>,
}

impl App {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("wgpu: create_surface failed: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "wgpu: no adapter available".to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ruana-timeline-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
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
            width: size.width,
            height: size.height,
            view_formats: vec![],
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("timeline-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    },
                    wgpu::VertexBufferLayout {
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
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
            multiview: None,
            cache: None,
        });

        // === Dados demo ===
        let clips = vec![
            ClipDemo { track_index: 0, beat: 0.0, len: 8.0, is_midi: false, color: 0 },
            ClipDemo { track_index: 0, beat: 10.0, len: 4.0, is_midi: true, color: 0 },
            ClipDemo { track_index: 1, beat: 2.0, len: 12.0, is_midi: false, color: 3 },
            ClipDemo { track_index: 2, beat: 0.0, len: 16.0, is_midi: false, color: 1 },
            ClipDemo { track_index: 2, beat: 18.0, len: 6.0, is_midi: true, color: 0 },
            ClipDemo { track_index: 3, beat: 4.0, len: 20.0, is_midi: false, color: 5 },
        ];

        let mut wave_bars: Vec<f32> = Vec::new();
        let px_per_beat = 40.0;
        for clip in &clips {
            if clip.is_midi {
                continue;
            }
            let n_cols = ((clip.len * px_per_beat).ceil() as u32).max(1).min(256);
            for col in 0..n_cols {
                let phase = (col as f32) / n_cols as f32 * std::f32::consts::TAU;
                let mn = -0.5 * (phase * 1.3).sin();
                let mx = 0.5 * (phase * 0.7 + 0.5).cos();
                wave_bars.push(mn);
                wave_bars.push(mx);
            }
        }

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

            theme: 6,   // <- Mudança: usar Paper (claro) como tema base
            scroll_x: 0.0,
            scroll_y: 0.0,
            px_per_beat,
            zoom_y: 1.0,
            ruler_px: 25.0,
            playing: true,
            transport_seconds: 0.0,
            last_play_head_update: Instant::now(),
            panning: false,
            last_cursor: (0.0, 0.0),

            wave_bars,
            clips,
        })
    }

    fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.size = size;
        if size.width > 0 && size.height > 0 {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn handle_scroll(&mut self, delta: &winit::event::MouseScrollDelta, with_ctrl: bool, with_alt: bool) {
        let (dx, dy) = match *delta {
            winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
            winit::event::MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
        };
        let (mx, my) = self.last_cursor;

        if with_ctrl && dy.abs() >= 1.0 {
            // Zoom horizontal — mantém o ponto sob o cursor fixo.
            let old_px = self.px_per_beat;
            let new_px = (old_px + dy * 0.05).clamp(8.0, 640.0);
            // Beat sob o cursor antes do zoom.
            let beat_under = (mx + self.scroll_x) / old_px.max(0.01);
            self.px_per_beat = new_px;
            self.scroll_x = (beat_under * new_px - mx).max(0.0);
        } else if with_alt && dy.abs() >= 1.0 {
            // Zoom vertical — mantém a linha sob o cursor fixa.
            let old_zy = self.zoom_y;
            let new_zy = (old_zy + dy * 0.01).clamp(0.25, 6.0);
            // Linha sob o cursor antes do zoom (base 40px).
            let row_under = (my - self.ruler_px + self.scroll_y) / (40.0 * old_zy.max(0.01)).max(0.01);
            self.zoom_y = new_zy;
            self.scroll_y = (row_under * 40.0 * new_zy - (my - self.ruler_px)).max(0.0);
        } else {
            // Scroll pelos eixos reais do delta: x → horizontal, y → vertical.
            self.scroll_x = (self.scroll_x - dx).max(0.0);
            self.scroll_y = (self.scroll_y - dy).max(0.0);
        }
    }

    fn handle_mouse_button(&mut self, state: ElementState, pos: Option<(f32, f32)>) {
        match state {
            ElementState::Pressed => {
                self.panning = true;
                if let Some(p) = pos { self.last_cursor = p; }
            }
            ElementState::Released => {
                self.panning = false;
            }
        }
    }

    fn handle_cursor_moved(&mut self, pos: (f32, f32)) {
        if self.panning {
            let (lx, ly) = self.last_cursor;
            self.scroll_x = (self.scroll_x - (pos.0 - lx)).max(0.0);
            self.scroll_y = (self.scroll_y - (pos.1 - ly)).max(0.0);
        }
        self.last_cursor = pos;
    }

    fn build_instances(&self, width: f32, height: f32) -> Vec<RectInstance> {
        let t = self.theme.clamp(0, 6) as usize;
        let mut instances = Vec::new();
        let px_per_bar = self.px_per_beat * 4.0;
        let _total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        let row_height = 40.0 * self.zoom_y;

        // Fundo geral - cobre desde a régua até o fundo
        instances.push(RectInstance {
            x: 0.0, y: self.ruler_px,
            w: width, h: (height - self.ruler_px).max(0.0),
            color: rgba(palette::TIMELINE_BG[t], 255),
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Régua — com listras alternadas (igualdade das faixas/tracks)
        // Desenha segments alternados começando com SURFACE_STEP (mais escuro)
        let ruler_segment_w = px_per_bar; // uma "coluna" de compasso por segmento
        let n_segments = (width / ruler_segment_w).ceil() as usize + 1;
        for seg in 0..n_segments {
            let seg_x = (seg as f32 * ruler_segment_w) - self.scroll_x;
            let is_odd = seg % 2 == 1;
            let bg_color = if is_odd {
                palette::SURFACE_HIGH[t]   // ímpar: mais claro
            } else {
                palette::SURFACE_STEP[t]   // par: mais escuro (primeiro)
            };
            // Largura do segmento, limitado ao viewport
            let seg_w = ruler_segment_w.min(width - seg_x).max(0.0);
            if seg_w > 0.0 {
                instances.push(RectInstance {
                    x: seg_x, y: 0.0, w: seg_w, h: self.ruler_px,
                    color: [bg_color.0, bg_color.1, bg_color.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Fundos alternados das colunas do grid (antes das linhas do grid)
        let px_per_bar = self.px_per_beat * 4.0;
        let total_bars = ((width + self.scroll_x) / px_per_bar).ceil() as usize + 1;
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let is_odd_col = i % 2 == 1;
            let bg = if is_odd_col {
                palette::SURFACE_HIGH[t]
            } else {
                palette::SURFACE_STEP[t]
            };
            let bw = px_per_bar;
            let bh = (height - self.ruler_px).max(0.0); // corpo abaixo da régua
            if bw > 0.0 && bh > 0.0 {
                instances.push(RectInstance {
                    x: bx, y: self.ruler_px - self.scroll_y, w: bw, h: bh,
                    color: [bg.0, bg.1, bg.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
            }
        }

        // Grid do corpo (abaixo da régua) — rola verticalmente com scroll_y,
        // enquanto a régua permanece fixa no topo.
        let body_top = self.ruler_px - self.scroll_y;
        let body_h = (height - self.ruler_px).max(0.0);
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }
            let bar_a = if i % 2 == 1 { 255 } else { 140 };
            instances.push(RectInstance {
                x: bx, y: body_top, w: 1.0, h: body_h,
                color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, bar_a],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });

            // Subdivisões como frações uniformes do compasso (px_per_bar), da
            // mais fina para a mais grossa para que as linhas principais fiquem por cima.
            // 1/64
            if px_per_bar >= 960.0 {
                let n = 64u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 70],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/32
            if px_per_bar >= 480.0 {
                let n = 32u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 90],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/16
            if px_per_bar >= 240.0 {
                let n = 16u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // 1/8
            if px_per_bar >= 120.0 {
                let n = 8u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Batidas (1/4) — mais grossas, por cima das subdivisões
            if px_per_bar >= 40.0 {
                let n = 4u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let xs = bx + m as f32 * step;
                    if xs <= 0.0 || xs > width { continue; }
                    instances.push(RectInstance {
                        x: xs, y: body_top, w: 1.0, h: body_h,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }

        // Régua fixa no topo — será desenhada por cima dos clips mais adiante.
        let ruler_h = self.ruler_px;

        // Clips
        let header_h = 16.0;
        let bw = 1.0;
        for (idx, clip) in self.clips.iter().enumerate() {
            let base = if clip.color == 0 {
                if clip.is_midi { palette::CLIP_MIDI_DEFAULT[t] } else { palette::CLIP_AUDIO_DEFAULT }
            } else {
                palette::CLIP_PALETTE[clip.color % palette::CLIP_PALETTE.len()]
            };

            let x = clip.beat * self.px_per_beat - self.scroll_x;
            let w = (clip.len * self.px_per_beat).max(40.0);
            let y = self.ruler_px + clip.track_index as f32 * row_height - self.scroll_y;

            if w <= 0.0 || x + w < 0.0 || x > width { continue; }
            let draw_w = w.min(width - x).max(0.0);

            // Header
            instances.push(RectInstance {
                x: x + bw, y: y + bw,
                w: (draw_w - 2.0 * bw).max(0.0),
                h: (header_h - bw).max(0.0),
                color: rgba(base, 255),
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });

            // Corpo (com waveform simulado)
            let body_h = row_height - header_h;
            let body_w = w;
            let wave_info_idx = self.clips[..idx].iter().position(|c| !c.is_midi && c.track_index == clip.track_index);
            let (wo, wc) = if clip.is_midi {
                (0u32, 0u32)
            } else {
                // Cada clip de áudio usa as barras acumuladas em wave_bars
                let bar_idx = self.clips[..idx].iter().filter(|c| !c.is_midi).count();
                let offset = bar_idx as u32 * 2;
                let total = self.wave_bars.len() as u32 / 2;
                (offset, total)
            };
            let _ = (wave_info_idx, body_h, body_w, wo, wc);

            instances.push(RectInstance {
                x: x + bw, y: y + header_h,
                w: body_w, h: body_h,
                color: rgba_alpha(darker(base, 0.12), 0.50),
                wave_offset: wo, wave_count: wc, clip_w: body_w, clip_h: body_h, radius: 0.0,
            });

            // Divisória
            instances.push(RectInstance {
                x: x + bw, y: y + header_h - 1.0,
                w: (draw_w - 2.0 * bw).max(0.0), h: 1.0,
                color: rgba(darker(base, 0.18), 255),
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });

            // Indicador direito
            instances.push(RectInstance {
                x: x + draw_w - bw - 6.0, y: y + 5.0,
                w: 3.0, h: 8.0,
                color: rgba(palette::TEXT_PRIMARY[t.clamp(0, 6)], 255),
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
        }

        // Régua fixa no topo — desenhada por cima do grid e dos clips
        // (cobre qualquer track que role por baixo dela).
        // Fundos alternados dos compassos (par/ímpar), igual ao corpo.
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width + px_per_bar || bx < -px_per_bar { continue; }
            let bg = if i % 2 == 1 {
                palette::SURFACE_HIGH[t]
            } else {
                palette::SURFACE_STEP[t]
            };
            let bw = px_per_bar;
            if bx + bw > 0.0 && bx < width && bw > 0.0 {
                let draw_x = bx.max(0.0);
                let draw_w = (bx + bw).min(width) - draw_x;
                if draw_w > 0.0 {
                    instances.push(RectInstance {
                        x: draw_x, y: 0.0, w: draw_w, h: ruler_h,
                        color: [bg.0, bg.1, bg.2, 255],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }
        for i in 0..total_bars {
            let bx = i as f32 * px_per_bar - self.scroll_x;
            if bx > width || bx < -px_per_bar { continue; }
            // Número do compasso
            let ruler_text_color = rgba(palette::TEXT_PRIMARY[t], 220);
            if bx > -px_per_bar * 0.5 && bx < width {
                instances.extend(build_text_instances(&format!("{}", i + 1), bx + 4.0, 3.0, ruler_text_color));
            }
            // Linha vertical do compasso (1px, altura cheia da régua)
            instances.push(RectInstance {
                x: bx, y: 0.0, w: 1.0, h: ruler_h,
                color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
            });
            let beat_px = px_per_bar / 4.0;
            for k in 0..4 {
                let x = bx + k as f32 * beat_px;
                if x <= 0.0 || x > width { continue; }
                // Linhas de batida (quando px_per_bar >= 40)
                if px_per_bar >= 40.0 {
                    instances.push(RectInstance {
                        x, y: 2.0, w: 1.0, h: ruler_h - 2.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 150],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Subdivisões 1/8 (quando px_per_bar >= 120) — tiques menores no baixo,
            // como frações uniformes do compasso
            if px_per_bar >= 120.0 {
                let n = 8u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let x8 = bx + m as f32 * step;
                    if x8 <= 0.0 || x8 > width { continue; }
                    instances.push(RectInstance {
                        x: x8, y: ruler_h - 8.0, w: 1.0, h: 8.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 130],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
            // Subdivisões 1/16 (quando px_per_bar >= 240) — tiques ainda menores
            if px_per_bar >= 240.0 {
                let n = 16u32;
                let step = px_per_bar / n as f32;
                for m in 1..n {
                    let x16 = bx + m as f32 * step;
                    if x16 <= 0.0 || x16 > width { continue; }
                    instances.push(RectInstance {
                        x: x16, y: ruler_h - 6.0, w: 1.0, h: 6.0,
                        color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 110],
                        wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                    });
                }
            }
        }
        // Linha inferior da régua (borda entre a régua e a timeline)
        instances.push(RectInstance {
            x: 0.0, y: ruler_h - 1.0, w: width, h: 1.0,
            color: [palette::BASE[t].0, palette::BASE[t].1, palette::BASE[t].2, 255],
            wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
        });

        // Playhead
        if self.playing {
            let bpm = 120.0;
            let pixels_per_second = self.px_per_beat * bpm / 60.0;
            let head_x = self.transport_seconds * pixels_per_second - self.scroll_x;
            instances.push(RectInstance {
                x: head_x - 1.0, y: 1.0,
                w: 2.0, h: height - 1.0,
                color: [palette::PLAYHEAD_COLOR.0, palette::PLAYHEAD_COLOR.1, palette::PLAYHEAD_COLOR.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0,
                radius: 0.0,
            });
            instances.push(RectInstance {
                x: head_x - 6.0, y: 1.0,
                w: 12.0, h: 12.0,
                color: [palette::PLAYHEAD_COLOR.0, palette::PLAYHEAD_COLOR.1, palette::PLAYHEAD_COLOR.2, 255],
                wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0,
                radius: 6.0,
            });
        }

        instances
    }

    fn render(&mut self) {
        let size = self.size;
        if size.width == 0 || size.height == 0 {
            return;
        }

        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(_e) => {
                self.surface.configure(&self.device, &self.config);
                self.surface
                    .get_current_texture()
                    .expect("surface texture unavailable")
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("timeline-encoder"),
            });

        let width = size.width as f32;
        let height = size.height as f32;
        let instances = self.build_instances(width, height);

        let data: Vec<f32> = instances.iter().flat_map(|i| i.to_f32()).collect();
        let instance_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::cast_slice(&OrthoCamera {
                scroll_x: 0.0,
                scroll_y: 0.0,
                viewport_w: width.max(1.0),
                viewport_h: height.max(1.0),
            }.proj()),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let wave_bars_data = if self.wave_bars.is_empty() {
            vec![0.0f32, 0.0f32]
        } else {
            self.wave_bars.clone()
        };
        let wave_bars_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wave-bars"),
            contents: bytemuck::cast_slice(&wave_bars_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wave_bars_buf.as_entire_binding(),
                },
            ],
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timeline-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1, g: 0.1, b: 0.12, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.set_vertex_buffer(0, self.unit_quad.slice(..));
            rpass.set_vertex_buffer(1, instance_buf.slice(..));
            rpass.draw(0..4, 0..instances.len() as u32);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    fn update_playhead(&mut self) {
        if !self.playing { return; }
        let dt = self.last_play_head_update.elapsed().as_secs_f32();
        self.last_play_head_update = Instant::now();
        self.transport_seconds += dt;
    }
}

// ============================================================================
// Event handler (winit 0.30)
// ============================================================================

struct AppHandler {
    app: Option<App>,
    window_id: WindowId,
    ctrl_pressed: bool,
    alt_pressed: bool,
}

impl AppHandler {
    fn new() -> Self {
        Self {
            app: None,
            window_id: WindowId::dummy(),
            ctrl_pressed: false,
            alt_pressed: false,
        }
    }
}

impl winit::application::ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none() {
            let window = event_loop
                .create_window(
                    window::Window::default_attributes()
                        .with_title("timeline-rendered — wgpu standalone")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
                )
                .expect("failed to create window");
            self.window_id = window.id();
            let win = Arc::new(window);
            self.app = Some(App::new(win).expect("app init failed"));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if window_id != self.window_id { return; }
        let app = self.app.as_mut().expect("app not initialized");
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                app.resize(size);
            }
            WindowEvent::RedrawRequested => {
                app.update_playhead();
                app.render();
            }
            WindowEvent::MouseWheel {
                device_id: _,
                delta,
                phase: _,
            } => {
                app.handle_scroll(&delta, self.ctrl_pressed, self.alt_pressed);
            }
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                ..
            } => {
                let pressed = event.state == ElementState::Pressed;
                let vk = event.logical_key;
                if pressed && vk == Key::Named(NamedKey::Space) {
                    app.playing = !app.playing;
                    app.last_play_head_update = Instant::now();
                }
                if pressed && vk == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
                // Track ctrl/alt para zoom com touchpad/mouse.
                use winit::keyboard::{KeyCode, PhysicalKey};
                let is_ctrl = matches!(vk, Key::Named(NamedKey::Control))
                    || matches!(event.physical_key, PhysicalKey::Code(code) if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight));
                let is_alt = matches!(vk, Key::Named(NamedKey::Alt))
                    || matches!(event.physical_key, PhysicalKey::Code(code) if matches!(code, KeyCode::AltLeft | KeyCode::AltRight));
                self.ctrl_pressed = pressed && is_ctrl;
                self.alt_pressed = pressed && is_alt;
            }
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
                ..
            } => {
                if button == winit::event::MouseButton::Middle {
                    let pos = Some(app.last_cursor);
                    app.handle_mouse_button(state, pos);
                }
            }
            WindowEvent::CursorMoved {
                device_id: _,
                position,
                ..
            } => {
                app.handle_cursor_moved((position.x as f32, position.y as f32));
            }
            WindowEvent::Focused(focused) => {
                if !focused {
                    app.playing = false;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            if app.playing {
                app.window.request_redraw();
            }
        }
    }
}

// ============================================================================
// Entry point
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut handler = AppHandler::new();
    event_loop.run_app(&mut handler)?;
    Ok(())
}
