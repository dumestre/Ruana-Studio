//! Histórico de mutações do documento (undo/redo).
//!
//! Cada mutação do `Project` registra um `DocumentEvent`; a pilha
//! guarda a ordem das mudanças para desfazer/refazer. O próprio
//! evento carrega os dados de antes/depois necessários para
//! reverter (undo) ou reaplicar (redo) a operação.

use crate::DocumentEvent;

/// Pilha de eventos aplicados (undo) e desfeitos (redo).
#[derive(Debug, Default)]
pub struct History {
    undo_stack: Vec<DocumentEvent>,
    redo_stack: Vec<DocumentEvent>,
}

impl History {
    /// Registra uma mutação aplicada e descarta o redo.
    pub fn record(&mut self, event: DocumentEvent) {
        self.undo_stack.push(event);
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn pop_undo(&mut self) -> Option<DocumentEvent> {
        self.undo_stack.pop()
    }

    pub fn pop_redo(&mut self) -> Option<DocumentEvent> {
        self.redo_stack.pop()
    }

    pub fn push_undo(&mut self, event: DocumentEvent) {
        self.undo_stack.push(event);
    }

    pub fn push_redo(&mut self, event: DocumentEvent) {
        self.redo_stack.push(event);
    }
}
