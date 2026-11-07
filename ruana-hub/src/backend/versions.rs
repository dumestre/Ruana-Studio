use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use zip::ZipArchive;

const R2_PUBLIC_HOST: &str = "builds.daime.space";
const R2_CHANNEL_PREFIX: &str = "/ruana/stable/";
pub const CHANNEL_MANIFEST_URL: &str = "https://builds.daime.space/ruana/stable/channel.json";
const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

const APP_EXECUTABLE: &str = if cfg!(windows) {
    "ruana-studio.exe"
} else {
    "ruana-studio"
};

#[derive(Debug, Deserialize)]
pub struct ChannelManifest {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub release_notes: String,
}

pub fn list(root: &Path) -> io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(root)?;
    let mut versions = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join(APP_EXECUTABLE).is_file() {
            versions.push(path);
        }
    }
    versions.sort();
    Ok(versions)
}

pub fn check_channel() -> io::Result<ChannelManifest> {
    let client = http_client()?;
    let response = client
        .get(CHANNEL_MANIFEST_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(network_error)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o manifesto do canal excede o tamanho máximo permitido",
        ));
    }
    let mut manifest_bytes = Vec::new();
    response
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)
        .map_err(network_error)?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o manifesto do canal excede o tamanho máximo permitido",
        ));
    }
    let manifest: ChannelManifest =
        serde_json::from_slice(&manifest_bytes).map_err(network_error)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn install_latest(root: &Path) -> io::Result<PathBuf> {
    let manifest = check_channel()?;
    install_manifest(root, &manifest, None)
}

pub fn update_latest(root: &Path, installed: &Path) -> io::Result<PathBuf> {
    let manifest = check_channel()?;
    install_manifest(root, &manifest, Some(installed))
}

pub fn launch(version: &Path) -> io::Result<()> {
    let executable = version.join(APP_EXECUTABLE);
    if !executable.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("executável não encontrado em {}", executable.display()),
        ));
    }
    Command::new(executable).current_dir(version).spawn()?;
    Ok(())
}

fn install_manifest(
    root: &Path,
    manifest: &ChannelManifest,
    installed: Option<&Path>,
) -> io::Result<PathBuf> {
    validate_manifest(manifest)?;
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let destination = root.join(format!("Ruana Studio {}", manifest.version));

    let current = match installed {
        Some(path) => {
            let path = path.canonicalize()?;
            if path.parent() != Some(root.as_path()) || !path.join(APP_EXECUTABLE).is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a versão selecionada não pertence à pasta gerenciada pelo Hub",
                ));
            }
            if destination.exists() && destination != path {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("a versão {} já está instalada", manifest.version),
                ));
            }
            Some(path)
        }
        None if destination.exists() => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "a versão {} já está instalada; atualize uma versão existente",
                    manifest.version
                ),
            ));
        }
        None => None,
    };

    let archive_path = root.join(format!(".ruana-download-{}.zip", std::process::id()));
    let archive_guard = TempFile::create(archive_path)?;
    download_archive(
        &manifest.download_url,
        &manifest.sha256,
        archive_guard.path(),
    )?;

    let staging = unique_staging_path(&root, "install")?;
    if let Err(error) = extract_archive(archive_guard.path(), &staging) {
        return Err(cleanup_staging_error(error, &staging));
    }
    if let Err(error) = validate_build_folder(&staging) {
        return Err(cleanup_staging_error(error, &staging));
    }
    if let Err(error) = write_version_metadata(&staging, manifest) {
        return Err(cleanup_staging_error(error, &staging));
    }

    match current {
        Some(current) => replace_version(&root, &current, &destination, &staging),
        None => {
            if let Err(error) = std::fs::rename(&staging, &destination) {
                return Err(cleanup_staging_error(error, &staging));
            }
            Ok(destination)
        }
    }
}

