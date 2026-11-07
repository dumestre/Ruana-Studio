//! Temas: aplica a paleta selecionada e persiste o índice.
//!
//! As 7 paletas moram em `ui/theme.slint` (bindings do global
//! `Theme.active`). O Rust não tem acessor gerado do global, então
//! a aplicação passa pela property raiz `MainWindow.theme-index`
//! (o `changed` dela seta `Theme.active`). Aqui só validamos/clamp,
//! salvamos `theme.cfg` ao lado do executável e restauramos no boot.

use crate::MainWindow;

pub const THEME_COUNT: i32 = 7;

/// Índice do tema Paper (claro). `Theme.is-paper` no .slint é `active == 6`.
pub const THEME_PAPER: i32 = 6;

fn cfg_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("theme.cfg")))
        .unwrap_or_else(|| std::path::PathBuf::from("theme.cfg"))
}

/// Aplica o tema `index` na UI (clampa para o intervalo válido)
/// e devolve o índice efetivo.
pub fn apply(ui: &MainWindow, index: i32) -> i32 {
    let index = index.clamp(0, THEME_COUNT - 1);
    ui.set_theme_index(index);
    index
}

/// Persiste o índice selecionado.
pub fn save(index: i32) {
    let _ = std::fs::write(cfg_path(), index.to_string());
}

/// Lê o tema salvo (ou `None` quando não há arquivo / inválido).
pub fn load() -> Option<i32> {
    std::fs::read_to_string(cfg_path())
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
}