//! Estado do mixer.
//!
//! Cada trilha carrega um `MixerChannel` (parâmetros do canal) e
//! o projeto tem um canal master. A UI projeta esse estado; o
//! valor autoritativo vive aqui (UI-agnostic).

/// Parâmetro de canal que pode ser alterado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerParam {
    Volume,
    Pan,
    Muted,
    Soloed,
    Armed,
}

impl MixerParam {
    pub fn as_str(self) -> &'static str {
        match self {
            MixerParam::Volume => "volume",
            MixerParam::Pan => "pan",
            MixerParam::Muted => "muted",
            MixerParam::Soloed => "soloed",
            MixerParam::Armed => "arm",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "volume" => Some(MixerParam::Volume),
            "pan" => Some(MixerParam::Pan),
            "muted" => Some(MixerParam::Muted),
            "soloed" => Some(MixerParam::Soloed),
            "arm" => Some(MixerParam::Armed),
            _ => None,
        }
    }
}

/// Canal do mixer.
#[derive(Debug, Clone, PartialEq)]
pub struct MixerChannel {
    /// Ganho linear 0..1 (0.8 ≈ 0 dB no display da UI).
    pub volume: f32,
    /// Panorama 0..1 (0.5 = centro).
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub armed: bool,
}

impl Default for MixerChannel {
    fn default() -> Self {
        Self {
            volume: 0.8,
            pan: 0.5,
            muted: false,
            soloed: false,
            armed: false,
        }
    }
}

impl MixerChannel {
    /// Aplica um parâmetro com clamp (volume/pan em 0..1).
    pub fn set(&mut self, param: MixerParam, value: f32) {
        match param {
            MixerParam::Volume => self.volume = value.clamp(0.0, 1.0),
            MixerParam::Pan => self.pan = value.clamp(0.0, 1.0),
            MixerParam::Muted => self.muted = value >= 0.5,
            MixerParam::Soloed => self.soloed = value >= 0.5,
            MixerParam::Armed => self.armed = value >= 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let ch = MixerChannel::default();
        assert_eq!(ch.volume, 0.8);
        assert_eq!(ch.pan, 0.5);
        assert!(!ch.muted);
        assert!(!ch.soloed);
        assert!(!ch.armed);
    }

    #[test]
    fn set_clamps_and_bools() {
        let mut ch = MixerChannel::default();
        ch.set(MixerParam::Volume, 2.0);
        assert_eq!(ch.volume, 1.0);
        ch.set(MixerParam::Pan, -1.0);
        assert_eq!(ch.pan, 0.0);
        ch.set(MixerParam::Muted, 1.0);
        assert!(ch.muted);
        ch.set(MixerParam::Soloed, 0.0);
        assert!(!ch.soloed);
        ch.set(MixerParam::Armed, 1.0);
        assert!(ch.armed);
    }

    #[test]
    fn from_str_maps() {
        assert_eq!(MixerParam::from_str("volume"), Some(MixerParam::Volume));
        assert_eq!(MixerParam::from_str("pan"), Some(MixerParam::Pan));
        assert_eq!(MixerParam::from_str("muted"), Some(MixerParam::Muted));
        assert_eq!(MixerParam::from_str("soloed"), Some(MixerParam::Soloed));
        assert_eq!(MixerParam::from_str("arm"), Some(MixerParam::Armed));
        assert_eq!(MixerParam::from_str("wat"), None);
    }
}
