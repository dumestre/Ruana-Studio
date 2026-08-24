//! Mixer: extração de estado, modelos compartilhados entre a
//! janela principal e as flutuantes.

use daw_core::mixer::MixerParam;
use daw_core::project::Project;
use slint::Model;
use std::rc::Rc;

/// Máximo de slots de insert renderizados por canal no mixer.
pub const MAX_INSERTS: usize = 8;

/// Extrai o estado do mixer do projeto (trilhas + master) como arrays.
pub fn mixer_state(project: &Project) -> (Vec<bool>, Vec<bool>, Vec<bool>, Vec<f32>, Vec<f32>) {
    let n = project.tracks.len() + 1; // trilhas + master
    let mut mutes = Vec::with_capacity(n);
    let mut soloes = Vec::with_capacity(n);
    let mut arms = Vec::with_capacity(n);
    let mut volumes = Vec::with_capacity(n);
    let mut pans = Vec::with_capacity(n);

    for t in &project.tracks {
        mutes.push(t.mixer.muted);
        soloes.push(t.mixer.soloed);
        arms.push(t.mixer.armed);
        volumes.push(t.mixer.volume);
        pans.push(t.mixer.pan);
    }
    // Master
    mutes.push(project.master_mixer.muted);
    soloes.push(project.master_mixer.soloed);
    arms.push(project.master_mixer.armed);
    volumes.push(project.master_mixer.volume);
    pans.push(project.master_mixer.pan);

    (mutes, soloes, arms, volumes, pans)
}

/// Extrai o estado de inserts do mixer como arrays achatados.
///
/// Cada canal ocupa `MAX_INSERTS` entradas em `names`/`enabled`
/// (índice `canal * MAX_INSERTS + slot`). Retorna as contagens por
/// canal, nomes, estados e o maior nº de inserts entre todos os canais.
pub fn mixer_inserts_state(
    project: &Project,
) -> (Vec<i32>, Vec<slint::SharedString>, Vec<bool>, i32) {
    let n = project.tracks.len() + 1; // trilhas + master
    let mut counts = Vec::with_capacity(n);
    let mut names = vec![slint::SharedString::default(); n * MAX_INSERTS];
    let mut enabled = vec![false; n * MAX_INSERTS];
    let mut max_count = 0;
    for (channel, mixer) in project
        .tracks
        .iter()
        .map(|t| &t.mixer)
        .chain(std::iter::once(&project.master_mixer))
        .enumerate()
    {
        counts.push(mixer.inserts.len() as i32);
        max_count = max_count.max(mixer.inserts.len() as i32);
        for (slot, insert) in mixer.inserts.iter().enumerate() {
            let idx = channel * MAX_INSERTS + slot;
            names[idx] = insert.name.clone().into();
            enabled[idx] = insert.enabled;
        }
    }
    (counts, names, enabled, max_count)
}

/// VecModel do mixer COMPARTILHADOS entre a janela principal e as
/// flutuantes. Como todas as janelas apontam para os mesmos `Rc`,
/// um `set_row_data` por canal propaga para todas de uma vez — sem
/// recriar modelos a cada movimento de fader/knob (zero garbage).
pub struct MixerModels {
    pub mutes: Rc<slint::VecModel<bool>>,
    pub soloes: Rc<slint::VecModel<bool>>,
    pub arms: Rc<slint::VecModel<bool>>,
    pub volumes: Rc<slint::VecModel<f32>>,
    pub pans: Rc<slint::VecModel<f32>>,
}

impl MixerModels {
    pub fn from_project(project: &Project) -> Self {
        let (mutes, soloes, arms, volumes, pans) = mixer_state(project);
        Self {
            mutes: Rc::new(slint::VecModel::from(mutes)),
            soloes: Rc::new(slint::VecModel::from(soloes)),
            arms: Rc::new(slint::VecModel::from(arms)),
            volumes: Rc::new(slint::VecModel::from(volumes)),
            pans: Rc::new(slint::VecModel::from(pans)),
        }
    }

    /// Atualiza apenas o canal informado (propaga para todas as janelas).
    pub fn update_channel(&self, channel: usize, param: MixerParam, value: f32) {
        match param {
            MixerParam::Volume => self.volumes.set_row_data(channel, value.clamp(0.0, 1.0)),
            MixerParam::Pan => self.pans.set_row_data(channel, value.clamp(0.0, 1.0)),
            MixerParam::Muted => self.mutes.set_row_data(channel, value >= 0.5),
            MixerParam::Soloed => self.soloes.set_row_data(channel, value >= 0.5),
            MixerParam::Armed => self.arms.set_row_data(channel, value >= 0.5),
        }
    }
}

/// Aplica os modelos do mixer numa janela (principal ou flutuante).
macro_rules! apply_mixer_models {
    ($ui:expr, $m:expr) => {{
        $ui.set_mixer_mutes($m.mutes.clone().into());
        $ui.set_mixer_soloes($m.soloes.clone().into());
        $ui.set_mixer_arms($m.arms.clone().into());
        $ui.set_mixer_volumes($m.volumes.clone().into());
        $ui.set_mixer_pans($m.pans.clone().into());
    }};
}

pub(crate) use apply_mixer_models;
