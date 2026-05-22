use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::Arc;

use itertools::Itertools;
use typed_path::TypedPathBuf;
#[cfg(windows)]
use typed_path::{UnixComponent, WindowsComponent, WindowsPrefix};
use warp_completer::completer::{CompletionContext, EngineDirEntry, PathCompletionContext};
use warp_completer::signatures::CommandRegistry;
use warpui::App;

use crate::completer::SessionContext;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};
use crate::test_util::{Stub, VirtualFS};

fn test_session_context(session: Session, cwd: TypedPathBuf, app: &App) -> SessionContext {
    app.read(|ctx| SessionContext::new(session, CommandRegistry::default().into(), cwd, ctx))
}

fn working_directory() -> TypedPathBuf {
    #[cfg(unix)]
    let cwd = TypedPathBuf::from("/test/home/");
    #[cfg(windows)]
    let cwd = TypedPathBuf::from_windows(shellexpand::tilde("~").into_owned());

    cwd
}

#[test]
pub fn test_session_context_top_level_commands_includes_function_names() {
    App::test((), |app| async move {
        let function_names = vec![
            "my_func".into(),
            "foo".into(),
            "bar".into(),
            "foobar".into(),
        ];
        let session = Session::new(
            SessionInfo::new_for_test()
                .with_function_names(function_names.clone().into_iter().collect()),
            Arc::new(TestCommandExecutor::default()),
        );
        let ctx = test_session_context(session, working_directory(), &app);

        let top_level_commands = ctx.top_level_commands().collect_vec();
        for function_name in function_names.iter() {
            assert!(top_level_commands.contains(&function_name.as_str()));
        }
    });
}

#[test]
pub fn test_session_context_top_level_commands_includes_aliases() {
    App::test((), |app| async move {
        let aliases = HashMap::from_iter([
            ("first".into(), "test one".into()),
            ("second".into(), "first".into()),
            ("third".into(), "cd".into()),
            ("ls".into(), "ls -l".into()),
        ]);
        let session = Session::new(
            SessionInfo::new_for_test().with_aliases(aliases.clone()),
            Arc::new(TestCommandExecutor::default()),
        );
        let ctx = test_session_context(session, working_directory(), &app);

        let top_level_commands = ctx.top_level_commands().collect_vec();
        for alias in aliases.keys() {
            assert!(top_level_commands.contains(&alias.as_str()));
        }
    });
}

#[test]
pub fn test_session_context_top_level_commands_includes_abbreviations() {
    App::test((), |app| async move {
        let abbreviations = HashMap::from_iter([
            ("gl".into(), "git log".into()),
            ("gs".into(), "git status".into()),
        ]);
        let session = Session::new(
            SessionInfo::new_for_test().with_abbreviations(abbreviations.clone()),
            Arc::new(TestCommandExecutor::default()),
        );
        let ctx = test_session_context(session, working_directory(), &app);

        let top_level_commands = ctx.top_level_commands().collect_vec();
        for abbreviation in abbreviations.keys() {
            assert!(top_level_commands.contains(&abbreviation.as_str()));
        }
    });
}

#[test]
pub fn test_session_context_top_level_commands_includes_keywords() {
    App::test((), |app| async move {
        let keywords = vec!["while".into(), "foreach".into(), "repeat".into()];
        let session = Session::new(
            SessionInfo::new_for_test().with_keywords(keywords.clone()),
            Arc::new(TestCommandExecutor::default()),
        );
        let ctx = test_session_context(session, working_directory(), &app);

        let top_level_commands = ctx.top_level_commands().collect_vec();
        for keyword in keywords.iter() {
            assert!(top_level_commands.contains(&keyword.as_str()));
        }
    });
}

#[test]
pub fn test_session_context_top_level_commands_includes_external_commands() {
    App::test((), |app| async move {
        let session = Session::new(
            SessionInfo::new_for_test(),
            Arc::new(TestCommandExecutor::default()),
        );
        warpui::r#async::block_on(session.load_external_commands());

        let ctx = test_session_context(session, working_directory(), &app);

        // We expect git to be installed and on the PATH on all machines on
        // which we're running our unit tests.
        assert!(ctx.top_level_commands().contains(&"git"));
    });
}

