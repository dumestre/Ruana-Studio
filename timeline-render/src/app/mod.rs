//! Módulo `app`: estado do aplicativo (struct `App`), construtor, métodos
//! base e o laço de eventos ((re)exporta `run`).

pub(crate) mod audio;
pub(crate) mod edit_toolbar;
pub(crate) mod interaction;
pub(crate) mod layout;
pub(crate) mod piano_roll;
pub(crate) mod render;
pub(crate) mod timewarp;
pub mod window;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use wgpu::util::DeviceExt;
use daw_core::Transport;
#[cfg(feature = "audio")]
use daw_audio::AudioEngine;

/// Estado de botão do mouse teclado (neutro; convertido do winit no entry point).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonState {
    Pressed,
    Released,
}

impl From<winit::event::ElementState> for ButtonState {
    fn from(s: winit::event::ElementState) -> Self {
        match s {
            winit::event::ElementState::Pressed => ButtonState::Pressed,
            winit::event::ElementState::Released => ButtonState::Released,
        }
    }
}

/// O que o HOST/loop deve fazer além do que o motor já processou numa tecla.
/// Espaço e Esc NÃO mexem no estado do motor: o host decide (play/pause no
/// transporte do app; standalone fecha na Esc).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyAction {
    /// O motor já tratou internamente (atalhos, ferramentas, *clipboard*…).
    None,
    /// Espaço: o host decide o play/pause do transporte.
    TogglePlay,
    /// Esc: o host decide (standalone fecha a janela; hosted ignora).
    ExitWindow,
}

use crate::clips;
use crate::fades;
use crate::shctx::{ICON_SHADER_WGSL, OVERLAY_SHADER_WGSL, SHADER_WGSL};
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

pub struct App {
    /// Janela winit opcional (modo standalone). Em modo "engine pura"
    /// (`None`) a renderização vai para textura offscreen via
    /// [`App::render_to_rgba`].
    pub window: Option<std::sync::Arc<winit::window::Window>>,
    pub surface: Option<wgpu::Surface<'static>>,
    pub config: Option<wgpu::SurfaceConfiguration>,
    pub render_format: wgpu::TextureFormat,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    /// Recursos originais para compartilhar com o renderer wgpu do Slint
    /// (Skia) — a UI passa a ser desenhada na MESMA device/queue desta engine.
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub unit_quad: wgpu::Buffer,
    pub size: (u32, u32),

/// Textura alvo da renderização offscreen (engine pura). Reutilizada
    /// enquanto o tamanho não mudar.
    pub offscreen_texture: Option<wgpu::Texture>,
    pub readback_buffer: Option<wgpu::Buffer>,
    pub readback_size: (u32, u32),
    /// Um frame foi submetido e o readback dele ainda não foi coletado.
    /// Enquanto isso, novos `submit_frame` são descartados (não empilhar).
    pub readback_pending: bool,
    /// Sinalizado pelo callback do `map_async` quando a cópia terminou.
    pub readback_done: std::sync::Arc<std::sync::atomic::AtomicBool>,

    pub theme: usize,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub px_per_beat: f32,
    pub zoom_y: f32,
    pub ruler_px: f32,
    pub playing: bool,
    /// Snap do grid (toolbar edit): true = deltas de arrasto em beats inteiros;
    /// false = posição livre (beats fracionários — o core já persiste float).
    pub snap_enabled: bool,
    pub transport_seconds: f32,
    pub play_start_seconds: f32,
    /// Início do último play, para o espaço retomar do começo (comportamento DAM).
    pub last_play_start: f32,
    pub last_play_head_update: Instant,
    pub panning: bool,
    pub scrubbing: bool,
    pub last_cursor: (f32, f32),

    pub transport: Arc<Mutex<Transport>>,
    #[cfg(feature = "audio")]
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

    pub focus_saved: Option<(f32, f32, f32, f32)>,
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
        pub cursor_warp_pending: bool,
        // ------------------------------------------------------------

    /// ===== Piano roll (editor MIDI) =====
    /// Editor de notas: fonte de verdade compartilhado com o Slint (daw-midi::NoteEditor).
    /// Quando `show_piano_roll` é verdadeiro, a área inferior da janela exibe
    /// o editor em grade em vez da time_ruler.
    pub note_editor: daw_midi::NoteEditor,
    pub piano_roll_notes: Vec<piano_roll::PianoRollNote>,
    pub piano_roll_inter: piano_roll::PianoRollInteraction,
    pub piano_roll_scroll_y: f32,
    pub piano_roll_px_per_beat: f32,
    pub piano_roll_scroll_x: f32,
    pub piano_roll_key_height: f32,
    pub show_piano_roll: bool,
        /// Estado de teclas modificadoras (para scroll/zoom contextual).
        pub ctrl_pressed: bool,
        pub alt_pressed: bool,
        pub shift_pressed: bool,
    /// Altura atual do piano roll (drag da splitter).
    pub piano_roll_height: f32,
    /// Altura mínima/máxima do piano roll.
    pub piano_roll_min_h: f32,
    pub piano_roll_max_h: f32,
    /// true quando a splitter está sendo arrastada.
    pub splitter_drag: bool,