fn replace_version(
    root: &Path,
    current: &Path,
    destination: &Path,
    staging: &Path,
) -> io::Result<PathBuf> {
    let backup = unique_staging_path(root, "backup")?;
    if let Err(error) = std::fs::rename(current, &backup) {
        return Err(cleanup_staging_error(error, staging));
    }

    if let Err(error) = std::fs::rename(staging, destination) {
        if let Err(restore_error) = std::fs::rename(&backup, current) {
            return Err(io::Error::other(format!(
                "{error}; não foi possível restaurar a versão anterior: {restore_error}; a build temporária permanece em {} e o backup em {}",
                staging.display(),
                backup.display()
            )));
        }
        return Err(cleanup_staging_error(error, staging));
    }

    if let Err(error) = std::fs::remove_dir_all(&backup) {
        return Err(io::Error::other(format!(
            "a versão foi atualizada, mas não foi possível remover o backup em {}: {error}",
            backup.display()
        )));
    }
    Ok(destination.to_path_buf())
}

fn validate_manifest(manifest: &ChannelManifest) -> io::Result<()> {
    if manifest.version.is_empty()
        || manifest.version.len() > 64
        || !manifest
            .version
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        || !manifest.version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o canal publicou uma versão com formato inválido",
        ));
    }

    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o canal publicou um SHA-256 inválido",
        ));
    }

    validate_channel_url(&manifest.download_url)
}

fn validate_channel_url(url: &str) -> io::Result<()> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let valid_archive_path =
        parsed
            .path()
            .strip_prefix(R2_CHANNEL_PREFIX)
            .is_some_and(|file_name| {
                !file_name.is_empty()
                    && file_name.ends_with(".zip")
                    && file_name.len() <= 128
                    && file_name.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
                    })
            });

    if parsed.scheme() != "https"
        || parsed.host_str() != Some(R2_PUBLIC_HOST)
        || parsed.port().is_some()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
        || !valid_archive_path
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o canal só pode baixar pacotes ZIP por HTTPS do caminho público configurado no Cloudflare R2",
        ));
    }
    Ok(())
}

fn http_client() -> io::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(network_error)
}

fn download_archive(url: &str, expected_sha256: &str, path: &Path) -> io::Result<()> {
    validate_channel_url(url)?;
    let mut response = http_client()?
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(network_error)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a build excede o tamanho máximo permitido para download",
        ));
    }

    let mut file = File::create(path)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response.read(&mut buffer).map_err(network_error)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > MAX_DOWNLOAD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a build excede o tamanho máximo permitido para download",
            ));
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut file, &buffer[..read])?;
    }
    std::io::Write::flush(&mut file)?;
    let actual_sha256 = format!("{:x}", hasher.finalize());
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o SHA-256 da build não corresponde ao manifesto; instalação cancelada",
        ));
    }
    Ok(())
}

fn extract_archive(archive_path: &Path, destination: &Path) -> io::Result<()> {
    std::fs::create_dir(destination)?;
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(invalid_archive)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "o pacote contém arquivos demais",
        ));
    }

    let mut unpacked_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(invalid_archive)?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "o pacote contém um caminho de arquivo inválido",
            )
        })?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "o pacote contém links simbólicos não permitidos",
            ));
        }

        unpacked_bytes = unpacked_bytes.saturating_add(entry.size());
        if unpacked_bytes > MAX_UNPACKED_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "o conteúdo extraído excede o limite permitido",
            ));
        }

        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?;
        let expected_size = entry.size();
        let copied = std::io::copy(
            &mut entry.by_ref().take(expected_size + 1),
            &mut output_file,
        )?;
        if copied != expected_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "o conteúdo extraído não corresponde ao tamanho declarado no pacote",
            ));
        }
    }
    Ok(())
}

fn validate_build_folder(source: &Path) -> io::Result<()> {
    if !source.join(APP_EXECUTABLE).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("o pacote não contém {APP_EXECUTABLE} na raiz"),
        ));
    }
    Ok(())
}

fn write_version_metadata(folder: &Path, manifest: &ChannelManifest) -> io::Result<()> {
    let metadata = serde_json::json!({
        "version": manifest.version,
        "release_notes": manifest.release_notes,
    });
    std::fs::write(
        folder.join("ruana-version.json"),
        serde_json::to_vec_pretty(&metadata).map_err(network_error)?,
    )
}

