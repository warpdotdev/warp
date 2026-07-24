use super::*;

/// The WSL UNC rewrite must replay an explicit `env_remove("PATH")` onto the
/// replacement command rather than dropping it together with the skipped
/// `PATH` values; otherwise the spawned `wsl.exe` silently inherits the
/// parent's Windows `PATH`.
#[test]
fn wsl_unc_translation_preserves_path_removal() {
    use std::ffi::OsString;
    use std::path::Path;

    let mut cmd = Command::new("git");
    cmd.arg("status");
    cmd.current_dir(Path::new(r"\\wsl$\Ubuntu\home\user\repo"));
    cmd.env("GIT_AUTHOR_NAME", "test");
    cmd.env_remove("PATH");

    cmd.apply_wsl_unc_translation();

    assert_eq!(cmd.inner.get_program(), "wsl.exe");
    let envs: Vec<(OsString, Option<OsString>)> = cmd
        .inner
        .get_envs()
        .map(|(key, value)| (key.to_owned(), value.map(|value| value.to_owned())))
        .collect();
    assert!(envs.contains(&(OsString::from("PATH"), None)));
    assert!(envs.contains(&(
        OsString::from("GIT_AUTHOR_NAME"),
        Some(OsString::from("test"))
    )));
}
