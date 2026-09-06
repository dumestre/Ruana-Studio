//! Clips da timeline: estruturas de dados, estado de hover/arrasto e
//! geração dos retângulos de render (corpo, header, waveform, fantasma e
//! overlay do clip em arrasto).
//!
//! O módulo foi subdivido em:
//! - [`data`]: tipos e geometria (`ClipDemo`, `ClipMetrics`, `clip_rect`).
//! - [`interaction`]: estado e lógica de hover/arrasto/marquee/draw/glue.
//! - [`render`]: geração de instâncias (`build_clip_instances` e helpers).

mod data;
mod interaction;
mod render;

pub use data::{clip_rect, ClipDemo, ClipMetrics};
pub use interaction::{ClipInteraction, DrawToolState};
pub use render::build_clip_instances;