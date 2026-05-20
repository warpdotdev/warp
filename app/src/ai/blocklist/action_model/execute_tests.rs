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

/// Tests for the pure command builders behind [`is_file_path`] and
/// [`is_git_repository`]. These pin the shell-quoting guarantee documented
/// in #11132: an agent-supplied path containing shell metacharacters must
/// survive verbatim into the underlying `test -f` / `Test-Path` / `git -C`
/// invocation, regardless of the session's shell family.
mod path_quoting {
    use warp_util::path::ShellFamily;

    use super::super::{build_is_file_path_command, build_is_git_repository_command};

    #[test]
    fn is_file_path_posix_emits_shell_escaped_path() {
        // Plain path: nothing to escape; the path appears as-is after
        // `test -f`, separated by a single space.
        assert_eq!(
            build_is_file_path_command("/tmp/plain", ShellFamily::Posix),
            "test -f /tmp/plain"
        );
    }

    #[test]
    fn is_file_path_posix_escapes_spaces_and_metacharacters() {
        // `(`, `)`, `$`, `~`, and space are all shell-significant in POSIX
        // shells; `ShellFamily::Posix.shell_escape` backslash-escapes
        // each one so the command sees a single literal argument. `~`
        // also gets escaped here because the path doesn't START with `~`
        // (which would trigger the tilde-expansion preservation special
        // case in `shell_escape`).
        let command =
            build_is_file_path_command("/tmp/innocent$(touch ~/PROBE_RAN)", ShellFamily::Posix);
        assert_eq!(command, r"test -f /tmp/innocent\$\(touch\ \~/PROBE_RAN\)");
        // The shell must never see a bare `$(...)`, which would otherwise
        // execute `touch ~/PROBE_RAN` before `test -f` runs.
        assert!(!command.contains("$(touch"));
    }

    #[test]
    fn is_file_path_powershell_escapes_metacharacters_with_backticks() {
        // PowerShell uses backtick (\u{60}) as its escape character.
        let command =
            build_is_file_path_command("C:\\Users\\me\\My Stuff", ShellFamily::PowerShell);
        // The escaped path appears inside the if-test, and the shell sees
        // a single argument rather than splitting on space.
        assert!(command.starts_with("if (Test-Path -PathType Leaf "));
        assert!(command.contains("My`\u{20}Stuff"));
        assert!(command.ends_with("{ exit 0 } else { exit 1 }"));
    }

    #[test]
    fn is_git_repository_posix_escapes_path() {
        let command = build_is_git_repository_command("/tmp/my repo", ShellFamily::Posix);
        assert_eq!(command, r"git -C /tmp/my\ repo rev-parse");
    }

    #[test]
    fn is_git_repository_posix_escapes_command_substitution() {
        // The whole point of #11132: a path that *looks like* a backtick
        // command substitution stays a literal path argument to `git -C`.
        // Every backtick in the path is preceded by a backslash escape so
        // the shell parses the whole thing as a single word.
        let command = build_is_git_repository_command("/tmp/`rm -rf ~`/repo", ShellFamily::Posix);
        assert_eq!(command, r"git -C /tmp/\`rm\ -rf\ \~\`/repo rev-parse");
        // No bare backtick survives: every ` is preceded by a `\`.
        for (i, c) in command.char_indices() {
            if c == '`' {
                let prev = command[..i].chars().next_back();
                assert_eq!(
                    prev,
                    Some('\\'),
                    "unescaped backtick at byte {i}: {command}"
                );
            }
        }
    }

    #[test]
    fn is_git_repository_powershell_escapes_path() {
        let command =
            build_is_git_repository_command("C:\\Users\\me\\dev repo", ShellFamily::PowerShell);
        assert!(command.starts_with("git -C "));
        assert!(command.ends_with(" rev-parse"));
        // Space is escaped via backtick.
        assert!(command.contains("dev`\u{20}repo"));
    }

    #[test]
    fn empty_path_is_handled_safely() {
        // `ShellFamily::shell_escape("")` returns `''` (the POSIX literal
        // empty string). Make sure the builder doesn't produce an
        // empty positional that splits weirdly.
        let posix = build_is_file_path_command("", ShellFamily::Posix);
        assert_eq!(posix, "test -f ''");
        let pwsh = build_is_git_repository_command("", ShellFamily::PowerShell);
        assert_eq!(pwsh, "git -C '' rev-parse");
    }
}
