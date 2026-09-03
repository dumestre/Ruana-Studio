//! Renderer de waveform via wgpu (headless/offscreen).
//!
//! A janela do Slint usa Skia e não compartilha surface com wgpu. Este
//! renderer cria um dispositivo headless (sem surface) e renderiza as
//! barras min→max numa textura offscreen, faz readback RGBA para CPU e
//! devolve um `Vec<u8>` que o chamador converte em `slint::Image`.
//!
//! Estratégia GPU: um quad unitário expandido por instância. Cada coluna
//! da waveform é uma instância; a vertex shader lê (min,max) num storage
//! buffer e mapeia amplitude [-1,1] → NDC Y.
//!
//! O dispositivo é criado UMA vez (custoso) e reutilizado para todos os
//! clips; cada render aloca só o necessário e devolve RGBA compacto.

use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Contexto wgpu headless compartilhado (main thread).
pub struct WgpuRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    unit_quad: wgpu::Buffer,
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

        // Bind group: uniform (count/scale) + storage (barras min/max).
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
        })
    }

    /// Renderiza a waveform "Bin Visual" (barras min→max) numa textura
    /// RGBA e faz readback para CPU.
    ///
    /// `mins`/`maxs` são arrays paralelos de amplitude em [-1,1], já com
    /// ganho/fade baked (uma entrada por coluna de 1px). `color` é RGBA
    /// (0..255) usado para pré-multiplicar o branco do fragment shader.
    /// `hscale` (0..1) controla a fração da meia-altura que a onda ocupa.
    ///
    /// Retorna `Vec<u8>` RGBA de `width x height` (ou `None` em erro).
    pub fn render_waveform(
        &self,
        width: u32,
        height: u32,
        mins: &[f32],
        maxs: &[f32],
        color: [u8; 4],
        hscale: f32,
    ) -> Option<Vec<u8>> {
        let count = mins.len();
        if count == 0 || count != maxs.len() || width == 0 || height == 0 {
            return None;
        }
        // A textura não pode exceder o limite de dimensão do dispositivo
        // (default 8192). Zoom muito aberto pode pedir larguras maiores;
        // limitamos e o `image-fit: fill` estica no clip (degradação suave).
        let max_dim = self.device.limits().max_texture_dimension_2d.max(1);
        let width = width.min(max_dim);
        let height = height.min(max_dim);
        let count = count as u32;
        let count = count.min(width); // nº de colunas não pode passar a largura real

        // Barras em storage buffer: vec2 (min, max) por coluna.
        // Apenas as primeiras `count` colunas (alinhadas à largura final).
        let bar_data: Vec<f32> = mins
            .iter()
            .zip(maxs.iter())
            .take(count as usize)
            .flat_map(|(&mn, &mx)| [mn.clamp(-1.0, 1.0), mx.clamp(-1.0, 1.0)])
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

        // Uniforms: count e hscale.
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::cast_slice(&[count as f32, hscale, 0.0f32, 0.0f32]),
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

        // Clear com a cor de fundo (alfa 0 = transparente no buffer).
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("waveform-pass"),
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

        // Shot da cor uma vez.
        let [cr, cg, cb, ca] = color;
        let c = [cr as f32, cg as f32, cb as f32, ca as f32];
        let c = [
            (c[0] * (ca as f32 / 255.0)) as u8,
            (c[1] * (ca as f32 / 255.0)) as u8,
            (c[2] * (ca as f32 / 255.0)) as u8,
            ca,
        ];

        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((bytes_per_row as usize) * height as usize);
        for row in 0..height as usize {
            let start = row * padded as usize;
            let row_src = &data[start..start + bytes_per_row as usize];
            // O shader pinta branco puro; compomos a cor cliente por
            // multiplicação preservando o alpha do pixel.
            for px in row_src.chunks_exact(4) {
                let a = px[3];
                out.push((px[0] as u32 * c[0] as u32 / 255) as u8);
                out.push((px[1] as u32 * c[1] as u32 / 255) as u8);
                out.push((px[2] as u32 * c[2] as u32 / 255) as u8);
                out.push(a);
            }
        }
        drop(data);
        readback.unmap();

        Some(out)
    }
}
