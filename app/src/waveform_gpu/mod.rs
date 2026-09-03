//! Waveform GPU Instanced Rendering Module
//!
//! Renderiza waveform "Bin Visual" (barras min→max) via OpenGL instanced rendering.
//! Um VAO/VBO global (pool), bufferSubData por clip, vertex shader expande quad.

pub mod instance_pool;
pub mod overlay;
pub mod renderer;
pub mod shader;

#[cfg(windows)]
pub mod win32_gdi;
#[cfg(target_os = "linux")]
pub mod linux_egl;

#[allow(unused_imports)]
pub use instance_pool::{InstancePool, ClipRange, WaveInstance};
#[allow(unused_imports)]
pub use renderer::{WaveformGpuRenderer, ClipBounds, GpuClipState};
#[allow(unused_imports)]
pub use shader::WaveformShader;