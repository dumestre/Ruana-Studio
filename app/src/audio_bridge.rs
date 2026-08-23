//! Ponte entre a UI (thread principal) e o motor de áudio.
//!
//! O motor abre o dispositivo no startup e fica aberto até o programa
//! fechar. Duas razões:
//!
//! - Abrir stream custa dezenas de milissegundos e estala no primeiro
//!   buffer; fazer isso a cada play é latência visível.
//! - Sem stream aberta não existe pré-escuta: o "arrasta e ouve" do
//!   channel rack precisa soar com o transporte PARADO.
//!
//! Toda mutação do documento que o áudio precisa ver só marca um flag
//! (`mark_dirty`). Um timer curto consolida e publica o snapshot uma
//! vez — arrastar um fader não pode reconstruir snapshot por frame.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use daw_core::project::Project;

/// Intervalo de consolidação do snapshot.
///
/// Curto o bastante para o disparo parecer imediato depois de soltar o
/// sample, longo o bastante para um gesto contínuo não republicar a
/// cada quadro.
const SYNC_INTERVAL: Duration = Duration::from_millis(90);

/// Handle da UI para o motor de áudio.
///
/// Clonável e barato. Se o dispositivo de áudio falhou ao abrir, o
/// handle interno é `None` e todos os métodos viram no-op — a UI
/// continua funcionando sem som, em vez de derrubar o programa.
#[derive(Clone)]
pub struct AudioBridge {
    handle: Option<daw_audio::EngineHandle>,
    dirty: Rc<Cell<bool>>,
}

impl AudioBridge {
    pub fn new(handle: Option<daw_audio::EngineHandle>) -> Self {
        Self {
            handle,
            dirty: Rc::new(Cell::new(true)),
        }
    }

    /// Sem áudio disponível?
    pub fn is_silent(&self) -> bool {
        self.handle.is_none()
    }

    /// Marca o snapshot como desatualizado. Barato: só liga um flag.
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// Publica o snapshot agora, se estiver sujo.
    pub fn flush(&self, project: &Project) {
        if !self.dirty.get() {
            return;
        }
        let Some(handle) = self.handle.as_ref() else {
            self.dirty.set(false);
            return;
        };
        self.dirty.set(false);
        handle.sync_project(project);
    }

    /// Publica o snapshot imediatamente, sujo ou não.
    pub fn flush_now(&self, project: &Project) {
        self.dirty.set(true);
        self.flush(project);
    }

    /// Toca um sample na hora (pré-escuta), independente do transporte.
    pub fn preview(&self, path: &str, volume: f32, pan: f32) {
        if path.is_empty() {
            return;
        }
        if let Some(handle) = self.handle.as_ref() {
            handle.preview_path(path, volume, pan);
        }
    }

    /// Decodifica um sample antecipadamente, sem tocar.
    ///
    /// Evita o custo do decode no primeiro disparo — que cairia dentro
    /// do gesto do usuário e apareceria como travada.
    pub fn preload(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        if let Some(handle) = self.handle.as_ref() {
            handle.preload_path(path);
        }
    }
}

/// Liga o timer que consolida os snapshots sujos.
///
/// Devolve o `Timer`; quem chama precisa mantê-lo vivo (um `Timer`
/// solto para de disparar ao ser dropado).
pub fn start_sync_timer(bridge: AudioBridge, project: Rc<RefCell<Project>>) -> slint::Timer {
    let timer = slint::Timer::default();
    if bridge.is_silent() {
        return timer;
    }
    timer.start(slint::TimerMode::Repeated, SYNC_INTERVAL, move || {
        // `try_borrow`: se o documento estiver emprestado no meio de
        // uma mutação, o flag continua ligado e o próximo tick publica.
        if let Ok(proj) = project.try_borrow() {
            bridge.flush(&proj);
        }
    });
    timer
}
