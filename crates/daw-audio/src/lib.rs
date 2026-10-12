//! daw-audio — audio engine profissional.
//! Responsável por playback, mix stereo, e sincronização com o Transport.

#[cfg(feature = "cpal-out")]
pub mod backend;
pub mod buffer;
#[cfg(feature = "cpal-out")]
pub mod engine;
pub mod mapped_buffer;
pub mod meter;
pub mod mixer;
pub mod resample;
pub mod stretch;

#[cfg(feature = "cpal-out")]
pub use backend::{AudioConfig, AudioError, CpalBackend};
pub use buffer::{AudioBuffer, BufferCache};
#[cfg(feature = "cpal-out")]
pub use engine::{AudioEngine, EngineHandle, ProjectSnapshot, STEPS_PER_BEAT};
pub use mapped_buffer::AudioBufferKind;
pub use mapped_buffer::{MappedAudioBuffer, load_audio_buffer};
pub use meter::{FLOATS_PER_CHANNEL, MAX_METER_CHANNELS, MeterBank, MeterReading};
pub use mixer::{
    clamp_stereo, limit_stereo, mix_stereo_frame, mix_stereo_source, soft_clip, stereo_gains,
    stereo_gains_bipolar, track_is_active,
};
pub use resample::{resample_linear, resample_sinc};
pub use stretch::{clamp_ratio, frames_of, is_identity, stretch_interleaved};
