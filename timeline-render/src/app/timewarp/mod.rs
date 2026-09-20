//! Timewarp: editor de tempo estilo Cubase (eixo Y = BPM) na área inferior
//! expandida, espelhando o piano roll (régua idêntica, grade só vertical,
//! sem teclado e sem grid horizontal).
//!
//! Submódulos:
//! - [`data`]: tipos e geometria (`TimewarpMetrics`, pontos, gutter de BPM).
//! - [`render`]: instâncias (régua reutilizada, corpo, curva, marcadores).
//! - [`interaction`]: hover, arrasto de pontos, scrub na régua.

mod data;
mod interaction;
mod render;

pub use data::{
    TimewarpMetrics, TempoPoint, BPM_LABEL_W, BPM_MAX, BPM_MIN, TIMEWARP_RULER_H,
};
pub use interaction::TimewarpInteraction;
pub use render::{build_timewarp_instances, build_timewarp_ruler_instances};