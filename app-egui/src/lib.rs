//! RuanaStudio (egui) — núcleo da UI.
//!
//! Port do shell Makepad → egui: shell com transport (play/stop/bpm/relógio)
//! e a timeline real (timeline-render) numa janela winit/wgpu dedicada.
//! O estado autoritativo (Project, Transport, AudioEngine) vive fora da UI.

pub mod app;
pub mod toolbar;