//! daw-audio — audio engine profissional.
//! Responsável por playback, mix stereo, e sincronização com o Transport.

pub mod buffer;
pub mod resample;

pub use buffer::{BufferCache, SampleBuffer};
pub use resample::resample_linear;
