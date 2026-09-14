//! Shader WGSL embutido (mesmo do app/src/timeline_gpu/wgpu.wgsl).
//!
//! Reaproveitado verbatim para garantir visual idêntico.

pub const SHADER_WGSL: &str = include_str!("timeline.wgsl");

/// Shader do pipeline de ícones (quads texturizados da edit toolbar).
pub const ICON_SHADER_WGSL: &str = include_str!("icon.wgsl");

/// Shader do pipeline de overlay (textura RGBA do Slint por cima da cena).
pub const OVERLAY_SHADER_WGSL: &str = include_str!("overlay.wgsl");
