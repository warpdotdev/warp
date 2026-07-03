use super::*;

// Regression: `compute_fallback_shell` previously did
// `.expect("current user should exist")` on the `Option<User>` returned by
// `nix::unistd::User::from_uid`, which panicked when the current uid had no
// password-database entry (`Ok(None)`). It must now yield `None` and let the
// fallback-shell logic take over instead of crashing.
#[cfg(unix)]
#[test]
fn user_default_shell_path_handles_missing_passwd_entry() {
    assert_eq!(ShellStarter::user_default_shell_path(Ok(None)), None);
    assert_eq!(
        ShellStarter::user_default_shell_path(Err(nix::errno::Errno::ENOENT)),
        None
    );
}

#[cfg(unix)]
#[test]
fn user_default_shell_path_returns_shell_for_existing_user() {
    // When the user exists, their login shell path is returned unchanged.
    let user = nix::unistd::User::from_uid(nix::unistd::getuid())
        .expect("reading the current user should not fail on the test host");
    if let Some(user) = user {
        let expected = user.shell.display().to_string();
        assert_eq!(
            ShellStarter::user_default_shell_path(Ok(Some(user))),
            Some(expected)
        );
    }
}

#[test]
fn test_program_invalid_bash() {
    // This test assumes there is no bash binary at /some/weird/path/bash.
    let shell_path = "/some/weird/path/bash".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_invalid_zsh() {
    // This test assumes there is no bash zsh at /some/weird/path/bash.
    let shell_path = "/some/weird/path/zsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_unknown_shell() {
    let shell_path = "/some/weird/path/wtfsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_trim_wsl_err_from_output() {
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n".to_vec()),
        b"/bin/bash\n".to_vec()
    );
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n\r\0\n\0W\0A\0R\0N\0I\0N\0G\0".to_vec()),
        b"/bin/bash\n".to_vec()
    );
}
