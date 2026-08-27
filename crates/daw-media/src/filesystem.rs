use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_directory: bool,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub extension: String,
}

/// Lista drives disponíveis (Windows: "C:\\", "D:\\", etc.; Unix: "/")
pub fn list_drives() -> io::Result<Vec<FileEntry>> {
    let mut drives = Vec::new();
    
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::GetLogicalDrives;
        
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26 {
            if mask & (1 << i) != 0 {
                let letter = (b'A' + i) as char;
                let path = format!("{}:\\", letter);
                let path_buf = PathBuf::from(&path);
                if let Ok(metadata) = fs::metadata(&path_buf) {
                    drives.push(FileEntry {
                        name: format!("{} ({})", letter, drive_type_name(&path_buf)),
                        path: path_buf.clone(),
                        is_directory: true,
                        size: 0,
                        modified: metadata.modified().ok(),
                        extension: String::new(),
                    });
                }
            }
        }
    }
    
    #[cfg(not(windows))]
    {
        // Unix: root filesystem
        let root = PathBuf::from("/");
        if let Ok(metadata) = fs::metadata(&root) {
            drives.push(FileEntry {
                name: "Computer".to_string(),
                path: root,
                is_directory: true,
                size: 0,
                modified: metadata.modified().ok(),
                extension: String::new(),
            });
        }
    }
    
    Ok(drives)
}

#[cfg(windows)]
fn drive_type_name(path: &Path) -> &'static str {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    
    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
    let drive_type = unsafe { GetDriveTypeW(windows::core::PCWSTR(wide.as_ptr())) };
    
    match drive_type {
        2 => "Removível",
        3 => "Local",
        4 => "Rede",
        5 => "CD/DVD",
        6 => "RAM",
        _ => "Desconhecido",
    }
}

pub fn list_directory(path: impl AsRef<Path>) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.clone(),
            is_directory: metadata.is_dir(),
            size: if metadata.is_dir() { 0 } else { metadata.len() },
            modified: metadata.modified().ok(),
            extension: ext,
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
