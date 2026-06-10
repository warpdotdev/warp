mod binary_detection {
    use std::io::Write as _;

    use async_io::block_on;
    use tempfile::TempDir;

    use super::super::{is_file_content_binary_async, should_read_as_binary};

    fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        file.flush().expect("flush temp file");
        path
    }

    #[test]
    fn text_file_with_known_extension_is_not_binary() {
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "script.sh", b"#!/usr/bin/env bash\necho hi\n");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn binary_file_with_known_extension_is_binary() {
        let dir = TempDir::new().expect("create tempdir");
        // Known binary extension — should be classified as binary without
        // needing content inspection.
        let path = write_file(&dir, "image.png", b"not really a png but extension wins\n");
        assert!(block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_shell_script_is_not_binary() {
        // Regression test for QUALITY-507: an extensionless shell script (e.g.
        // `script/linux/bundle`) was being classified as binary solely because
        // its basename isn't in the known extensionless-text allow-list.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(
            &dir,
            "bundle",
            b"#!/usr/bin/env bash\n#\n# Builds a Warp binary and bundles it up for distribution.\n\nset -e\n",
        );
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_binary_content_is_binary() {
        // An extensionless file whose contents are actually binary should fall
        // through the content-based check and be classified as binary.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(
            &dir,
            "payload",
            // NUL byte is a strong binary signal for content_inspector.
            &[0u8, 1, 2, 3, b'A', 0, 0, 0, 0xFF, 0xFE, 0xFD],
        );
        assert!(block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn extensionless_text_allowlisted_is_not_binary() {
        // Files whose basenames are in the known text allow-list (e.g. README)
        // should take the fast path and skip content inspection.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "README", b"Hello, world!\n");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn empty_extensionless_file_is_not_binary() {
        // `content_inspector` treats an empty buffer as text, which is the
        // desired behavior for `read_files`: an empty file should be
        // surfaced to the agent as an empty string, not as zero binary bytes.
        let dir = TempDir::new().expect("create tempdir");
        let path = write_file(&dir, "empty", b"");
        assert!(!block_on(should_read_as_binary(&path)));
    }

    #[test]
    fn missing_extensionless_file_is_classified_as_binary() {
        // When an extensionless file cannot be opened during content
        // inspection, `should_read_as_binary` must route to the binary path
        // so the binary reader can produce a consistent `Missing` result.
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("does-not-exist");
        assert!(block_on(should_read_as_binary(&missing)));
    }

    #[test]
    fn missing_file_helper_is_classified_as_binary() {
        // Direct coverage of the low-level helper: opening a non-existent
        // path must return `true` so the caller doesn't accidentally try the
        // text path on an unreadable file.
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("does-not-exist");
        assert!(block_on(is_file_content_binary_async(&missing)));
    }
}

mod path_shell_quoting {
    use super::super::{build_is_file_path_command, build_is_git_repository_command};
    use crate::terminal::shell::ShellType;

    #[test]
    fn is_file_path_quotes_posix_path_as_single_argument() {
        let command = build_is_file_path_command("/tmp/repo path/file.rs", ShellType::Bash);

        assert_eq!(command, "test -f '/tmp/repo path/file.rs'");
    }

    #[test]
    fn is_file_path_neutralizes_posix_substitutions() {
        let command =
            build_is_file_path_command("/tmp/x$(touch /tmp/warp-poc)`id`", ShellType::Bash);

        assert_eq!(command, "test -f '/tmp/x$(touch /tmp/warp-poc)`id`'");
    }

    #[test]
    fn is_file_path_neutralizes_embedded_quote_posix() {
        let command = build_is_file_path_command("/tmp/foo'; rm -rf ~; echo '", ShellType::Bash);

        assert_eq!(command, r#"test -f '/tmp/foo'"'"'; rm -rf ~; echo '"'"''"#);
    }

    #[test]
    fn is_file_path_quotes_powershell_path_as_single_argument() {
        let command =
            build_is_file_path_command(r#"C:\Users\me\file path.rs"#, ShellType::PowerShell);

        assert_eq!(
            command,
            r#"if (Test-Path -PathType Leaf 'C:\Users\me\file path.rs') { exit 0 } else { exit 1 }"#
        );
    }

    #[test]
    fn is_file_path_neutralizes_powershell_substitutions() {
        let command = build_is_file_path_command(
            r#"C:\tmp\x$(New-Item C:\poc)$env:USERPROFILE"#,
            ShellType::PowerShell,
        );

        assert_eq!(
            command,
            r#"if (Test-Path -PathType Leaf 'C:\tmp\x$(New-Item C:\poc)$env:USERPROFILE') { exit 0 } else { exit 1 }"#
        );
    }

    #[test]
    fn is_file_path_neutralizes_fish_embedded_quote() {
        let command = build_is_file_path_command("/tmp/owner's file", ShellType::Fish);

        assert_eq!(command, r"test -f '/tmp/owner\'s file'");
    }

    #[test]
    fn is_git_repository_quotes_posix_path_as_single_argument() {
        let command = build_is_git_repository_command("/tmp/repo path", ShellType::Zsh);

        assert_eq!(command, "git -C '/tmp/repo path' rev-parse");
    }

    #[test]
    fn is_git_repository_neutralizes_posix_substitutions() {
        let command =
            build_is_git_repository_command("/tmp/x$(curl evil.example)`id`", ShellType::Bash);

        assert_eq!(command, "git -C '/tmp/x$(curl evil.example)`id`' rev-parse");
    }

    #[test]
    fn is_git_repository_neutralizes_embedded_quote_posix() {
        let command =
            build_is_git_repository_command("/tmp/foo'; rm -rf ~; echo '", ShellType::Bash);

        assert_eq!(
            command,
            r#"git -C '/tmp/foo'"'"'; rm -rf ~; echo '"'"'' rev-parse"#
        );
    }

    #[test]
    fn is_git_repository_neutralizes_powershell_substitutions() {
        let command = build_is_git_repository_command(
            r#"C:\repo$(New-Item C:\poc)$env:USERPROFILE"#,
            ShellType::PowerShell,
        );

        assert_eq!(
            command,
            r#"git -C 'C:\repo$(New-Item C:\poc)$env:USERPROFILE' rev-parse"#
        );
    }
}
