use std::io::Write;

use tempfile::NamedTempFile;

use crate::terminal::shell::ShellType;

/// Represents a temporary file used for bootstrap.
pub struct TempBootstrapFile {
    temp_file: NamedTempFile,
}

impl TempBootstrapFile {
    pub fn new<C>(builder: tempfile::Builder, contents: C) -> std::io::Result<Self>
    where
        C: AsRef<[u8]>,
    {
        let mut file = builder.tempfile()?;
        file.write_all(contents.as_ref())?;
        file.flush()?;
        Ok(Self { temp_file: file })
    }

    pub fn path_as_bytes(&self) -> Option<Vec<u8>> {
        use std::os::unix::ffi::OsStringExt;
        Some(
            self.temp_file
                .path()
                .to_path_buf()
                .into_os_string()
                .into_vec(),
        )
    }
}

impl TryFrom<NamedTempFile> for TempBootstrapFile {
    type Error = tempfile::PersistError;

    fn try_from(temp_file: NamedTempFile) -> Result<Self, Self::Error> {
        Ok(Self { temp_file })
    }
}

/// We don't currently create a permanent bootstrap file on Unix-like systems.
pub fn path_to_permanent_bootstrap_file(_shell_type: ShellType) -> Option<Vec<u8>> {
    None
}
