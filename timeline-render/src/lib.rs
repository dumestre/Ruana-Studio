//! Timeline renderer — biblioteca + binário.
//!
//! Replica visualmente a timeline do app: régua, grid, clips, playhead e VU.
//! A `struct App` (módulo `app`) contém todo o estado e renderização (wgpu);
//! `clips`, `fades`, `waveform` e `tools` são submódulos de apoio.

pub mod app;
pub mod clips;
pub mod fades;
pub(crate) mod shctx;
pub mod tools;
pub mod waveform;

pub use app::App;

// Re-exports usados pelos submódulos `clips`/`fades` (que resolvem via
// `super::`) e por código externo que consuma a biblioteca.
pub use app::render::{
    build_text_instances,
    build_text_instances_font,
    darker,
    get_regular_font,
    palette,
    rgba,
    rgba_alpha,
    srgb_to_linear,
    OrthoCamera,
    RectInstance,
};

/// Ponto de entrada do binário: cria a `EventLoop` e roda o handler.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}