//! Timeline GPU Rendering Module — wgpu (headless/offscreen).
//!
//! Motor da timeline conforme AGENST.md: câmera ortográfica 2D, instancing
//! render (um quad base expandido por instância) e hit-testing separado em
//! CPU. A imagem é rasterizada numa textura offscreen + readback e exibida
//! pelo Slint como `slint::Image` (único padrão que funciona — Slint/Skia
//! não compartilha surface com wgpu, ver `crate::waveform_gpu`).

pub mod wgpu;

#[allow(unused_imports)]
pub use wgpu::{OrthoCamera, RectInstance, TimelineGpuRenderer};