#[test]
pub fn test_session_context_top_level_commands_includes_builtins() {
    App::test((), |app| async move {
        let builtins = vec!["export".into(), "print".into(), "break".into()];
        let session = Session::new(
            SessionInfo::new_for_test().with_builtins(builtins.clone().into_iter().collect()),
            Arc::new(TestCommandExecutor::default()),
        );
        let ctx = test_session_context(session, working_directory(), &app);

        let top_level_commands = ctx.top_level_commands().collect_vec();
        for builtin in builtins.iter() {
            assert!(top_level_commands.contains(&builtin.as_str()));
        }
    });
}

#[test]
pub fn test_session_context_lists_directory_entries_locally() {
    App::test((), |app| async move {
        VirtualFS::test(
            "test_session_context_lists_directory_entries_locally",
            |dirs, mut sandbox| {
                sandbox.mkdir("src/app");
                sandbox.mkdir("target/debug");
                sandbox.mkdir(".hidden/foo");

                sandbox.touch(vec![
                    Stub::EmptyFile("Cargo.toml"),
                    Stub::EmptyFile("src/app/mod.rs"),
                    Stub::EmptyFile("target/debug/warpui"),
                ]);

                let tests_dir = TypedPathBuf::from(dirs.tests().to_string_lossy().as_bytes());

                let ctx = test_session_context(Session::test(), tests_dir.clone(), &app);
                let ctx = ctx
                    .path_completion_context()
                    .expect("Path completion context should exist with active session");

                assert_eq!(
                    HashSet::<EngineDirEntry>::from_iter(Arc::unwrap_or_clone(
                        warpui::r#async::block_on(ctx.list_directory_entries(tests_dir))
                    )),
                    HashSet::from_iter([
                        EngineDirEntry::test_dir(".hidden"),
                        EngineDirEntry::test_file("Cargo.toml"),
                        EngineDirEntry::test_dir("target"),
                        EngineDirEntry::test_dir("src"),
                    ])
                );
            },
        );
    });
}

