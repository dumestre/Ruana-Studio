//! daw-audio — audio engine profissional.
//! Responsável por playback, mix stereo, e sincronização com o Transport.

#[cfg(feature = "cpal-out")]
pub mod backend;
pub mod buffer;
#[cfg(feature = "cpal-out")]
pub mod engine;
pub mod mixer;
pub mod resample;

#[cfg(feature = "cpal-out")]
pub use backend::{AudioConfig, AudioError, CpalBackend};
pub use buffer::{BufferCache, SampleBuffer};
#[cfg(feature = "cpal-out")]
pub use engine::{AudioEngine, ProjectSnapshot};
pub use mixer::{clamp_stereo, mix_stereo_frame, stereo_gains, track_is_active};
pub use resample::resample_linear;
