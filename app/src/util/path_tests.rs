use crate::util::extensions::TrimStringExt;

#[test]
fn test_trim_newline() {
    let mut string = "".to_string();
    string.trim_trailing_newline();
    assert_eq!("", string);

    let mut string = "test\n".to_string();
    string.trim_trailing_newline();
    assert_eq!("test", string);

    let mut string = "   test    \n".to_string();
    string.trim_trailing_newline();
    assert_eq!("   test    ", string);
}

/// TODO(CORE-3626): write an equivalent test with Windows paths.
#[cfg(not(windows))]
#[test]
fn test_resolve_command() {
    use crate::util::path::resolve_executable;
    use std::path::Path;

    assert_eq!(
        &resolve_executable("/bin/sh").unwrap(),
        Path::new("/bin/sh")
    );
    assert_eq!(
        &resolve_executable("env").unwrap(),
        Path::new("/usr/bin/env")
    );
    // This path exists in the Warp repo, so it should resolve. The `../`
    // is because Rust unit tests run from the root of the crate (`app` in
    // this case).
    assert_eq!(
        &resolve_executable("../script/run").unwrap(),
        Path::new("../script/run")
    );
    // `pwd` should always exist (it's also a shell builtin), but we won't
    // assume a specific location.
    assert!(resolve_executable("pwd").is_some());
    assert!(resolve_executable("unlikely-command").is_none());
    assert!(resolve_executable("nonexistent/relative/path").is_none());
    // src/main.rs does exist, but is not executable.
    assert!(resolve_executable("src/main.rs").is_none());
    // Note the trailing space.
    assert!(resolve_executable("zsh ").is_none());
}

#[cfg(windows)]
struct EnvVarGuard {
    key: &'static str,
    original: Option<std::ffi::OsString>,
}

#[cfg(windows)]
impl EnvVarGuard {
    fn set(key: &'static str, value: impl Into<std::ffi::OsString>) -> Self {
        let original = std::env::var_os(key);
        std::env::set_var(key, value.into());
        Self { key, original }
    }
}

#[cfg(windows)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            std::env::set_var(self.key, original);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(windows)]
#[test]
#[serial_test::serial]
fn test_resolve_command_uses_pathext() {
    use crate::util::path::resolve_executable_in_path;

    let temp_dir = tempfile::TempDir::new().unwrap();
    let command_path = temp_dir.path().join("codex.cmd");
    std::fs::write(&command_path, "@echo off\r\n").unwrap();

    let _pathext = EnvVarGuard::set("PATHEXT", ".CMD;.EXE");
    let resolved = resolve_executable_in_path("codex", temp_dir.path().as_os_str()).unwrap();
    assert_eq!(
        resolved.as_ref().to_string_lossy().to_ascii_lowercase(),
        command_path.to_string_lossy().to_ascii_lowercase()
    );
}
