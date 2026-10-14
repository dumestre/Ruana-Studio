# Referência wgpu 30

> Referência do wgpu usado no RuanaStudio (30.0.x).

---

## Fontes oficiais

- **wgpu docs**: https://wgpu.rs/doc/wgpu/index.html
- **wgpu API**: https://wgpu.rs/doc/wgpu/api/index.html
- **wgpu examples**: https://wgpu.rs/examples/
- **Learn Wgpu**: https://sotrh.github.io/learn-wgpu/
- **WebGPU Fundamentals**: https://webgpufundamentals.org/
- **WGSL spec**: https://www.w3.org/TR/WGSL/

## Visão geral

wgpu é uma API gráfica cross-platform, segura, em Rust puro. Roda nativamente em Vulkan, Metal, D3D12 e OpenGL; e sobre WebGL2 e WebGPU em wasm.

Baseada no padrão WebGPU, mas é uma biblioteca Rust totalmente nativa e inclui muitas extensões específicas para backends nativos.

## Entry point

```rust
use wgpu::Instance;

let instance = Instance::default();
let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
    power_preference: wgpu::PowerPreference::HighPerformance,
    ..Default::default()
}).await.unwrap();
let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
    ..Default::default()
}).await.unwrap();
```

## Tipos principais

| Tipo | Descrição |
|---|---|
| `Instance` | Entry point. Cria `Adapter`, `Surface`. |
| `Adapter` | Handle para dispositivo físico. |
| `Device` | Handle lógico para GPU. Cria recursos. |
| `Queue` | Fila de comandos para GPU. |
| `Surface` | Superfície de apresentação (janela). |
| `Buffer` | Memória GPU (vertex, index, uniform, storage). |
| `Texture` | Imagem GPU. |
| `TextureView` | View de uma textura. |
| `BindGroup` | Grupo de recursos ligados a um pipeline. |
| `BindGroupLayout` | Layout de um bind group. |
| `PipelineLayout` | Layout de um pipeline. |
| `RenderPipeline` | Pipeline de renderização. |
| `ComputePipeline` | Pipeline de compute. |
| `ShaderModule` | Módulo de shader (WGSL). |
| `CommandEncoder` | Codificador de comandos GPU. |
| `RenderPass` | Pass de renderização em progresso. |

## Ciclo de vida de recursos

```rust
// 1. Criar superfície
let surface = instance.create_surface(&window).unwrap();

// 2. Configurar superfície
let config = wgpu::SurfaceConfiguration {
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    format: wgpu::TextureFormat::Bgra8UnormSrgb,
    width: size.width,
    height: size.height,
    present_mode: wgpu::PresentMode::Fifo,
    alpha_mode: wgpu::CompositeAlphaMode::Auto,
    view_formats: vec![],
    desired_maximum_frame_latency: 2,
};
surface.configure(&device, &config);

// 3. Loop de renderização
let frame = surface.get_current_texture().unwrap();
let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
{
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    // Draw calls aqui
}
queue.submit(std::iter::once(encoder.finish()));
frame.present();
```

## Shaders WGSL

```wgsl
// timeline.wgsl
struct RectInstance {
    position: vec2<f32>,
    size: vec2<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) instance: RectInstance) -> @builtin(position) vec4<f32> {
    let pos = (position * instance.size + instance.position) * viewport.scale + viewport.offset;
    return vec4<f32>(pos, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
```

## Coordenadas

wgpu usa o sistema de coordenadas de D3D e Metal. Profundidade varia de [0, 1].

## MSRV

wgpu 30 tem MSRV 1.87.

## Features

| Feature | Descrição |
|---|---|
| `strict_asserts` | Checks em runtime, mesmo em release |
| `serde` | Serialização via serde |
| `trace` | Captura de trace |

## Extensões

wgpu suporta extensões além do padrão WebGPU. Ver documentação para Features e especificações relevantes.

## Padrões do projeto

### Instancing

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct RectInstance {
    position: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
}

let instance_buffer = device.create_buffer_init(&wgpu::BufferInitDescriptor {
    label: Some("Instance Buffer"),
    contents: bytemuck::cast_slice(&instances),
    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
});
```

### Storage buffer para waveform

```rust
let waveform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("Waveform Data"),
    size: (num_samples * 4) as u64,
    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
});
```

## Links

- [[wgpu Timeline Rendering]]
- [[Referência Rust]]
- [[Arquitetura Geral]]
