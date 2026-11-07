//! Waveform GPU Rendering Module — wgpu (headless/offscreen).
//!
//! Renderiza waveform "Bin Visual" (barras min→max) via wgpu numa
//! textura offscreen, faz readback RGBA e devolve uma imagem que o Slint
//! exibe dentro do clip. Cross-platform (Windows + Linux) sem hacks de
//! contexto OpenGL compartilhado.

pub mod wgpu;

#[allow(unused_imports)]
pub use wgpu::WgpuRenderer;