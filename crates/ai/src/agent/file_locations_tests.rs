use std::path::PathBuf;

use warp_terminal::shell::{ShellLaunchData, ShellType};

use super::*;
use crate::agent::action_result::AnyFileContent;

fn file_context(file_name: &str) -> FileContext {
    FileContext::new(
        file_name.to_string(),
        AnyFileContent::StringContent(String::new()),
        None,
        None,
    )
}

fn wsl_shell() -> Option<ShellLaunchData> {
    Some(ShellLaunchData::WSL {
        distro: "Ubuntu".to_string(),
    })
}

fn msys2_shell() -> Option<ShellLaunchData> {
    Some(ShellLaunchData::MSYS2 {
        executable_path: PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
        shell_type: ShellType::Bash,
    })
}

/// Regression tests for the corrected root cause of APP-5438: a completed `read_files` result
/// can store `FileContext.file_name` in the host's native encoding (see
/// `host_native_absolute_path`), which must be normalized back to the shell's native form when
/// grouped for display.
#[test]
fn wsl_unc_file_name_renders_as_unix_path() {
    let files = [file_context(r"\\WSL$\Ubuntu\home\user\file.txt")];
    let cwd = Some("/home/user".to_string());
    assert_eq!(
        group_file_contexts_for_display(&files, wsl_shell().as_ref(), cwd.as_ref()),
        vec!["/home/user/file.txt".to_string()]
    );
}

#[test]
fn msys2_windows_file_name_renders_as_unix_path() {
    let files = [file_context(r"C:\Users\username\project\file.txt")];
    let cwd = Some("/c/Users/username".to_string());
    assert_eq!(
        group_file_contexts_for_display(&files, msys2_shell().as_ref(), cwd.as_ref()),
        vec!["/c/Users/username/project/file.txt".to_string()]
    );
}

#[test]
fn wsl_unc_file_name_from_a_different_distro_is_left_explicit() {
    // A stored UNC path naming a distro other than the session's own must not be rendered as if
    // it were a local path within the current session.
    let files = [file_context(r"\\WSL$\Debian\home\other\file.txt")];
    let cwd = Some("/home/user".to_string());
    assert_eq!(
        group_file_contexts_for_display(&files, wsl_shell().as_ref(), cwd.as_ref()),
        vec!["/WSL$/Debian/home/other/file.txt".to_string()]
    );
}

#[test]
fn native_windows_file_name_is_unaffected() {
    let files = [file_context(r"C:\home\user\file.txt")];
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        group_file_contexts_for_display(&files, None, cwd.as_ref()),
        vec![r"C:\home\user\file.txt".to_string()]
    );
}

#[test]
fn pending_and_complete_read_files_rendering_agree() {
    // The "pending" branch renders the request's shell-native location directly via
    // `FileLocations::to_user_message`, while the "complete" branch renders the stored
    // (potentially host-native) result via `group_file_contexts_for_display`. Both must agree
    // once the file has actually been read.
    let cwd = Some("/home/user".to_string());

    let pending = FileLocations {
        name: "/home/user/file.txt".to_string(),
        lines: vec![],
    }
    .to_user_message(wsl_shell().as_ref(), cwd.as_ref(), None);

    let complete = group_file_contexts_for_display(
        &[file_context(r"\\WSL$\Ubuntu\home\user\file.txt")],
        wsl_shell().as_ref(),
        cwd.as_ref(),
    );

    assert_eq!(complete, vec![pending.clone()]);
    assert_eq!(pending, "/home/user/file.txt");
}
