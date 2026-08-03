#[cfg(feature = "local_fs")]
use std::io::Write;
use std::path::PathBuf;

use typed_path::TypedPath;
use warp_util::path::{WSLPathConversionError, convert_wsl_to_windows_host_path};

use crate::terminal::shell::ShellType;

/// Represents a temporary file used for bootstrap.
/// This is necessary because [`tempfile`] requires us to hold on to the file handle. However,
/// this causes Windows to prevent the shell from reading the file (since it is being used by
/// another process).
#[derive(Debug)]
pub struct TempBootstrapFile {
    file_path: FilePath,
}

#[derive(Debug)]
enum FilePath {
    Direct(PathBuf),
    Wsl(PathBuf),
}

#[derive(thiserror::Error, Debug)]
pub enum TempBootstrapFileError {
    #[error(transparent)]
    ConvertPath(#[from] WSLPathConversionError),
    #[error("could not create temporary file")]
    Create(std::io::Error),
    #[error("could not write bootstrap script to temporary file")]
    Write(std::io::Error),
    #[error("could not flush to temporary file")]
    Flush(std::io::Error),
    #[error("could not persist temporary file")]
    Persist(#[from] tempfile::PersistError),
}

impl TempBootstrapFile {
    pub fn new<C, S>(
        builder: tempfile::Builder,
        contents: C,
        wsl_distribution: Option<S>,
    ) -> Result<Self, TempBootstrapFileError>
    where
        C: AsRef<[u8]>,
        S: AsRef<str>,
    {
        let mut file = match &wsl_distribution {
            Some(distro_name) => {
                let wsl_temp_dir = convert_wsl_to_windows_host_path(
                    &TypedPath::from("/tmp"),
                    distro_name.as_ref(),
                )?;
                builder
                    .tempfile_in(wsl_temp_dir)
                    .map_err(TempBootstrapFileError::Create)?
            }
            None => builder.tempfile().map_err(TempBootstrapFileError::Create)?,
        };
        file.write_all(contents.as_ref())
            .map_err(TempBootstrapFileError::Write)?;
        file.flush().map_err(TempBootstrapFileError::Flush)?;

        // We persist the file here because there are issues with using NamedTempFile
        // as-is. If we hold on to the file handle without persisting, Windows
        // does not let the shell access the file.
        file.keep()
            .map(|(_file, path)| Self {
                file_path: match wsl_distribution.is_some() {
                    true => FilePath::Wsl(path),
                    false => FilePath::Direct(path),
                },
            })
            .map_err(TempBootstrapFileError::from)
    }

    pub fn path_as_bytes(&self) -> Option<Vec<u8>> {
        match &self.file_path {
            FilePath::Direct(path) => path
                .as_os_str()
                .to_str()
                .map(|path_str| path_str.as_bytes().to_vec()),
            FilePath::Wsl(path) => {
                let file_name = path
                    .file_name()
                    .map(|file_name| file_name.to_string_lossy())?;
                // We need to source the path from the Linux distribution's perspective here.
                let path = format!("/tmp/{file_name}");
                Some(path.as_bytes().to_vec())
            }
        }
    }
}

impl Drop for TempBootstrapFile {
    fn drop(&mut self) {
        if let Err(err) = match &self.file_path {
            FilePath::Direct(path) => std::fs::remove_file(path),
            FilePath::Wsl(path) => std::fs::remove_file(path),
        } {
            log::warn!("Unable to remove temporary bootstrap file: {err:?}");
        }
    }
}

/// Returns the path to the permanent bootstrap file in bytes, if it exists.
///
/// Currently we only create a permanent bootstrap file for PowerShell, located
/// alongside the Warp executable.
pub fn path_to_permanent_bootstrap_file(shell_type: ShellType) -> Option<Vec<u8>> {
    if shell_type != ShellType::PowerShell {
        return None;
    }

    let install_dir = crate::util::windows::install_dir().ok()?;
    let path = install_dir.join("pwsh.ps1");

    path.is_file()
        .then(|| path.as_os_str().to_str().map(|s| s.as_bytes().to_vec()))
        .flatten()
}
