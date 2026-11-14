use std::borrow::Cow;
use std::path::Path;

use typed_path::{TypedPath, TypedPathBuf, WindowsPath};
use warp_errors::report_error;
use warp_terminal::shell::ShellLaunchData;
use warp_util::path::{
    convert_msys2_to_windows_native_path, convert_windows_path_to_msys2,
    convert_windows_path_to_wsl, convert_wsl_to_windows_host_path, msys2_exe_to_root,
    parse_wsl_unc_path,
};
use warpui_core::platform::OperatingSystem;

/// The path separator style a path string appears to use, inferred from its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathStyle {
    Unix,
    Windows,
}

/// Infers the path style of `path` from its shape (e.g. a leading `/` or `~`
/// for Unix, or a drive letter like `C:\` for Windows). Returns `None` when
/// the shape is ambiguous, e.g. for a relative path with no separators.
fn detect_path_style(path: &str) -> Option<PathStyle> {
    if path.starts_with('/') || path.starts_with('~') {
        return Some(PathStyle::Unix);
    }

    let bytes = path.as_bytes();
    let has_drive_letter = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if has_drive_letter || path.starts_with('\\') {
        return Some(PathStyle::Windows);
    }

    None
}

/// Returns the first detectable [`PathStyle`] among `path_hints`, checked in order.
fn detect_path_style_from_hints<'a>(
    path_hints: impl IntoIterator<Item = &'a str>,
) -> Option<PathStyle> {
    path_hints.into_iter().find_map(detect_path_style)
}

/// Shells whose paths are always Unix-style, regardless of the local
/// operating system running Warp (or, on the web, the viewer's browser OS).
fn is_always_unix_shell(shell: Option<&ShellLaunchData>) -> bool {
    shell.is_some_and(|shell| {
        matches!(
            shell,
            ShellLaunchData::WSL { .. }
                | ShellLaunchData::MSYS2 { .. }
                | ShellLaunchData::DockerSandbox { .. }
        )
    })
}

/// Determines whether Unix path separators should be used to display or join paths.
///
/// This intentionally does not solely trust `operating_system`: on the web, the "local"
/// operating system is the viewer's browser OS (parsed from the user agent), which may not
/// match the OS of the shared session actually being displayed. Instead, when a path hint
/// (e.g. a cwd) is available, its shape is used to detect the session's native path style.
/// `operating_system` is only used as a fallback when no hint yields an unambiguous style,
/// e.g. no cwd is available, or it's a relative path with no separators.
fn use_unix_paths_impl<'a>(
    operating_system: OperatingSystem,
    shell: Option<&ShellLaunchData>,
    path_hints: impl IntoIterator<Item = &'a str>,
) -> bool {
    if is_always_unix_shell(shell) {
        return true;
    }

    match detect_path_style_from_hints(path_hints) {
        Some(PathStyle::Unix) => true,
        Some(PathStyle::Windows) => false,
        None => operating_system.is_linux() || operating_system.is_mac(),
    }
}

fn use_unix_paths<'a>(
    shell: Option<&ShellLaunchData>,
    path_hints: impl IntoIterator<Item = &'a str>,
) -> bool {
    use_unix_paths_impl(OperatingSystem::get(), shell, path_hints)
}

pub fn join_paths(paths: &[&str], shell: Option<&ShellLaunchData>) -> String {
    let use_unix_paths = use_unix_paths(shell, paths.iter().copied());

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

// Persisted `read_files` results can carry the host's native path encoding (see
// `host_native_absolute_path`); this normalizes such a path back to the shell's native form for
// display. Only used where a stored `FileContext.file_name` is being rendered — never on the
// path-for-execution path, since shape isn't provenance and a legitimate Unix filename can look
// Windows-shaped.
pub(crate) fn to_shell_native_display_path<'a>(
    path: &'a str,
    shell: Option<&ShellLaunchData>,
) -> Cow<'a, str> {
    match shell {
        Some(ShellLaunchData::WSL { distro }) => {
            if let Some(unc) = parse_wsl_unc_path(Path::new(path)) {
                if unc.distro.eq_ignore_ascii_case(distro) {
                    return Cow::Owned(unc.linux_path);
                }
                // A UNC path naming a different distro isn't a path within this session. Keep it
                // explicit (the `WSL$`/distro segments make it unmistakably not a real local
                // path) rather than rendering it as if it were local: swap in forward slashes so
                // it stays absolute and isn't joined onto the cwd like a relative path.
                return Cow::Owned(path.replace('\\', "/"));
            }
            if detect_path_style(path) == Some(PathStyle::Windows) {
                return Cow::Owned(convert_windows_path_to_wsl(path));
            }
            Cow::Borrowed(path)
        }
        Some(ShellLaunchData::MSYS2 { .. }) => {
            if detect_path_style(path) == Some(PathStyle::Windows) {
                return Cow::Owned(convert_windows_path_to_msys2(path));
            }
            Cow::Borrowed(path)
        }
        _ => Cow::Borrowed(path),
    }
}

fn shell_native_absolute_path_internal(
    file_path: &str,
    shell: Option<&ShellLaunchData>,
    current_working_directory: &str,
) -> TypedPathBuf {
    let expanded_path = shellexpand::tilde(file_path).into_owned();

    let use_unix_paths = use_unix_paths(shell, [current_working_directory, expanded_path.as_str()]);
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
/// will be a Windows encoded path unless the user is using WSL, Git Bash, or
/// a Docker sandbox, in which case Unix encoded paths will be used. The path
/// style is also inferred from the shape of `current_working_directory` (and
/// `file_path`) when possible, so that e.g. a shared session's paths render
/// correctly for a viewer whose local operating system doesn't match the
/// session's.
pub fn shell_native_absolute_path(
    file_path: &str,
    shell: Option<&ShellLaunchData>,
    current_working_directory: Option<&String>,
) -> String {
    let Some(cwd) = current_working_directory else {
        return shellexpand::tilde(file_path).into_owned();
    };
    shell_native_absolute_path_internal(file_path, shell, cwd)
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
    let normalized_path = shell_native_absolute_path_internal(file_path, shell.as_ref(), cwd);

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
