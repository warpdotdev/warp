#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::iter::FromIterator;
use std::sync::Arc;

use typed_path::{TypedPath, TypedPathBuf};
#[cfg(unix)]
use warp_completer::completer::EngineDirEntry;
use warp_completer::signatures::CommandRegistry;
use warpui::App;

use crate::completer::SessionContext;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};
use crate::terminal::shell::ShellType;
#[cfg(unix)]
use crate::test_util::{Stub, VirtualFS};

fn test_session_context(session: Session, cwd: TypedPathBuf, app: &App) -> SessionContext {
    app.read(|ctx| SessionContext::new(session, CommandRegistry::default().into(), cwd, ctx))
}

fn test_wsl_like_session() -> Session {
    Session::new(
        SessionInfo::new_for_test().with_shell_type(ShellType::Bash),
        Arc::new(TestCommandExecutor::default()),
    )
}

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

#[test]
fn test_list_entries_returns_none_on_guest_failure() {
    App::test((), |app| async move {
        let directory = TypedPath::unix("/definitely/does/not/exist/on/this/machine");
        let ctx = test_session_context(test_wsl_like_session(), directory.to_path_buf(), &app);

        let result = warpui::r#async::block_on(super::list_entries(&ctx, &directory));
        assert_eq!(result, None);
    });
}

#[test]
fn test_run_guest_listing_returns_none_on_timeout() {
    App::test((), |app| async move {
        let ctx = test_session_context(
            test_wsl_like_session(),
            TypedPath::unix("/").to_path_buf(),
            &app,
        );

        let result = warpui::r#async::block_on(super::run_guest_listing(
            &ctx,
            "sleep 5",
            super::GUEST_LISTING_TIMEOUT,
        ));
        assert_eq!(result, None);
    });
}
