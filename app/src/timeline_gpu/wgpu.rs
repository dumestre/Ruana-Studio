//! Motor da Timeline via wgpu (headless/offscreen).
//!
//! A janela do Slint usa Skia e não compartilha surface com wgpu — a mesma
//! restrição documentada em `crate::waveform_gpu`. Portanto a timeline é
//! renderizada numa textura offscreen e, após readback RGBA para a CPU,
//! o resultado vira um `slint::Image` que o Slint exibe.
//!
//! Arquitetura (conforme AGENST.md):
//! - **Câmera ortográfica 2D**: uma matriz de projeção converte coordenadas
//!   de conteúdo (pixels) em NDC, incorporando scroll (pan) e zoom. Zoom/pan
//!   são operações puramente matemáticas na matriz — custo constante.
//! - **Instancing render**: um único quad unitário é expandido por instância.
//!   Cada instância é um retângulo (clip, linha de grade, faixa do ruler,
//!   recorte selecionado). Milhares de clips = um draw call.
//! - **Hit-testing em CPU**: separado do raster, em `mod cursor`/`hit_test`.
//!
//! O dispositivo é criado UMA vez (custoso) e reutilizado; cada render
//! aloca apenas o declarado e devolve RGBA compacto.
//!
//! A API pública (TypeA `TimelineGpuRenderer`, `OrthoCamera`, `RectInstance`)
//! ainda não está conectada ao Slint/wiring — o wiring do canvas é o
//! próximo incremento. `#[allow(dead_code)]` mantém o build limpo enquanto
//! a integração não consome estes itens.

#![allow(dead_code)]

use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Instância retangular enviada ao shader.
///
/// `rect` em pixels de conteúdo (x, y, w, h); `color` RGBA (0..255).
#[derive(Clone, Copy, Debug)]
pub struct RectInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 4],
}

impl RectInstance {
    fn to_f32(self) -> [f32; 8] {
        let [r, g, b, a] = self.color;
        [
            self.x,
            self.y,
            self.w,
            self.h,
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    }
}

/// Câmera ortográfica 2D da timeline.
///
/// Mapeia um pixel de CONTEÚDO `(cx, cy)` para NDC. O conteúdo é o espaço de
/// dados (grid, clips) que se move com scroll e se escala com zoom:
///
/// ```text
/// ndc.x = (cx + scroll_x) / (viewport_w / 2)  - 1
/// ndc.y = 1 - (cy + scroll_y) / (viewport_h / 2)
/// ```
#[derive(Clone, Copy, Debug)]
pub struct OrthoCamera {
    /// Scroll (deslocamento do conteúdo) em pixels.
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// Tamanho da área visível (viewport) em pixels.
    pub viewport_w: f32,
    pub viewport_h: f32,
}

impl OrthoCamera {
    /// Matriz 4x4 column-major (layout de memória do wgpu/glam).
    ///
    /// O wgpu espera o layout `[c0.x, c1.x, c2.x, c3.x, c0.y, ...]`?
    /// Não — `wgpu` (e o glam) usam column-major DENTRO de um array de 16
    /// f32, ou seja: primeiros 4 elementos = primeira coluna. Construímos
    /// as colunas explicitamente.
    pub fn proj(&self) -> [f32; 16] {
        let sx = self.scroll_x;
        let sy = self.scroll_y;
        let hw = (self.viewport_w / 2.0).max(1.0);
        let hh = (self.viewport_h / 2.0).max(1.0);

        // ndc.x = (cx + sx)/hw - 1  →  x' = cx * (1/hw) + (sx/hw - 1)
        // ndc.y = 1 - (cy + sy)/hh  →  y' = cy * (-1/hh) + (1 - sy/hh)
        [
            // coluna 0 (eixo x)
            1.0 / hw,
            0.0,
            0.0,
            0.0,
            // coluna 1 (eixo y)
            0.0,
            -1.0 / hh,
            0.0,
            0.0,
            // coluna 2 (eixo z) — topologia 2D, z fica 0
            0.0,
            0.0,
            1.0,
            0.0,
            // coluna 3 (translação)
            sx / hw - 1.0,
            1.0 - sy / hh,
            0.0,
            1.0,
        ]
    }
}

/// Renderer headless da timeline.
pub struct TimelineGpuRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    unit_quad: wgpu::Buffer,
}

impl TimelineGpuRenderer {
    /// Inicializa o dispositivo wgpu (headless, sem surface) + pipeline.
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            },
        ))
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
            source: wgpu::ShaderSource::Wgsl(include_str!("wgpu.wgsl").into()),
        });

        // Bind group: uniform (matriz proj). Instâncias são atributos de
        // vértice (step-mode Instance), então não precisam de bind group.
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("timeline-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("timeline-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Quad unitário (0..1) expandido por instância.
        let unit_quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("timeline-unit-quad"),
            contents: bytemuck::cast_slice(&[
                0.0f32, 0.0f32, 1.0f32, 0.0f32, 0.0f32, 1.0f32, 1.0f32, 1.0f32,
            ]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("timeline-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    // Corner do quad (por vértice).
                    wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    },
                    // Traits por instância: rect (vec4) + color (vec4).
                    wgpu::VertexBufferLayout {
                        array_stride: 32,
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
                        ],
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
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

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            unit_quad,
        })
    }

    /// Renderiza um frame da timeline (instâncias retangulares) numa
    /// textura RGBA offscreen e faz readback para a CPU.
    ///
    /// - `width`/`height`: resolução da imagem de saída (pixels).
    /// - `camera`: câmera ortográfica (scroll + viewport).
    /// - `instances`: retângulos a desenhar (clips, grid, ruler, faixas).
    ///
    /// Retorna `Vec<u8>` RGB(A) de `width * height * 4` (ou `None` em erro).
    pub fn render_frame(
        &self,
        width: u32,
        height: u32,
        camera: &OrthoCamera,
        instances: &[RectInstance],
    ) -> Option<Vec<u8>> {
        if width == 0 || height == 0 {
            return None;
        }
        let max_dim = self.device.limits().max_texture_dimension_2d.max(1);
        let width = width.min(max_dim);
        let height = height.min(max_dim);

        // Instâncias → buffer, no layout [rect(4) | color(4)].
        let data: Vec<f32> = instances.iter().flat_map(|i| i.to_f32()).collect();

        let instance_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("timeline-instances"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Uniform: matriz de projeção (column-major).
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("timeline-uniforms"),
            contents: bytemuck::cast_slice(&camera.proj()),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("timeline-bg"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });

        let (bytes_per_row, padded) = {
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let unpadded = width * 4;
            (unpadded, unpadded.next_multiple_of(align))
        };

        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timeline-readback"),
            size: (padded as u64) * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("timeline-tex"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("timeline-encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timeline-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let _ = rx.recv();

        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((bytes_per_row as usize) * height as usize);
        for row in 0..height as usize {
            let start = row * padded as usize;
            out.extend_from_slice(&data[start..start + bytes_per_row as usize]);
        }
        drop(data);
        readback.unmap();

        Some(out)
    }
}

/// Atalho: rasteriza um recorte retangular sólido (RGBA) numa imagem, sem
/// criar um renderer persistente. Útil para fundos simples da timeline.
pub fn solid_rect_rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for _ in 0..(width as usize) * (height as usize) {
        out.extend_from_slice(&color);
    }
    out
}