    /// ===== Piano roll ruler: estado INDEPENDENTE da ruler =====
    /// Posição INDEPENDENTE da playhead no piano roll (em beats).
    /// Não compartilha com `transport_seconds` da timeline.
    pub pr_playhead_beat: f32,
    /// Loop no piano roll (INDEPENDENTE da timeline).
    pub pr_loop_start_beat: f32,
    pub pr_loop_end_beat: f32,
    pub pr_loop_enabled: bool,
    pub pr_loop_drag: Option<LoopDrag>,
    pub pr_loop_drag_start_x: f32,
    pub pr_loop_drag_orig_start: f32,
    pub pr_loop_drag_orig_end: f32,
    /// Scrubbing na ruler do piano roll (INDEPENDENTE).
    pub pr_scrubbing: bool,
    /// Follow playhead no piano roll (INDEPENDENTE).
    pub pr_follow_playhead: bool,

    /// ===== Timewarp (editor de tempo estilo Cubase) =====
    /// Espelho do mapa de tempo do projeto (`TempoMap` é a fonte da verdade
    /// em `daw_core::project`; aqui é a cópia usada pela renderização).
    pub tempo_map: daw_core::timeline::TempoMap,
    /// Zoom (px/beat) e scroll horizontal INDEPENDENTES do timewarp.
    pub timewarp_px_per_beat: f32,
    pub timewarp_scroll_x: f32,
    /// true quando o timewarp expande na área inferior (exclusivo com o PR).
    pub show_timewarp: bool,
    /// Estado de interação (arrasto de pontos, scrub).
    pub timewarp_inter: timewarp::TimewarpInteraction,
    /// Scrubbing na régua do timewarp (playhead próprio).
    pub tw_scrubbing: bool,
    /// Playhead INDEPENDENTE do timewarp (em beats). Durante playback sem
    /// scrub, a posição vem do transporte via `tempo_map.seconds_to_beats`.
    pub tw_playhead_beat: f32,
    /// Callback de write-back: invocado com o mapa atualizado após qualquer
    /// edição de tempo na engine. O host aplica no `Project::tempo_map`.
    /// Roda sempre na thread da engine (não precisa ser `Send`/`Sync`).
    pub tempo_write_back: Option<Box<dyn Fn(&daw_core::timeline::TempoMap)>>,

    /// ===== Edit toolbar (ferramentas do piano roll) =====
    /// Pipeline de quads texturizados (ícones da toolbar).
    pub icon_pipeline: wgpu::RenderPipeline,
    pub icon_bind_group_layout: wgpu::BindGroupLayout,
    pub icon_sampler: wgpu::Sampler,
    pub icon_atlas_view: wgpu::TextureView,
    /// Índice (0..5) do botão sob o cursor, ou None.
    pub toolbar_hover: Option<usize>,

    /// ===== Overlay de UI (Slint etc.) =====
    /// Textura RGBA renderizada por software (host) composta por cima da cena
    /// no fim de cada frame. `None` quando não há overlay.
    pub overlay_texture: Option<wgpu::Texture>,
    pub overlay_view: Option<wgpu::TextureView>,
    pub overlay_size: (u32, u32),
    /// Pipeline que desenha a textura cobrindo o viewport inteiro.
    pub overlay_pipeline: wgpu::RenderPipeline,
    pub overlay_bind_group_layout: wgpu::BindGroupLayout,
    pub overlay_sampler: wgpu::Sampler,

    /// ===== Modo hostilhado (app é dono do winit) =====
    /// Em `hosted`, a cena da timeline é rasterizada OFFSCREEN (via
    /// `submit_frame`/`take_rgba`), enviada por `set_timeline_bg` e composta
    /// na surface NA POSIÇÃO do retângulo da timeline (`timeline_rect`), com
    /// a UI do Slint (overlay) por cima — o `render()` NÃO desenha a cena
    /// diretamente na surface.
    pub hosted: bool,
    /// Tamanho da SURFACE (janela), distinto de `size` (cena da timeline).
    pub surface_size: (u32, u32),
    /// `true` enquanto `submit_frame` está rasterizando o frame offscreen:
    /// nesse target a composição (timeline-bg/overlay) nunca é desenhada.
    pub offscreen_target: bool,
    /// Frame RGBA da timeline (readback da cena offscreen) para o quad da
    /// área da timeline na composição.
    pub timeline_bg_texture: Option<wgpu::Texture>,
    pub timeline_bg_view: Option<wgpu::TextureView>,
    pub timeline_bg_size: (u32, u32),
    /// Retângulo (x, y, w, h EM PIXELS FÍSICOS) onde o frame da timeline é
    /// apresentado na surface — a posição da área do canvas no layout do Slint.
    pub timeline_rect: (f32, f32, f32, f32),
    /// Raio dos cantos (px físicos) da timeline na composição — casando com o
    /// `border-radius: 6px` dos painéis do Slint (explorer/clip pool).
    pub timeline_corner_radius: f32,
}

