use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_directory: bool,
}

pub fn list_directory(path: impl AsRef<Path>) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            is_directory: metadata.is_dir(),
        });
    }

    entries.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
            })
    });

    Ok(entries)
}

/// Retorna a pasta de samples padrão (documents/RuanaStudio/samples ou similar)
pub fn default_samples_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("RuanaStudio")
        .join("samples")
}
