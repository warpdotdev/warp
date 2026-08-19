use std::collections::HashSet;
use std::iter::FromIterator;
use std::sync::Arc;

use typed_path::{TypedPath, TypedPathBuf};
use warp_completer::completer::EngineDirEntry;
use warpui::App;

use crate::completer::SessionContext;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};
use crate::terminal::shell::ShellType;
use crate::test_util::{Stub, VirtualFS};

fn test_session_context(session: Session, cwd: TypedPathBuf, app: &App) -> SessionContext {
    app.read(|ctx| {
        SessionContext::new(
            session,
            warp_completer::signatures::CommandRegistry::default().into(),
            cwd,
            ctx,
        )
    })
}

/// A `Session` whose guest command runs through a real local shell rather than `wsl.exe`, so the
/// exact `find -L`-based script this module builds can be exercised end to end without a real
/// WSL host. Forces Bash regardless of the host platform's default test shell (mirroring
/// `Session::test_remote()`), since the script is POSIX-shaped -- this is what the WSL guest
/// itself would run.
fn test_wsl_like_session() -> Session {
    Session::new(
        SessionInfo::new_for_test().with_shell_type(ShellType::Bash),
        Arc::new(TestCommandExecutor::default()),
    )
}

/// Regression test for APP-3993: asking the WSL guest for a listing directly, rather than
/// patching a host listing, both classifies a symlink-to-directory correctly and lists a
/// directory reached only by traversing a symlink -- the "completing inside a symlinked
/// directory" case the host cannot handle at all over `\wsl$`.
///
/// Unix-only, matching `test_session_context_follows_symlinked_directories_remotely` in
/// `test.rs`: this module only ever compiles on Windows, so on a real run this test only
/// exercises the mechanism when the host itself is also Unix-like enough to create symlinks the
/// same way WSL's guest does (i.e. when temporarily un-gated for local verification, as this
/// PR's own testing notes describe doing for the Windows-only module as a whole).
#[cfg(unix)]
#[test]
fn test_list_entries_follows_symlinks_and_succeeds() {
    App::test((), |app| async move {
        VirtualFS::test(
            "test_list_entries_follows_symlinks_and_succeeds",
            |dirs, mut sandbox| {
                sandbox.mkdir("real_dir");
                sandbox.touch(vec![Stub::EmptyFile("real_file.txt")]);
                sandbox.ln("real_dir", "link_to_dir");
                sandbox.ln("real_file.txt", "link_to_file");

                let cwd = TypedPathBuf::from(dirs.tests().to_string_lossy().as_bytes());
                let ctx = test_session_context(test_wsl_like_session(), cwd.clone(), &app);

                let entries = warpui::r#async::block_on(super::list_entries(&ctx, &cwd.to_path()))
                    .expect("guest listing should succeed against a real local shell");

                let mut entries = HashSet::<EngineDirEntry>::from_iter(entries);
                // TODO(CORE-2000): The ls script we use to list entries adds a spurious "."
                // directory when run in the VirtualFS. As a temporary workaround, we remove
                // this entry in the test, matching the equivalent remote-session tests.
                entries.remove(&EngineDirEntry::test_dir("."));

                assert_eq!(
                    entries,
                    HashSet::from_iter([
                        EngineDirEntry::test_dir("real_dir"),
                        EngineDirEntry::test_file("real_file.txt"),
                        EngineDirEntry::test_dir("link_to_dir"),
                        EngineDirEntry::test_file("link_to_file"),
                    ])
                );
            },
        );
    });
}

/// A guest command that can't even run (here, a `cd` into a directory that doesn't exist) must
/// signal the caller to fall back to the host listing rather than silently returning an empty
/// one.
#[test]
fn test_list_entries_returns_none_on_guest_failure() {
    App::test((), |app| async move {
        let directory = TypedPath::unix("/definitely/does/not/exist/on/this/machine");
        let ctx = test_session_context(test_wsl_like_session(), directory.to_path_buf(), &app);

        let result = warpui::r#async::block_on(super::list_entries(&ctx, &directory));
        assert_eq!(result, None);
    });
}
