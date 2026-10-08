pub mod versions;

use std::io;
use std::path::PathBuf;

pub struct HubPaths {
    pub versions_dir: PathBuf,
}

impl HubPaths {
    pub fn discover() -> io::Result<Self> {
        let local_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .ok_or_else(|| io::Error::other("diretório de dados do usuário indisponível"))?;
        let hub_dir = local_data.join("RuanaStudio").join("Hub");
        Ok(Self {
            versions_dir: hub_dir.join("versions"),
        })
    }

    pub fn ensure(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.versions_dir)
    }
}
