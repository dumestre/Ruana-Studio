//! Explorador de arquivos: navegação, filtro e projeção na UI.

use slint::Model;
use std::path::PathBuf;
use std::rc::Rc;

use crate::BrowserFileEntry;
use crate::MainWindow;
use crate::importer::AUDIO_EXTS;
use daw_media::filesystem::{list_directory, list_drives};
use slint::ComponentHandle;

/// Mostra um diretório no explorador.
///
/// `push_history` distingue navegar (empurra no histórico, cortando
/// o "avançar" pendente, como um navegador) de andar no histórico
/// (só troca o conteúdo; o índice é de quem chamou).
///
/// Retorna `false` se o caminho não é um diretório legível — assim o
/// botão de voltar não move o índice para uma pasta que sumiu.
pub fn show_explorer_dir(ui: &MainWindow, path: &std::path::Path, push_history: bool) -> bool {
    if !path.is_dir() {
        return false;
    }
    let text = slint::SharedString::from(path.to_string_lossy().as_ref());

    if push_history {
        let history: Vec<slint::SharedString> = ui.get_explorer_path_history().iter().collect();
        let idx = ui.get_explorer_history_index();
        // Corta o ramo à frente: navegar depois de voltar descarta o
        // "avançar", igual a um navegador.
        let keep = ((idx + 1).max(0) as usize).min(history.len());
        let mut new_history = history[..keep].to_vec();
        new_history.push(text.clone());
        ui.set_explorer_history_index(new_history.len() as i32 - 1);
        ui.set_explorer_path_history(Rc::new(slint::VecModel::from(new_history)).into());
    }

    ui.set_explorer_current_path(text);
    ui.set_explorer_can_go_up(path.parent().is_some_and(|p| p.is_dir()));
    let filter = ui.get_explorer_search_text();
    let entries = load_directory_entries(path, filter.as_str());
    ui.set_explorer_entries(Rc::new(slint::VecModel::from(entries)).into());
    true
}

/// Lista um diretório para o explorador.
///
/// `filter` vazio lista tudo; senão mantém só os nomes que contêm o
/// termo (case-insensitive). Só arquivos de áudio aparecem — qualquer
/// outro tipo é escondido — e diretórios sempre passam, para o usuário
/// continuar navegando enquanto filtra.
pub fn load_directory_entries(path: &std::path::Path, filter: &str) -> Vec<BrowserFileEntry> {
    let needle = filter.trim().to_lowercase();
    let Ok(entries) = list_directory(path) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| {
            let matches_needle =
                needle.is_empty() || e.name.to_lowercase().contains(&needle);
            if e.is_directory {
                matches_needle
            } else {
                let ext = e
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_uppercase();
                AUDIO_EXTS.contains(&ext.as_str()) && matches_needle
            }
        })
        .map(|e| {
            let entry_type = if e.is_directory {
                "Folder".to_string()
            } else {
                let ext = e
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_uppercase();
                // Todo formato que o decoder aceita mostra a extensão —
                // é isso que habilita o arraste no `file_browser`.
                if AUDIO_EXTS.contains(&ext.as_str()) {
                    ext
                } else {
                    "File".to_string()
                }
            };
            // Formatar data
            let modified_str = e.modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    let secs = d.as_secs();
                    // Conversão simples sem chrono: dias/segundos.
                    let total_secs = secs;
                    let days = total_secs / 86400;
                    let remaining = total_secs % 86400;
                    let hours = remaining / 3600;
                    let minutes = (remaining % 3600) / 60;
                    // Offset simplificado (não leva timezone em conta).
                    format!("{:02}/{:02}/1970 {:02}:{:02}", days % 31 + 1, (days / 31) % 12 + 1, hours, minutes)
                })
                .unwrap_or_default();
            BrowserFileEntry {
                name: e.name.clone().into(),
                entry_type: entry_type.into(),
                path: e.path.to_string_lossy().to_string().into(),
                is_directory: e.is_directory,
                size: e.size as i32,
                modified: modified_str.into(),
                extension: e.extension.clone().into(),
            }
        })
        .collect()
}

/// Assina navegação/histórico/busca do explorador.
pub fn wire_explorer(ui: &MainWindow) {
    // Filesystem: navegação de diretórios
    //
    // Uma função só faz o trabalho ("mostre esta pasta"); o que muda
    // entre os handlers é apenas se o histórico avança ou não.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_path_changed(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                show_explorer_dir(&ui, &PathBuf::from(path.as_str()), true);
            }
        });
    }

    // Filesystem: entrar em diretório (duplo papel do path-changed,
    // mantido separado porque a UI distingue clique em pasta de
    // navegação por caminho).
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_directory_entered(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                show_explorer_dir(&ui, &PathBuf::from(path.as_str()), true);
            }
        });
    }

    // Filesystem: subir um nível.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_go_up(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current = PathBuf::from(ui.get_explorer_current_path().as_str());
                if let Some(parent) = current.parent() {
                    show_explorer_dir(&ui, &parent.to_path_buf(), true);
                }
            }
        });
    }

    // Filesystem: voltar / avançar.
    //
    // Anda NO histórico em vez de empurrar nele — empurrar era o que
    // fazia o "voltar" virar um caminho novo e travar o botão.
    {
        let ui_weak = ui.as_weak();
        ui.on_explorer_history_go(move |delta| {
            if let Some(ui) = ui_weak.upgrade() {
                let history: Vec<slint::SharedString> =
                    ui.get_explorer_path_history().iter().collect();
                let target = ui.get_explorer_history_index() + delta;
                if target < 0 || target as usize >= history.len() {
                    return;
                }
                let path = PathBuf::from(history[target as usize].as_str());
                if show_explorer_dir(&ui, &path, false) {
                    ui.set_explorer_history_index(target);
                }
            }
        });
    }

    // Filesystem: filtro de busca — relê a pasta atual.
        {
            let ui_weak = ui.as_weak();
            ui.on_explorer_search_changed(move |term| {
                if let Some(ui) = ui_weak.upgrade() {
                    let current = PathBuf::from(ui.get_explorer_current_path().as_str());
                    let entries = load_directory_entries(&current, term.as_str());
                    ui.set_explorer_entries(Rc::new(slint::VecModel::from(entries)).into());
                }
            });
        }

        // Filesystem: ir para a raiz "Este Computador" (lista drives).
        {
            let ui_weak = ui.as_weak();
            ui.on_explorer_go_to_drives(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let drives = match list_drives() {
                        Ok(d) => d,
                        Err(_) => return,
                    };
                    let entries: Vec<BrowserFileEntry> = drives
                        .into_iter()
                        .map(|d| BrowserFileEntry {
                            name: d.name.into(),
                            entry_type: "Drive".into(),
                            path: d.path.to_string_lossy().to_string().into(),
                            is_directory: true,
                            size: 0,
                            modified: String::new().into(),
                            extension: String::new().into(),
                        })
                        .collect();
                    ui.set_explorer_entries(Rc::new(slint::VecModel::from(entries)).into());
                    ui.set_explorer_current_path("Este Computador".into());
                    ui.set_explorer_can_go_up(false);
                    // Limpa histórico — a raiz não tem "voltar"
                    ui.set_explorer_path_history(Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new())).into());
                    ui.set_explorer_history_index(-1);
                }
            });
        }
    }
