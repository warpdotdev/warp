use crate::terminal::package_installers::command_at_cursor_has_common_package_installer_prefix;

#[test]
fn test_command_at_cursor_has_common_package_installer_prefix_basic_prefixes() {
    use warp_util::path::ShellFamily;

    // A representative subset of prefixes from is_common_package_installer_prefix
    let prefixes = vec![
        // Node ecosystem
        "npm install ",
        "pnpm add ",
        "yarn workspace ",
        "bun add ",
        "npx ",
        "npm uninstall ",
        "npm link ",
        "astro add ",
        "deno add ",
        // Python
        "pip install ",
        "pip3 install ",
        "python -m pip install ",
        "uv pip install ",
        // Poetry
        "poetry add ",
        // Ruby
        "gem install ",
        "bundle add ",
    ];

    for cmd in prefixes {
        let buffer = format!("{cmd}@");
        let at_index = buffer.rfind('@').expect("contains @");
        let is_pkg = command_at_cursor_has_common_package_installer_prefix(
            &buffer,
            at_index,
            ShellFamily::Posix,
            true,
            None,
        );
        assert!(
            is_pkg,
            "Expected installer prefix to be detected for: `{cmd}`"
        );
    }
}

#[test]
fn test_command_at_cursor_has_common_package_installer_prefix_with_alias_expansion() {
    use std::collections::HashMap;
    use std::sync::Arc;

    use typed_path::TypedPathBuf;
    use warp_completer::signatures::CommandRegistry;
    use warp_util::path::ShellFamily;
    use warpui::App;

    use crate::completer::SessionContext;
    use crate::terminal::model::session::{
        command_executor::testing::TestCommandExecutor, Session, SessionInfo,
    };

    App::test((), |app| async move {
        // Alias 'ya' expands to 'yarn add'
        let aliases = HashMap::from_iter([("ya".into(), "yarn add".to_string())]);
        let session = Session::new(
            SessionInfo::new_for_test().with_aliases(aliases),
            Arc::new(TestCommandExecutor::default()),
        );

        // Minimal working directory
        #[cfg(unix)]
        let cwd = TypedPathBuf::from("/");
        #[cfg(windows)]
        let cwd = TypedPathBuf::from_windows("C:\\");

        let session_ctx = app
            .read(|ctx| SessionContext::new(session, CommandRegistry::default().into(), cwd, ctx));

        let buffer = "ya @".to_string();
        let at_index = buffer.rfind('@').unwrap();
        let is_pkg = command_at_cursor_has_common_package_installer_prefix(
            &buffer,
            at_index,
            ShellFamily::Posix,
            /* is_alias_expansion_enabled */ false, // trigger internal alias expansion
            Some(&session_ctx),
        );
        assert!(
            is_pkg,
            "Expected alias-expanded installer prefix to be detected"
        );
    });
}

#[test]
fn test_command_at_cursor_has_common_package_installer_prefix_negative_cases() {
    use warp_util::path::ShellFamily;

    let cases = vec!["git add @", "echo @", "cargo run @"];

    for buffer in cases {
        let at_index = buffer.rfind('@').expect("contains @");
        let is_pkg = command_at_cursor_has_common_package_installer_prefix(
            buffer,
            at_index,
            ShellFamily::Posix,
            /* is_alias_expansion_enabled */ true,
            None,
        );
        assert!(
            !is_pkg,
            "Expected non-installer prefix to NOT be detected for: `{buffer}`"
        );
    }
}

#[test]
fn test_command_at_cursor_has_common_package_installer_prefix_multi_segment_commands() {
    use warp_util::path::ShellFamily;

    // Test cases with multi-segment commands and different cursor positions
    let test_cases = vec![
        // npm install && git add @[cursor] -> should be false (cursor in git add segment)
        ("npm install && git add @", false),
        // git add . && npm install @[cursor] -> should be true (cursor in npm install segment)
        ("git add . && npm install @", true),
        // git add @[cursor] && npm install @example/package -> should be false (cursor in git add segment)
        ("git add @ && npm install @example/package", false),
        // Test with various separators
        ("echo hello; npm install @", true),
        ("ls && pnpm add @", true),
        // Test with pipes
        ("cat file | npm install @", true),
        ("npm install | echo @", false),
    ];

    for (buffer, expected) in test_cases {
        let at_index = buffer.find('@').expect("contains @");
        let is_pkg = command_at_cursor_has_common_package_installer_prefix(
            buffer,
            at_index,
            ShellFamily::Posix,
            true,
            None,
        );
        assert_eq!(
            is_pkg, expected,
            "Expected {expected} for multi-segment command: `{buffer}`"
        );
    }
}
