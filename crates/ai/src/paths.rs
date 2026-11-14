use typed_path::{TypedPath, TypedPathBuf, WindowsPath};
use warp_errors::report_error;
use warp_terminal::shell::ShellLaunchData;
use warp_util::path::{
    convert_msys2_to_windows_native_path, convert_wsl_to_windows_host_path, msys2_exe_to_root,
};
use warpui_core::platform::OperatingSystem;

fn use_unix_paths(shell: Option<&ShellLaunchData>) -> bool {
    OperatingSystem::get().is_linux()
        || OperatingSystem::get().is_mac()
        || shell.is_some_and(|shell| {
            matches!(
                shell,
                ShellLaunchData::WSL { .. } | ShellLaunchData::MSYS2 { .. }
            )
        })
}

fn target_uses_unix_paths(
    shell: Option<&ShellLaunchData>,
    current_working_directory: &str,
) -> bool {
    if shell.is_some_and(|shell| {
        matches!(
            shell,
            ShellLaunchData::WSL { .. }
                | ShellLaunchData::MSYS2 { .. }
                | ShellLaunchData::DockerSandbox { .. }
        )
    }) {
        return true;
    }

    let current_working_directory = TypedPath::derive(current_working_directory);
    if current_working_directory.is_windows() {
        false
    } else if current_working_directory.is_absolute() {
        true
    } else {
        use_unix_paths(shell)
    }
}

pub fn join_paths(paths: &[&str], shell: Option<&ShellLaunchData>) -> String {
    let use_unix_paths = use_unix_paths(shell);

    let base_path = if use_unix_paths {
        TypedPathBuf::unix()
    } else {
        TypedPathBuf::windows()
    };
    paths
        .iter()
        .fold(base_path, |acc, path| acc.join(path))
        .to_string_lossy()
        .into_owned()
}

fn shell_native_absolute_path_internal(
    file_path: &str,
    current_working_directory: &str,
    use_unix_paths: bool,
) -> TypedPathBuf {
    let expanded_path = shellexpand::tilde(file_path).into_owned();

    let (cwd, file_path) = if use_unix_paths {
        (
            TypedPathBuf::from_unix(current_working_directory),
            TypedPath::unix(&expanded_path),
        )
    } else {
        (
            TypedPathBuf::from_windows(current_working_directory),
            TypedPath::windows(&expanded_path),
        )
    };
    cwd.join(file_path).normalize()
}

/// Returns the absolute path of the path in the shell's native format.
///
/// On Unix systems, this will always be Unix encoded paths. On Windows, this
/// will be a Windows encoded path unless the user is using WSL or Git Bash, in
/// which case Unix encoded paths will be used.
pub fn shell_native_absolute_path(
    file_path: &str,
    shell: Option<&ShellLaunchData>,
    current_working_directory: Option<&String>,
) -> String {
    let Some(cwd) = current_working_directory else {
        return shellexpand::tilde(file_path).into_owned();
    };
    shell_native_absolute_path_internal(file_path, cwd, use_unix_paths(shell))
        .to_string_lossy()
        .into_owned()
}

/// Returns an absolute path encoded for the target session for display.
pub fn shell_native_absolute_path_for_display(
    file_path: &str,
    shell: Option<&ShellLaunchData>,
    current_working_directory: Option<&String>,
) -> String {
    let Some(cwd) = current_working_directory else {
        return shellexpand::tilde(file_path).into_owned();
    };
    shell_native_absolute_path_internal(file_path, cwd, target_uses_unix_paths(shell, cwd))
        .to_string_lossy()
        .into_owned()
}

/// Returns the absolute path of the path in the host's native format.
///
/// This should be used over [`shell_native_absolute_path`] when we need an
/// absolute path in the format of the user's OS, regardless of what shell
/// they're using. e.g. A Windows encoded path when the user is using WSL.
pub fn host_native_absolute_path(
    file_path: &str,
    shell: &Option<ShellLaunchData>,
    current_working_directory: &Option<String>,
) -> String {
    let Some(cwd) = current_working_directory.as_ref() else {
        return shellexpand::tilde(file_path).into_owned();
    };
    let normalized_path =
        shell_native_absolute_path_internal(file_path, cwd, use_unix_paths(shell.as_ref()));

    match shell {
        Some(ShellLaunchData::WSL { distro }) => {
            match convert_wsl_to_windows_host_path(&normalized_path.to_path(), distro) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(err) => {
                    report_error!(
                        anyhow::Error::new(err).context("Could not convert WSL to Windows host path"),
                        extra: { "path" => ?normalized_path }
                    );
                    normalized_path.to_string_lossy().into_owned()
                }
            }
        }
        Some(ShellLaunchData::MSYS2 {
            executable_path, ..
        }) => {
            match convert_msys2_to_windows_native_path(
                &normalized_path.to_path(),
                &msys2_exe_to_root(WindowsPath::new(
                    executable_path.as_os_str().as_encoded_bytes(),
                )),
            ) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(err) => {
                    report_error!(
                        anyhow::Error::new(err)
                            .context("Could not convert MSYS2 to Windows host path"),
                        extra: { "path" => ?normalized_path }
                    );
                    normalized_path.to_string_lossy().into_owned()
                }
            }
        }
        _ => normalized_path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