impl App {
    /// Cria a engine. `window` é opcional:
    /// - `Some(window)`: modo standalone (surface + present via winit).
    /// - `None`: engine pura; a renderização usa [`App::render_to_rgba`].
    /// `size` é o tamanho inicial (standalone: `window.inner_size()`).
    /// `transport` compartilhado opcional (senão cria um próprio).
    pub fn new(
        window: Option<std::sync::Arc<winit::window::Window>>,
        size: (u32, u32),
        transport: Option<Arc<Mutex<Transport>>>,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::empty(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: Default::default(),
            display: None,
        });
        let surface = match &window {
            Some(w) => instance
                .create_surface(w.clone())
                .map_err(|e| format!("wgpu: create_surface failed: {e}"))
                .ok(),
            None => None,
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface.as_ref(),
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

        let (render_format, config) = if let Some(surf) = &surface {
            let surface_caps = surf.get_capabilities(&adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .copied()
                .find(|f| f.is_srgb())
                .unwrap_or(surface_caps.formats[0]);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: size.0.max(1),
                height: size.1.max(1),
                view_formats: vec![],
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: surface_caps.alpha_modes[0],
                desired_maximum_frame_latency: 1,
                color_space: Default::default(),
            };
            surf.configure(&device, &config);
            (surface_format, Some(config))
        } else {
            (wgpu::TextureFormat::Rgba8UnormSrgb, None)
        };

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
                    format: render_format,
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

        // === Pipeline de ícones (edit toolbar) ===
        let icon_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("timeline-icon-shader"),
            source: wgpu::ShaderSource::Wgsl(ICON_SHADER_WGSL.into()),
        });

        let icon_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("timeline-icon-bgl"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let icon_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("timeline-icon-pl"),
            bind_group_layouts: &[Some(&icon_bind_group_layout)],
            immediate_size: 0,
        });

        let icon_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("timeline-icon-rp"),
            layout: Some(&icon_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &icon_shader,
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
                        array_stride: 48,
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
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &icon_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: render_format,
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

        // Textura do atlas de ícones (sRGB, alpha straight).
        let icon_atlas_w = edit_toolbar::ICON_ATLAS_W;
        let icon_atlas_h = edit_toolbar::ICON_ATLAS_H;
        let icon_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("icon-atlas"),
            size: wgpu::Extent3d {
                width: icon_atlas_w.max(1),
                height: icon_atlas_h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_pixels = edit_toolbar::ICON_ATLAS_RGBA;
        if !atlas_pixels.is_empty() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &icon_atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                atlas_pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(icon_atlas_w * 4),
                    rows_per_image: Some(icon_atlas_h),
                },
                wgpu::Extent3d {
                    width: icon_atlas_w,
                    height: icon_atlas_h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let icon_atlas_view = icon_atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let icon_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("icon-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // === Pipeline de overlay (textura RGBA por cima da cena) ===
        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("timeline-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER_WGSL.into()),
        });

        let overlay_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("timeline-overlay-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let overlay_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("timeline-overlay-pl"),
            bind_group_layouts: &[Some(&overlay_bind_group_layout)],
            immediate_size: 0,
        });

        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("timeline-overlay-rp"),
            layout: Some(&overlay_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: render_format,
                    // O buffer do software renderer do Slint é premultiplicado.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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

        let overlay_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // === Dados demo ===
        // Abre com apenas UMA track padrão, sem clips. O usuário importa o
        // áudio (por drag & drop ou menu) e a timeline cria as tracks.
        let mut clips: Vec<clips::ClipDemo> = Vec::new();

        // Gera as ondas dos clips demo e armazena cada uma em `clip.wave`
        // (a fonte da verdade). Buffer e segmentação derivam disso.
        let (wave_bars, wave_segments) = waveform::build_wave_bars_and_segments(&mut clips, WAVE_PPB, 120.0);
        // Cria um vetor de fades padrão para esses clips (não‑midi).
        let fades = clips.iter().map(|c| {
            if !c.is_midi {
                fades::ClipFade::default()
            } else {
                fades::ClipFade::none()
            }
        }).collect::<Vec<_>>();

        let transport = transport.unwrap_or_else(|| Arc::new(Mutex::new(Transport::default())));
        #[cfg(feature = "audio")]
        let audio_engine = AudioEngine::start(transport.clone()).ok();
#[cfg(not(feature = "audio"))]
        let _audio_engine: Option<()> = None;

        Ok(Self {
            window,
            surface,
            render_format,
            device,
            queue,
            instance,
            adapter,
            pipeline,
            bind_group_layout,
            unit_quad,
            config,
            size,
offscreen_texture: None,
            readback_buffer: None,
            readback_size: (0, 0),
            readback_pending: false,
            readback_done: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),

            theme: 6,
            scroll_x: 0.0,
            scroll_y: 0.0,
            px_per_beat: 40.0,
            zoom_y: 1.0,
            ruler_px: 28.0,
            playing: false,
            snap_enabled: true,
            transport_seconds: 0.0,
            play_start_seconds: 0.0,
            last_play_start: 0.0,
            last_play_head_update: Instant::now(),
            panning: false,
            scrubbing: false,
            last_cursor: (0.0, 0.0),

transport,
            #[cfg(feature = "audio")]
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
            cursor_warp_pending: false,

            // Piano roll: inicialmente oculto, com altura padrão.
            note_editor: daw_midi::NoteEditor::new(),
            piano_roll_notes: Vec::new(),
            piano_roll_inter: piano_roll::PianoRollInteraction::new(),
            piano_roll_scroll_y: 0.0,
            piano_roll_px_per_beat: 40.0,
            piano_roll_scroll_x: 0.0,
            piano_roll_key_height: 6.0,
            show_piano_roll: false,
            ctrl_pressed: false,
            alt_pressed: false,
            shift_pressed: false,
            piano_roll_height: 160.0,
            piano_roll_min_h: 60.0,
            piano_roll_max_h: 480.0,
            splitter_drag: false,

            // Piano roll ruler (estado INDEPENDENTE da timeline)
            pr_playhead_beat: 0.0,
            pr_loop_start_beat: 0.0,
            pr_loop_end_beat: 8.0,
            pr_loop_enabled: true,
            pr_loop_drag: None,
            pr_loop_drag_start_x: 0.0,
            pr_loop_drag_orig_start: 0.0,
            pr_loop_drag_orig_end: 0.0,
            pr_scrubbing: false,
            pr_follow_playhead: false,

            // Timewarp (exclusivo com o piano roll, inicialmente oculto).
            tempo_map: {
                // Demo standalone: curva de tempo visível ao abrir com `T`.
                let mut tm = daw_core::timeline::TempoMap::from_bpm(120.0);
                tm.set_change(64.0, 140.0);
                tm.set_change(128.0, 100.0);
                tm.set_change(192.0, 128.0);
                tm
            },
            timewarp_px_per_beat: 40.0,
            timewarp_scroll_x: 0.0,
            show_timewarp: false,
            timewarp_inter: timewarp::TimewarpInteraction::new(),
            tw_scrubbing: false,
            tw_playhead_beat: 0.0,
            tempo_write_back: None,

            // Edit toolbar
            icon_pipeline,
            icon_bind_group_layout,
            icon_sampler,
            icon_atlas_view,
            toolbar_hover: None,

            // Overlay de UI (Slint etc.)
            overlay_texture: None,
            overlay_view: None,
            overlay_size: (0, 0),
            overlay_pipeline,
            overlay_bind_group_layout,
            overlay_sampler,

            hosted: false,
            surface_size: size,
            offscreen_target: false,
            timeline_bg_texture: None,
            timeline_bg_view: None,
            timeline_bg_size: (0, 0),
            timeline_rect: (0.0, 0.0, 0.0, 0.0),
            timeline_corner_radius: 6.0,
        })
    }

    pub fn init_tools(&self) {
        let _ = init_tool_manager();
        let _ = set_dark_mode(self.theme == 6);
    }

    /// Redesenha a janela (no-op no modo engine pura, sem janela).
    pub fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Garante a textura do overlay no tamanho pedido e devolve uma referência
    /// para ela. A textura serve como ALVO de render do Slint (Skia/wgpu) e
    /// como fonte na composição, então precisa de `RENDER_ATTACHMENT`. Só
    /// recria quando o tamanho muda.
    pub fn ensure_overlay_texture(&mut self, width: u32, height: u32) -> Option<&wgpu::Texture> {
        if width == 0 || height == 0 {
            return None;
        }
        let recreate = self.overlay_texture.as_ref().map_or(true, |t| {
            t.width() != width || t.height() != height
        });
        if recreate {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("slint-overlay"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            // O Skia desenha na textura pelo queue raw do backend (invisível ao
            // tracker de usos do wgpu). Sem uma escrita do wgpu, o 1º sample no
            // composite enxerga a sub-resource como "uninitialized" e o wgpu
            // injeta um clear transparente POR CIMA do que o Skia já desenhou —
            // a UI some até um repaint seguinte (hover) marcar a textura. Espelha
            // o path on-screen do renderer Skia (limpa via wgpu ANTES do Skia).
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("slint-overlay-init"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("slint-overlay-init"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            self.queue.submit(Some(encoder.finish()));
            self.overlay_view = Some(view);
            self.overlay_texture = Some(texture);
            self.overlay_size = (width, height);
        }
        self.overlay_texture.as_ref()
    }

    /// Envia uma textura RGBA (pixels straight, linha 0 = topo) para ser
    /// composta por cima da cena no fim de cada frame. Se o tamanho mudar, a
    /// textura GPU é (re)criada; senão, reutiliza e apenas escreve os pixels.
    pub fn set_overlay(&mut self, rgba: &[u8], width: u32, height: u32) {
        if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
            return;
        }
        let recreate = self.overlay_texture.as_ref().map_or(true, |t| {
            t.width() != width || t.height() != height
        });
        if recreate {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("slint-overlay"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.overlay_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.overlay_texture = Some(texture);
            self.overlay_size = (width, height);
        }
        if let Some(texture) = &self.overlay_texture {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Remove o overlay (a cena volta a cobrir a janela inteira).
    pub fn clear_overlay(&mut self) {
        self.overlay_texture = None;
        self.overlay_view = None;
        self.overlay_size = (0, 0);
    }

    /// Liga/desliga o modo hostilhado (app dono do winit). Em `hosted`, o
    /// `render()` compõe o frame da timeline (`timeline_bg`) na posição do
    /// retângulo da timeline + o overlay Slint, sem desenhar a cena na surface.
    pub fn set_hosted(&mut self, hosted: bool) {
        self.hosted = hosted;
    }

    /// Define onde o frame da timeline aparece na surface (px físicos).
    pub fn set_timeline_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.timeline_rect = (x, y, w, h);
    }

    /// Define o raio dos cantos (logical) da timeline na composição. O host
    /// converte para px físicos (raio * devicePixelRatio) para casar com o
    /// `border-radius: 6px` dos painéis do Slint.
    pub fn set_timeline_corner_radius(&mut self, radius: f32) {
        self.timeline_corner_radius = radius;
    }

    /// Envia o frame (readback) da cena da timeline (RGBA, linha 0 = topo)
    /// para ser apresentado no retângulo da timeline (`timeline_rect`). Se o
    /// tamanho mudar a textura GPU é (re)criada; senão apenas escreve os pixels.
    /// A textura também é alvo de render para o [`App::render_timeline_bg`].
    pub fn set_timeline_bg(&mut self, rgba: &[u8], width: u32, height: u32) {
        if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
            return;
        }
        self.ensure_timeline_bg(width, height);
        if let Some(texture) = &self.timeline_bg_texture {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Garante que a textura `timeline_bg` existe em `width x height`. Além de
    /// ser amostrada no `render()` hostilhado, ela é alvo de render da cena
    /// (via `render_timeline_bg`) — por isso ganha `RENDER_ATTACHMENT`.
    fn ensure_timeline_bg(&mut self, width: u32, height: u32) {
        let recreate = self.timeline_bg_texture.as_ref().map_or(true, |t| {
            t.width() != width || t.height() != height
        });
        if recreate {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("timeline-bg"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Mesmo formato do pipeline da cena (`timeline-rp`): quando
                // hosted, a surface usa Bgra8UnormSrgb e a textura precisa
                // casar para o `render_timeline_bg` renderizar nela.
                format: self.render_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.timeline_bg_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.timeline_bg_texture = Some(texture);
            self.timeline_bg_size = (width, height);
        }
    }

    /// Aplica um novo tamanho (modo standalone reconfigure a surface; modo
    /// engine pura apenas atualiza `size`).
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        self.surface_size = size;
        if let (Some(surface), Some(config)) = (&self.surface, self.config.as_mut()) {
            if size.0 > 0 && size.1 > 0 {
                config.width = size.0;
                config.height = size.1;
                surface.configure(&self.device, config);
            }
        }
        self.clamp_scroll_y(size.1 as f32);
    }

    pub fn set_playhead_from_x(&mut self, mx: f32) {
        let bpm = self.bpm;
        let pps = self.px_per_beat * bpm / 60.0;
        self.transport_seconds = ((mx + self.scroll_x) / pps).max(0.0);
        // Um clique/posicionamento manual zera o "retomar do último play".
        self.last_play_start = self.transport_seconds;
    }

    /// Espaço (play/pause/retomar) com a semântica de DAW do standalone:
    ///  1. Tocar: começa do ponto atual;
    ///  2. Parar (segunda vez): pausa a agulha NO LUGAR;
    ///  3. Tocar de novo: volta ao início do último play e recomeça de lá.
    /// Atua no transporte compartilhado — o motor de áudio (daw_audio) que
    /// publica a posição é o MESMO, então o áudio e o feedback do playhead
    /// ficam idênticos ao renderer standalone, sem passar pela UI do Slint.
    pub fn handle_toggle_play(&mut self) {
        if self.playing {
            if let Ok(mut t) = self.transport.lock() {
                t.playing = false;
            }
            self.playing = false;
            // Guarda o ponto onde o play foi interrompido para, ao retomar
            // com espaço, pular de volta pro início do último play.
            self.last_play_start = self.play_start_seconds;
        } else {
            // Se houve um play anterior interrompido por espaço, o próximo
            // espaço volta ao início daquele play.
            self.transport_seconds = self.last_play_start;
            // Sincroniza o projeto com o motor de áudio antes de tocar.
            #[cfg(feature = "audio")]
            self.sync_audio_engine();
            // Sincroniza o estado do loop no transporte compartilhado.
            if let Ok(mut t) = self.transport.lock() {
                t.bpm = self.bpm as u32;
                t.loop_enabled = self.loop_enabled;
                if self.loop_enabled {
                    t.loop_start_ticks = (self.loop_start_beat * t.ppq as f32) as u64;
                    t.loop_end_ticks = (self.loop_end_beat * t.ppq as f32) as u64;
                }
                // Posiciona no playhead atual.
                let beats = self.transport_seconds * self.bpm / 60.0;
                t.position_ticks = (beats * t.ppq as f32) as u64;
                t.playing = true;
            }
            self.play_start_seconds = self.transport_seconds;
            self.playing = true;
        }
        self.last_play_head_update = Instant::now();
        self.request_redraw();
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
                let center = self.size.0 as f32 / 2.0;
                // Se o playhead passou do centro, rola a timeline para segui-lo
                if head_x >= center {
                    let target_scroll = self.transport_seconds * pps - center;
                    self.scroll_x = target_scroll.max(0.0);
                }
            }
        }
    }

    /// Sincroniza o estado de notas do `NoteEditor` (fonte de verdade do daw-midi)
    /// para o cache local usado pela renderização do piano roll.
    /// Esta função é chamada no início de `build_instances` para que o wgpu
    /// reflita sempre as últimas edições feitas no Slint UI.
    pub fn sync_piano_roll_from_core(&mut self) {
        self.piano_roll_notes.clear();
        for note in self.note_editor.notes() {
            self.piano_roll_notes.push(piano_roll::PianoRollNote {
                id: note.id,
                pitch: note.pitch,
                start: note.start,
                length: note.length,
                velocity: note.velocity,
                selected: false,
            });
        }
        self.piano_roll_inter.sync_selection(self.piano_roll_notes.len());
    }

    /// Aplica as notas editadas no wgpu de volta ao `NoteEditor`.
    /// Chamar após movimento de nota ou criação via draw tool.
    pub fn sync_piano_roll_to_core(&mut self) {
        for note in &self.piano_roll_notes {
            let core_note = self.note_editor.notes().iter().find(|n| n.id == note.id);
            if let Some(core_note) = core_note {
                let needs_move = (core_note.start - note.start).abs() > 0.001
                    || core_note.pitch != note.pitch;
                let needs_resize = (core_note.length - note.length).abs() > 0.001;
                let needs_vel = core_note.velocity != note.velocity;

                if needs_move {
                    self.note_editor.move_note(note.id, note.start);
                }
                if needs_resize {
                    self.note_editor.resize_note(note.id, note.length);
                }
                if needs_vel {
                    self.note_editor.change_note_velocity(note.id, note.velocity);
                }
            }
        }
    }

    /// Converte beat → x na ruler do PIANO ROLL (leva em conta KEYBOARD_W,
    /// px_per_beat e scroll_x INDEPENDENTES do piano roll).
    pub fn pr_beat_to_x(&self, beat: f32) -> f32 {
        use piano_roll::KEYBOARD_W;
        KEYBOARD_W + beat * self.piano_roll_px_per_beat - self.piano_roll_scroll_x
    }

    /// Converte x da janela → beat na ruler do PIANO ROLL (considera KEYBOARD_W).
    pub fn pr_x_to_beat(&self, x: f32) -> f32 {
        use piano_roll::KEYBOARD_W;
        ((x - KEYBOARD_W) + self.piano_roll_scroll_x) / self.piano_roll_px_per_beat.max(0.001)
    }

    /// Converte beat → x na ruler do TIMEWARP (considera o gutter de BPM).
    pub fn tw_beat_to_x(&self, beat: f32) -> f32 {
        use timewarp::BPM_LABEL_W;
        BPM_LABEL_W + beat * self.timewarp_px_per_beat - self.timewarp_scroll_x
    }

    /// Converte x da janela → beat na ruler do TIMEWARP (considera o gutter).
    pub fn tw_x_to_beat(&self, x: f32) -> f32 {
        use timewarp::BPM_LABEL_W;
        ((x - BPM_LABEL_W) + self.timewarp_scroll_x) / self.timewarp_px_per_beat.max(0.001)
    }

    /// Posiciona a playhead INDEPENDENTE do timewarp a partir do x na régua.
    pub fn tw_set_scrub_beat(&mut self, mx: f32) {
        self.tw_playhead_beat = self.tw_x_to_beat(mx).max(0.0);
    }

    /// Posiciona a playhead INDEPENDENTE do piano roll a partir do x na ruler.
    /// **NÃO** toca em `transport_seconds` (tocaria a timeline).
    pub fn pr_set_playhead_from_x(&mut self, mx: f32) {
        let beat = self.pr_x_to_beat(mx).max(0.0);
        self.pr_playhead_beat = beat;
    }

    /// Inicia arrasto do loop na ruler do PIANO ROLL.
    /// SÓ inicia se o clique caiu EXATAMENTE numa das alças de 6px
    /// (Start ou End). Qualquer ponto intermediário (incluindo o centro
    /// do loop) retorna false — o clique vira scrub da playhead.
    pub fn pr_begin_loop_drag(&mut self, mx: f32) -> bool {
        if !self.pr_loop_enabled {
            return false;
        }
        let sx = self.pr_beat_to_x(self.pr_loop_start_beat);
        let ex = self.pr_beat_to_x(self.pr_loop_end_beat);
        let grab = 6.0;
        self.pr_loop_drag_orig_start = self.pr_loop_start_beat;
        self.pr_loop_drag_orig_end = self.pr_loop_end_beat;
        self.pr_loop_drag_start_x = mx;
        if (mx - sx).abs() <= grab {
            self.pr_loop_drag = Some(LoopDrag::Start);
        } else if (mx - ex).abs() <= grab {
            self.pr_loop_drag = Some(LoopDrag::End);
        } else {
            // ===== Removido: LoopDrag::Move no centro =====
            // Antes, qualquer clique DENTRO do loop virava "mover loop
            // inteiro", o que bloqueava o scrub em quase toda a ruler.
            // Agora: clique no centro = scrub. Para mover o loop, o
            // usuário arrasta as alças de Start/End (6px cada) ou usa
            // Shift+clique para manter o Move, se quiser depois.
            return false;
        }
        true
    }

    /// Move a seleção de loop do PIANO ROLL com snap de 1 beat.
    pub fn pr_handle_loop_drag(&mut self, mx: f32) {
        let Some(kind) = self.pr_loop_drag else { return };
        let dx = mx - self.pr_loop_drag_start_x;
        let delta_beats = (dx / self.piano_roll_px_per_beat.max(0.001)).round();
        match kind {
            LoopDrag::Start => {
                let new = (self.pr_loop_drag_orig_start + delta_beats)
                    .max(0.0)
                    .min(self.pr_loop_end_beat - 1.0);
                self.pr_loop_start_beat = new.max(0.0);
            }
            LoopDrag::End => {
                let new = (self.pr_loop_drag_orig_end + delta_beats)
                    .max(self.pr_loop_start_beat + 1.0);
                self.pr_loop_end_beat = new;
            }
            LoopDrag::Move => {
                let span = self.pr_loop_drag_orig_end - self.pr_loop_drag_orig_start;
                let new_start = (self.pr_loop_drag_orig_start + delta_beats).max(0.0);
                self.pr_loop_start_beat = new_start;
                self.pr_loop_end_beat = new_start + span;
            }
        }
    }

    /// Clique na ruler do PIANO ROLL: inicia/finaliza scrub.
    pub fn pr_handle_ruler_click(&mut self, state: ButtonState, pos: (f32, f32)) {
        match state {
            ButtonState::Pressed => {
                self.pr_scrubbing = true;
                self.pr_set_playhead_from_x(pos.0);
            }
            ButtonState::Released => {
                self.pr_scrubbing = false;
            }
        }
    }

    /// Alterna a visibilidade do piano roll (tecla P). Fecha o timewarp (os
    /// dois são EXCLUSIVOS na área inferior expandida).
    pub fn toggle_piano_roll(&mut self) {
        self.show_timewarp = false;
        self.show_piano_roll = !self.show_piano_roll;
    }

    /// Alterna a visibilidade do timewarp (tecla T). Fecha o piano roll.
    pub fn toggle_timewarp(&mut self) {
        self.show_piano_roll = false;
        self.show_timewarp = !self.show_timewarp;
    }

    /// Começa o arrasto da splitter entre timeline e área inferior.
    pub fn begin_splitter_drag(&mut self, cursor: (f32, f32)) {
        if !self.show_piano_roll && !self.show_timewarp {
            return;
        }
        let h = self.size.1 as f32;
        if self.splitter_hit_test(cursor.1, h) {
            self.splitter_drag = true;
        }
    }

    /// Arrasta a splitter (entre time_ruler e a régua da área inferior) para
    /// redimensionar a altura do bloco (piano roll OU timewarp) em tempo real.
    pub fn splitter_drag_move(&mut self, cursor_y: f32) {
        if !self.splitter_drag {
            return;
        }
        let h = self.size.1 as f32;
        let tr_h = self.time_ruler_h;
        let area_ruler_h = if self.show_timewarp {
            timewarp::TIMEWARP_RULER_H
        } else {
            piano_roll::PIANO_RULER_H
        };
        // A splitter está em tr_y + tr_h. O cursor está na splitter.
        // Nova posição da splitter = cursor_y (aproximadamente).
        // tr_y = splitter_y - tr_h = cursor_y - tr_h
        let new_tr_y = (cursor_y - tr_h).clamp(self.ruler_px + 4.0, h - tr_h - 4.0);
        let area_ruler_y = new_tr_y + tr_h + 4.0;
        let new_total = (h - area_ruler_y).max(0.0);
        self.piano_roll_height = (new_total - area_ruler_h).max(self.piano_roll_min_h);
    }

    /// Finaliza o arrasto da splitter.
    pub fn end_splitter_drag(&mut self) {
        self.splitter_drag = false;
    }

    /// Gera as instâncias do piano roll e da splitter.
    /// O piano roll é uma área independente (não dentro da timeline).
    pub fn piano_roll_instances(&self, width: f32, height: f32) -> Vec<crate::RectInstance> {
        if !self.show_piano_roll {
            return Vec::new();
        }

        let layout = self.layout(width, height);
        let pr_ruler = layout.piano_ruler.unwrap();
        let pr_rect = layout.piano_roll.unwrap();

        // PianoRollMetrics: ruler_px = topo da régua do PR.
        let m = piano_roll::PianoRollMetrics {
            px_per_beat: self.piano_roll_px_per_beat,
            scroll_x: self.piano_roll_scroll_x,
            scroll_y: self.piano_roll_scroll_y,
            ruler_px: pr_ruler.y + piano_roll::PIANO_RULER_H,
            row_height: 6.0,
            theme: self.theme.clamp(0, 6),
            top_pitch: 96,
            bottom_pitch: 0,
            key_height: self.piano_roll_key_height,
            roll_height: pr_rect.height,
        };

        let t = self.theme.clamp(0, 6);
        let ruler_y = pr_ruler.y;
        let ruler_h = piano_roll::PIANO_RULER_H;
        let body_top = pr_rect.y;
        let body_h = pr_rect.height;

        let mut instances = piano_roll::build_piano_ruler_instances(
            &m,
            width,
            self.pr_playhead_beat,
            self.piano_roll_px_per_beat,
            self.piano_roll_scroll_x,
            t,
            ruler_y,
            ruler_h,
            body_top,
            body_h,
            self.pr_loop_enabled,
            self.pr_loop_start_beat,
            self.pr_loop_end_beat,
            self.pr_follow_playhead,
        );
        instances.extend(piano_roll::build_piano_roll_instances(
            &self.piano_roll_notes,
            &self.piano_roll_inter,
            &m,
            width,
            height,
        ));

        // Splitter
        let splitter_hovered = self.splitter_hit_test(self.last_cursor.1, height);
        instances.extend(piano_roll::build_splitter_instances(
            width,
            layout.splitter.y,
            self.piano_roll_height,
            height,
            splitter_hovered,
            self.theme.clamp(0, 6),
        ));

        // ====== Playhead do PIANO ROLL — desenhada POR ÚLTIMO (z-order mais alto) ======
        // Garante que a linha fique por CIMA de:
        //   - fill do loop (corpo e ruler)
        //   - grid vertical/horizontal do PR
        //   - teclas brancas/pretas do teclado à esquerda
        //   - notas coloridas
        //   - splitter (fica por baixo, a splitter desenha por último? não —
        //     desenhamos a splitter ANTES, então a playhead fica por CIMA da
        //     splitter; é o comportamento desejado: a linha do playhead atravessa
        //     a splitter do PR igual atravessa na timeline.)
        {
            use piano_roll::KEYBOARD_W;
            use crate::palette;
            let px_per_beat = self.piano_roll_px_per_beat;
            let scroll_x = self.piano_roll_scroll_x;
            let beat_to_x = |beat: f32| -> f32 {
                KEYBOARD_W + beat * px_per_beat - scroll_x
            };
            let head_x = beat_to_x(self.pr_playhead_beat);
            if head_x >= -20.0 && head_x <= width + 20.0 {
                let col = palette::PLAYHEAD_COLOR;
                // Linha vertical vermelha: DESCE DO TOPO DA RÉGUA (círculo)
                // ATÉ A BASE DO CORPO DO PIANO ROLL, sem gaps.
                let body_bottom = body_top + body_h.max(0.0);
                let line_top = ruler_y + 1.0;
                let line_h = (body_bottom - line_top).max(0.0);
                instances.push(crate::RectInstance {
                    x: head_x - 1.0, y: line_top,
                    w: 2.0, h: line_h,
                    color: [col.0, col.1, col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
                // Círculo (cabeça da playhead) no topo da régua.
                instances.push(crate::RectInstance {
                    x: head_x - 6.0, y: ruler_y + 1.0,
                    w: 12.0, h: 12.0,
                    color: [col.0, col.1, col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0,
                });
            }
        }

        instances
    }

    /// Gera as instâncias do timewarp (régua reutilizada + corpo + curva +
    /// pontos + playhead). Espelho do `piano_roll_instances`.
    pub fn timewarp_instances(&self, width: f32, height: f32) -> Vec<crate::RectInstance> {
        if !self.show_timewarp {
            return Vec::new();
        }

        let layout = self.layout(width, height);
        let Some(tw_ruler) = layout.timewarp_ruler else { return Vec::new(); };
        let Some(tw_rect) = layout.timewarp else { return Vec::new(); };

        let ruler_y = tw_ruler.y;
        let ruler_h = timewarp::TIMEWARP_RULER_H;
        let body_top = tw_rect.y;
        let body_h = tw_rect.height;

        let m = timewarp::TimewarpMetrics {
            px_per_beat: self.timewarp_px_per_beat,
            scroll_x: self.timewarp_scroll_x,
            ruler_px: ruler_y,
            ruler_h,
            body_top,
            body_h,
            theme: self.theme.clamp(0, 6),
        };

        let t = self.theme.clamp(0, 6);

        let mut instances = timewarp::build_timewarp_ruler_instances(
            width,
            self.timewarp_px_per_beat,
            self.timewarp_scroll_x,
            t,
            ruler_y,
            ruler_h,
            body_top,
            body_h,
            self.pr_follow_playhead,
        );
        instances.extend(timewarp::build_timewarp_instances(
            &self.tempo_map,
            &self.timewarp_inter,
            &m,
            width,
            height,
        ));

        // Splitter (mesma do piano roll — a área expandida é exclusiva).
        let splitter_hovered = self.splitter_hit_test(self.last_cursor.1, height);
        instances.extend(piano_roll::build_splitter_instances(
            width,
            layout.splitter.y,
            self.piano_roll_height,
            height,
            splitter_hovered,
            self.theme.clamp(0, 6),
        ));

        // ====== Playhead do TIMEWARP — desenhada POR ÚLTIMO (z-order alto) ======
        // Em playback usa o transporte convertido pelo mapa de tempo (o efeito
        // do tempo na posição fica visível); sob scrub usa a posição local.
        {
            use timewarp::BPM_LABEL_W;
            use crate::palette;
            let head_beat = if self.tw_scrubbing {
                self.tw_playhead_beat
            } else {
                self.tempo_map.seconds_to_beats(self.transport_seconds as f64) as f32
            };
            let head_x = BPM_LABEL_W + head_beat * self.timewarp_px_per_beat - self.timewarp_scroll_x;
            if head_x >= -20.0 && head_x <= width + 20.0 {
                let col = palette::PLAYHEAD_COLOR;
                let line_top = ruler_y + 1.0;
                let line_h = (body_top + body_h - line_top).max(0.0);
                instances.push(crate::RectInstance {
                    x: head_x - 1.0, y: line_top,
                    w: 2.0, h: line_h,
                    color: [col.0, col.1, col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 0.0,
                });
                instances.push(crate::RectInstance {
                    x: head_x - 6.0, y: ruler_y + 1.0,
                    w: 12.0, h: 12.0,
                    color: [col.0, col.1, col.2, 255],
                    wave_offset: 0, wave_count: 0, clip_w: 0.0, clip_h: 0.0, radius: 6.0,
                });
            }
        }

        instances
    }
}

/// Ponto de entrada do binário/lib: cria a `EventLoop` e roda o handler.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    window::run()
}
