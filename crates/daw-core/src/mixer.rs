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

/// Slot de insert (efeito/processador) encadeado no canal.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertSlot {
    /// Nome do processador (ex.: "Compressor").
    pub name: String,
    /// Se o slot está ativo (false = bypass).
    pub enabled: bool,
}

impl InsertSlot {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
        }
    }
}

/// Canal do mixer.
#[derive(Debug, Clone, PartialEq)]
pub struct MixerChannel {
    /// Ganho linear 0..1.
    ///
    /// O unity é 1.0. O fader da UI é posicionado em dB (-60..0), então
    /// o valor 0.8 aqui corresponde a -1,9 dB e o triângulo abre quase no
    /// topo mas não nele. Foi por isso que o default passou a ser 1.0:
    /// uma faixa nova tem de abrir no unity, e não numa posição que
    /// parece um arrasto de fader pela metade.
    pub volume: f32,
    /// Panorama 0..1 (0.5 = centro).
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub armed: bool,
    /// Cadeia de inserts (efeitos) do canal, em ordem de processamento.
    pub inserts: Vec<InsertSlot>,
}

impl Default for MixerChannel {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.5,
            muted: false,
            soloed: false,
            armed: false,
            inserts: Vec::new(),
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

    /// Adiciona um insert ao fim da cadeia.
    pub fn add_insert(&mut self, name: impl Into<String>) {
        self.inserts.push(InsertSlot::new(name));
    }

    /// Remove um insert por índice. Retorna `false` se fora do range.
    pub fn remove_insert(&mut self, index: usize) -> bool {
        if index < self.inserts.len() {
            self.inserts.remove(index);
            true
        } else {
            false
        }
    }

    /// Alterna o bypass de um insert. Retorna `false` se fora do range.
    pub fn toggle_insert(&mut self, index: usize) -> bool {
        if let Some(slot) = self.inserts.get_mut(index) {
            slot.enabled = !slot.enabled;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let ch = MixerChannel::default();
        // Unity: a UI posiciona o fader em dB, e um default abaixo de
        // 1.0 abriria a faixa nova quase no topo sem estar nele.
        assert_eq!(ch.volume, 1.0);
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

    #[test]
    fn inserts_start_empty() {
        let ch = MixerChannel::default();
        assert!(ch.inserts.is_empty());
    }

    #[test]
    fn add_remove_toggle_inserts() {
        let mut ch = MixerChannel::default();
        ch.add_insert("Compressor");
        ch.add_insert("EQ");
        assert_eq!(ch.inserts.len(), 2);
        assert!(ch.inserts[0].enabled);
        assert_eq!(ch.inserts[0].name, "Compressor");

        assert!(ch.toggle_insert(0));
        assert!(!ch.inserts[0].enabled);
        assert!(!ch.toggle_insert(9));

        assert!(ch.remove_insert(0));
        assert_eq!(ch.inserts.len(), 1);
        assert_eq!(ch.inserts[0].name, "EQ");
        assert!(!ch.remove_insert(5));
    }
}