fn unique_staging_path(root: &Path, operation: &str) -> io::Result<PathBuf> {
    let path = root.join(format!(".ruana-{operation}-{}", std::process::id()));
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("já existe uma operação incompleta em {}", path.display()),
        ));
    }
    Ok(path)
}

fn cleanup_staging_error(error: io::Error, staging: &Path) -> io::Error {
    match std::fs::remove_dir_all(staging) {
        Ok(()) => error,
        Err(cleanup_error) => io::Error::other(format!(
            "{error}; também não foi possível limpar a pasta temporária {}: {cleanup_error}",
            staging.display()
        )),
    }
}

fn invalid_archive(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("pacote ZIP inválido: {error}"),
    )
}

fn network_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("erro no canal de builds: {error}"))
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(path: PathBuf) -> io::Result<Self> {
        File::options().write(true).create_new(true).open(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn rejects_manifest_paths_and_hashes_that_are_not_safe() {
        let good = ChannelManifest {
            version: "0.2.0-beta.3".to_owned(),
            download_url:
                "https://builds.daime.space/ruana/stable/RuanaStudio-0.2.0-beta.3-windows-x64.zip"
                    .to_owned(),
            sha256: "a".repeat(64),
            release_notes: String::new(),
        };
        assert!(validate_manifest(&good).is_ok());

        let mut bad_url = good;
        bad_url.download_url = "http://builds.daime.space/ruana/stable/build.zip".to_owned();
        assert!(validate_manifest(&bad_url).is_err());
        bad_url.download_url = "https://evil.example/build.zip".to_owned();
        assert!(validate_manifest(&bad_url).is_err());

        let mut bad_version = bad_url;
        bad_version.download_url = "https://builds.daime.space/ruana/stable/build.zip".to_owned();
        bad_version.version = "..".to_owned();
        assert!(validate_manifest(&bad_version).is_err());

        bad_version.version = "0.2.0".to_owned();
        bad_version.download_url =
            "https://builds.daime.space/ruana/stable/../outside.zip".to_owned();
        assert!(validate_manifest(&bad_version).is_err());
    }

    #[test]
    fn installs_and_updates_a_portable_test_build() {
        let temp = std::env::temp_dir().join(format!("ruana-hub-install-{}", std::process::id()));
        let versions = temp.join("versions");
        let archive_path = temp.join("new-build.zip");
        let _ = std::fs::remove_dir_all(&temp);
        let old_install = versions.join("Ruana Studio 0.1.0");
        std::fs::create_dir_all(&old_install).unwrap();
        std::fs::write(old_install.join(APP_EXECUTABLE), b"version one").unwrap();
        write_test_archive(&archive_path, b"version two");

        let staging = versions.join(".ruana-install-test");
        extract_archive(&archive_path, &staging).unwrap();
        validate_build_folder(&staging).unwrap();
        let destination = versions.join("Ruana Studio 0.2.0");
        let installed = replace_version(&versions, &old_install, &destination, &staging).unwrap();
        assert_eq!(
            std::fs::read(installed.join(APP_EXECUTABLE)).unwrap(),
            b"version two"
        );
        assert!(!old_install.exists());
        assert!(installed.join("cursors").join("select.cur").is_file());

        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn archive_extraction_rejects_parent_directory_paths() {
        let temp = std::env::temp_dir().join(format!("ruana-hub-zip-slip-{}", std::process::id()));
        let archive_path = temp.join("malicious.zip");
        let destination = temp.join("unpacked");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let file = File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("../escaped.txt", SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"not allowed").unwrap();
        archive.finish().unwrap();

        assert!(extract_archive(&archive_path, &destination).is_err());
        assert!(!temp.join("escaped.txt").exists());

        std::fs::remove_dir_all(temp).unwrap();
    }

    fn write_test_archive(path: &Path, executable: &[u8]) {
        let file = File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        archive.start_file(APP_EXECUTABLE, options).unwrap();
        archive.write_all(executable).unwrap();
        archive.start_file("cursors/select.cur", options).unwrap();
        archive.write_all(b"cursor").unwrap();
        archive.finish().unwrap();
    }
}
