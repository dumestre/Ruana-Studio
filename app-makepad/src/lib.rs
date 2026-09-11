//! RuanaStudio (Makepad) — núcleo da UI.
//!
//! Fase 1 do port Slint → Makepad: shell com transport (play/stop/bpm/relógio)
//! e a timeline real (timeline-render) numa janela winit/wgpu dedicada.
//! O estado autoritativo (Project, Transport, AudioEngine) vive fora da UI.

pub mod app;