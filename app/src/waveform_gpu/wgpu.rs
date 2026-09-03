//! Renderer de waveform via wgpu (headless/offscreen).
//!
//! A janela do Slint usa Skia e não pode compartilhar a surface com o
//! wgpu. Esta abordagem renderiza as barras min→max numa textura
//! offscreen com wgpu, faz readback RGBA para CPU e devolve uma imagem
//! que o Slint exibe num elemento `Image` por clip.
//!
//! Estratégia GPU: um quad unitário expandido por instância. Cada
//! coluna da waveform (1px de largura) é uma instância; a vertex shader
//! lê (min,max) num storage buffer e mapeia amplitude [-1,1] → NDC Y.

use std::sync::Arc;
use std::sync::Mutex;
use wgpu::util::DeviceExt;

/// Contexto wgpu headless compartilhado (main thread).
pub struct WgpuRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    unit_quad: wgpu::Buffer,
    texture: Mutex<Option<RenderTarget>>,
}

struct RenderTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
}

impl WgpuRenderer {
    /// Inicializa o dispositivo wgpu (headless, sem surface).
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
                label: Some("ruana-waveform-device"),
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
            label: Some("waveform-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("wgpu.wgsl").into()),
        });

        // Bind group: uniform (count/height) + storage (bars).
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("waveform-bgl"),
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
                    visibility: wgpu::ShaderStages::VERTEX,
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
            label: Some("waveform-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Quad unitário (0..1) expandido por instância.
        let unit_quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit-quad"),
            contents: bytemuck::cast_slice(&[
                0.0f32, 0.0f32, 1.0f32, 0.0f32, 0.0f32, 1.0f32, 1.0f32, 1.0f32,
            ]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("waveform-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
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
            texture: Mutex::new(None),
        })
    }

    /// Renderiza barras da waveform numa textura RGBA e faz readback.
    ///
    /// `bars[i]` = amplitude central da i-ésima coluna 1px (já com
    /// ganho/fade aplicados, faixa [-1,1]). `color` é RGBA (0..255).
    /// Retorna `Vec<u8>` RGBA de `width x height`, ou `None` em erro.
    pub fn render(
        &self,
        width: u32,
        height: u32,
        bars: &[f32],
        color: [u8; 4],
    ) -> Option<Vec<u8>> {
        if bars.is_empty() || width == 0 || height == 0 {
            return None;
        }
        let count = bars.len() as u32;

        // Bars em storage buffer (vec2 por bar: min/max ~ mesma amplitude).
        // Como `bars` guarda só a amplitude por coluna, usamos (v, -v).
        let bar_data: Vec<f32> = bars
            .iter()
            .flat_map(|&v| {
                let v = v.clamp(-1.0, 1.0);
                [v, -v]
            })
            .collect();

        let bars_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bars"),
            contents: bytemuck::cast_slice(&bar_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let (bytes_per_row, padded) = {
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let unpadded = width * 4;
            let padded = unpadded.next_multiple_of(align);
            (unpadded, padded)
        };
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded as u64) * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waveform-tex"),
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

        // Uniforms: count e meia-altura (para mapear amplitude → NDC).
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::cast_slice(&[count as f32, 0.0f32, 0.0f32, 0.0f32]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("waveform-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bars_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("waveform-encoder"),
            });

        let (r, g, b, a) = (
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
            color[3] as f32 / 255.0,
        );
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("waveform-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r,
                            g,
                            b,
                            a,
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
            rpass.draw(0..4, 0..count);
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
