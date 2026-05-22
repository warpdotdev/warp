#[cfg_attr(unix, path = "unix.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod imp;

pub use imp::TempBootstrapFile;

use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::shell::ShellType;

/// Creates a `NamedTempFile` with the given bootstrap contents
///
/// Return `None` if any part of the operation fails
#[cfg(feature = "local_fs")]
pub fn create_bootstrap_file<C, S>(
    contents: C,
    shell_type: ShellType,
    _wsl_distribution: Option<S>,
) -> Option<TempBootstrapFile>
where
    C: AsRef<[u8]>,
    S: AsRef<str>,
{
    let mut builder = tempfile::Builder::new();
    // PowerShell will only source a file with the "ps1" extension.
    if shell_type == ShellType::PowerShell {
        builder.suffix(".ps1");
    }

    match TempBootstrapFile::new(
        builder,
        contents,
        #[cfg(windows)]
        _wsl_distribution,
    ) {
        Ok(bootstrap_file) => Some(bootstrap_file),
        Err(err) => {
            log::warn!("Error when creating temporary bootstrap file: {err:#}");
            None
        }
    }
}

/// Checks to see if we should be using a permanent bootstrap file given the
/// session info. If so, returns a path to the permanent bootstrap file for the
/// given shell type in bytes, if it exists.
pub fn permanent_bootstrap_file(
    shell_type: ShellType,
    pending_session_info: &SessionInfo,
) -> Option<Vec<u8>> {
    if pending_session_info.session_type != BootstrapSessionType::Local {
        return None;
    }

    imp::path_to_permanent_bootstrap_file(shell_type)
}
