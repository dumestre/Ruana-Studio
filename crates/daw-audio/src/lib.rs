//! daw-audio — audio engine profissional.
//! Responsável por playback, mix stereo, e sincronização com o Transport.

#[cfg(feature = "cpal-out")]
pub mod backend;
pub mod buffer;
pub mod mapped_buffer;
#[cfg(feature = "cpal-out")]
pub mod engine;
pub mod meter;
pub mod mixer;
pub mod resample;

#[cfg(feature = "cpal-out")]
pub use backend::{AudioConfig, AudioError, CpalBackend};
pub use buffer::{AudioBuffer, BufferCache};
pub use mapped_buffer::AudioBufferKind;
#[cfg(feature = "cpal-out")]
pub use engine::{AudioEngine, EngineHandle, ProjectSnapshot, STEPS_PER_BEAT};
pub use mapped_buffer::{load_audio_buffer, MappedAudioBuffer};
pub use meter::{MeterBank, MeterReading, FLOATS_PER_CHANNEL, MAX_METER_CHANNELS};
pub use mixer::{
    clamp_stereo, limit_stereo, mix_stereo_frame, mix_stereo_source, soft_clip, stereo_gains,
    stereo_gains_bipolar, track_is_active,
};
pub use resample::{resample_linear, resample_sinc};