/// Given a Windows-encoded path, such as `C:\User\my_username`,
/// convert it to a UNIX shell path that will work with the `bash`
/// executable, such as `/mnt/c/Users/my_username`.
///
/// This is NOT the same as MSYS2 encoding, which does not
/// use the `/mnt` prefix.
#[cfg(windows)]
fn windows_to_unix_shell_encoding(
    windows_path: &typed_path::Path<typed_path::WindowsEncoding>,
) -> TypedPathBuf {
    let mut unix_path = TypedPathBuf::unix();
    for component in windows_path.components() {
        match component {
            WindowsComponent::Prefix(p) => {
                match p.kind() {
                    WindowsPrefix::Disk(disk_letter) | WindowsPrefix::VerbatimDisk(disk_letter) => {
                        let disk_byte = &[disk_letter];
                        let drive_name = String::from_utf8_lossy(disk_byte);
                        unix_path.push(UnixComponent::RootDir);
                        unix_path.push("mnt");
                        unix_path.push(drive_name.to_string().to_ascii_lowercase());
                    }
                    _ => {} // We don't care about other prefix types (see https://doc.rust-lang.org/nightly/std/path/enum.Prefix.html).
                }
            }
            // Avoid adding the root directory twice if there's already a drive
            WindowsComponent::RootDir => {}
            _ => {
                unix_path.push(component);
            }
        }
    }
    unix_path
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
pub fn test_session_context_lists_directory_entries_remotely() {
    App::test((), |app| async move {
        VirtualFS::test(
            "test_session_context_lists_directory_entries_remotely",
            |dirs, mut sandbox| {
                sandbox.mkdir("src/app");
                sandbox.mkdir("target/debug");

                sandbox.touch(vec![
                    Stub::EmptyFile("control_path.socket"),
                    Stub::EmptyFile("Cargo.toml"),
                    Stub::EmptyFile("src/app/mod.rs"),
                    Stub::EmptyFile("target/debug/warpui"),
                ]);

                let cwd = TypedPathBuf::from(dirs.tests().to_string_lossy().as_bytes());

                // We assume all remotes are UNIX-based.
                // The test directory we're using here is a local temp directory, which means
                // it uses native path encoding.
                // On Windows, we must convert the test directory to UNIX encoding
                // before being able to run bash commands within it.
                #[cfg(windows)]
                let cwd = match cwd {
                    TypedPathBuf::Unix(_) => cwd,
                    TypedPathBuf::Windows(windows_path) => {
                        windows_to_unix_shell_encoding(windows_path.as_path())
                    }
                };

                let ctx = test_session_context(Session::test_remote(), cwd.clone(), &app);

                let mut entries = HashSet::<EngineDirEntry>::from_iter(Arc::unwrap_or_clone(
                    warpui::r#async::block_on(ctx.list_directory_entries(cwd)),
                ));
                // TODO(CORE-2000): The ls script we use to list entries in remote
                // sessions adds a spurious "." directory when run in the VirtualFS.
                // As a temporary workaround, we remove this file in the test.
                entries.remove(&EngineDirEntry::test_dir("."));

                assert_eq!(
                    entries,
                    HashSet::from_iter([
                        EngineDirEntry::test_file("Cargo.toml"),
                        EngineDirEntry::test_file("control_path.socket"),
                        EngineDirEntry::test_dir("src"),
                        EngineDirEntry::test_dir("target"),
                    ])
                );
            },
        );
    });
}

fn perform_special_characters_in_path_test(session: Session, file_names: Vec<&str>) {
    let file_names = file_names
        .iter()
        .map(|&filename| String::from(filename))
        .collect_vec();
    App::test((), |app| async move {
        VirtualFS::test(
            "test_session_context_lists_directory_entries_with_special_characters",
            |dirs, mut sandbox| {
                sandbox.mkdir("te st/");
                sandbox.mkdir("te st/foo");

                let files_to_create = file_names
                    .iter()
                    .map(|file_name| String::from("te st/") + file_name.as_str())
                    .collect_vec();
                let file_stubs = files_to_create
                    .iter()
                    .map(|file_path| Stub::EmptyFile(file_path.as_str()))
                    .collect_vec();
                sandbox.touch(file_stubs);

                let test_dir_base = TypedPathBuf::from(dirs.tests().to_string_lossy().as_bytes());
                let test_dir = test_dir_base.join("te st/");

                #[cfg(windows)]
                let test_dir = if session.is_local() {
                    test_dir
                } else {
                    // We assume all remotes are UNIX-based.
                    // The test directory we're using here is a local temp directory, which means
                    // it uses native path encoding.
                    // On Windows, we must convert the test directory to UNIX encoding
                    // before being able to run bash commands within it.
                    match test_dir {
                        TypedPathBuf::Unix(_) => test_dir,
                        TypedPathBuf::Windows(windows_path) => {
                            windows_to_unix_shell_encoding(windows_path.as_path())
                        }
                    }
                };

                let ctx = test_session_context(session, test_dir.clone(), &app);

                let mut entries = HashSet::<EngineDirEntry>::from_iter(Arc::unwrap_or_clone(
                    warpui::r#async::block_on(ctx.list_directory_entries(test_dir)),
                ));
                // TODO(CORE-2000): The ls script we use to list entries in remote
                // sessions adds a spurious "." directory when run in the VirtualFS.
                // As a temporary workaround, we remove this file in the test.
                entries.remove(&EngineDirEntry::test_dir("."));

                let mut expected_dir_entries = file_names
                    .into_iter()
                    .map(|file_name| EngineDirEntry::test_file(&file_name))
                    .collect_vec();
                expected_dir_entries.push(EngineDirEntry::test_dir("foo"));

                assert_eq!(entries, HashSet::from_iter(expected_dir_entries));
            },
        );
    });
}

#[test]
pub fn test_session_context_lists_directory_entries_locally_with_special_characters_in_path() {
    #[cfg(unix)]
    let file_names = vec!["a.txt", "b file.txt", "c's.txt", "\"d\".txt", "e\nfile.txt"];

    // Windows filenames are more restrictive than UNIX. Notably,
    // Windows doesn't allow characters in the 1-31 range, which includes carriage returns (13, '\r')
    // and newlines (10, '\n') and reserves certain characters.
    // See https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file#naming-conventions.
    #[cfg(windows)]
    let file_names = vec![r"a.txt", r"b file.txt", r"c's.txt", r"#d&.txt"];

    perform_special_characters_in_path_test(Session::test(), file_names);
}

/// Regression test for CORE-1927.
#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
pub fn test_session_context_lists_directory_entries_remotely_with_special_characters_in_path() {
    #[cfg(unix)]
    let file_names = vec!["a.txt", "b file.txt", "c's.txt", "\"d\".txt", "e\nfile.txt"];

    // Windows filenames are more restrictive than UNIX. Notably,
    // Windows doesn't allow characters in the 1-31 range, which includes carriage returns (13, '\r')
    // and newlines (10, '\n') and reserves certain characters.
    // See https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file#naming-conventions.
    #[cfg(windows)]
    let file_names = vec![r"a.txt", r"b file.txt", r"c's.txt", r"#d&.txt"];

    perform_special_characters_in_path_test(Session::test_remote(), file_names);
}
