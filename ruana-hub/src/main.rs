slint::include_modules!();

mod backend;

use backend::versions;
use std::path::Path;
use std::path::PathBuf;

fn main() -> Result<(), slint::PlatformError> {
    let ui = HubWindow::new()?;
    let paths = match backend::HubPaths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            ui.set_status(format!("Não foi possível inicializar o Hub: {error}").into());
            ui.run()?;
            return Ok(());
        }
    };

    if let Err(error) = paths.ensure() {
        ui.set_status(format!("Falha ao preparar os diretórios do Hub: {error}").into());
    }

    refresh_versions(&ui, &paths.versions_dir);

    {
        let ui_weak = ui.as_weak();
        let versions_dir = paths.versions_dir.clone();
        ui.on_install_test_version_requested(move || {
            start_channel_install(ui_weak.clone(), versions_dir.clone(), None);
        });
    }

    {
        let ui_weak = ui.as_weak();
        let versions_dir = paths.versions_dir.clone();
        ui.on_update_test_version_requested(move |installed| {
            start_channel_install(
                ui_weak.clone(),
                versions_dir.clone(),
                Some(PathBuf::from(installed.as_str())),
            );
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_launch_version_requested(move |path| {
            match versions::launch(Path::new(path.as_str())) {
                Ok(()) => set_status(&ui_weak, "Ruana Studio iniciado.".to_owned()),
                Err(error) => set_status(
                    &ui_weak,
                    format!("Não foi possível iniciar a versão: {error}"),
                ),
            }
        });
    }

    ui.run()
}

fn set_status(ui: &slint::Weak<HubWindow>, message: String) {
    if let Some(ui) = ui.upgrade() {
        ui.set_status(message.into());
    }
}

fn start_channel_install(
    ui: slint::Weak<HubWindow>,
    versions_dir: PathBuf,
    installed: Option<PathBuf>,
) {
    if let Some(window) = ui.upgrade() {
        if window.get_busy() {
            return;
        }
        window.set_busy(true);
    } else {
        return;
    }
    let operation = if installed.is_some() {
        "Baixando atualização de teste..."
    } else {
        "Consultando o canal e baixando a versão de teste..."
    };
    set_status(&ui, operation.to_owned());

    let ui_weak = ui.clone();
    let worker_versions_dir = versions_dir.clone();
    let spawn = std::thread::Builder::new()
        .name("ruana-hub-updater".to_owned())
        .spawn(move || {
            let result = match installed {
                Some(installed) => versions::update_latest(&worker_versions_dir, &installed),
                None => versions::install_latest(&worker_versions_dir),
            };
            let status = match &result {
                Ok(path) => format!("Build de teste instalada: {}", path.display()),
                Err(error) => format!("Falha na instalação da build: {error}"),
            };

            let dispatch = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    refresh_versions(&ui, &worker_versions_dir);
                    ui.set_status(status.into());
                    ui.set_busy(false);
                }
            });
            if let Err(error) = dispatch {
                eprintln!(
                    "[ruana-hub] não foi possível publicar o resultado na interface: {error}"
                );
            }
        });
    if let Err(error) = spawn {
        set_status(&ui, format!("Não foi possível iniciar o download: {error}"));
        if let Some(window) = ui.upgrade() {
            window.set_busy(false);
        }
    }
}

fn refresh_versions(ui: &HubWindow, root: &Path) {
    match versions::list(root) {
        Ok(items) => {
            ui.set_versions(
                std::rc::Rc::new(slint::VecModel::from(
                    items
                        .into_iter()
                        .map(|path| HubVersion {
                            name: path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("Ruana Studio (teste)")
                                .into(),
                            path: path.to_string_lossy().into_owned().into(),
                        })
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
        }
        Err(error) => {
            ui.set_status(format!("Não foi possível carregar as versões: {error}").into())
        }
    }
}
