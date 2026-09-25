#[cfg(windows)]
use std::any::Any;
#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(windows)]
use anyhow::Result;
#[cfg(windows)]
use async_trait::async_trait;
#[cfg(windows)]
use warp_completer::completer::{CommandExitStatus, CommandOutput, CompletionContext};
#[cfg(windows)]
use warp_completer::signatures::CommandRegistry;
#[cfg(windows)]
use warpui::App;

use super::*;
#[cfg(windows)]
use crate::terminal::ShellLaunchData;
#[cfg(windows)]
use crate::terminal::model::session::command_executor::{CommandExecutor, ExecuteCommandOptions};
#[cfg(windows)]
use crate::terminal::model::session::{Session, SessionInfo};
#[cfg(windows)]
use crate::terminal::shell::{Shell, ShellType};
#[cfg(windows)]
use crate::test_util::{Stub, VirtualFS};

#[test]
fn expands_wsl_home_in_directory_chip_paths() {
    assert_eq!(
        expand_session_home("~", Some("/root"), &['/']),
        TypedPathBuf::from_unix("/root")
    );
    assert_eq!(
        expand_session_home("~/warp-chip-proof", Some("/root"), &['/']),
        TypedPathBuf::from_unix("/root/warp-chip-proof")
    );
}

#[cfg(windows)]
#[test]
fn expands_windows_session_home_in_directory_chip_paths() {
    assert_eq!(
        expand_session_home(r"~\Desktop", Some(r"C:\Users\runneradmin"), &['/', '\\']),
        TypedPathBuf::from_windows(r"C:\Users\runneradmin\Desktop")
    );
}

#[cfg(windows)]
#[test]
fn expands_wsl_home_to_host_unc_path() {
    let directory = expand_session_home("~/warp-chip-proof", Some("/root"), &['/']);
    let mut session_info = SessionInfo::new_for_test().with_shell_type(ShellType::Bash);
    session_info.launch_data = Some(ShellLaunchData::WSL {
        distro: "Ubuntu".to_owned(),
    });
    let session = Session::new(session_info, Arc::new(ListingExecutor::default()));

    assert_eq!(
        session
            .maybe_convert_to_native_path(&directory.to_path())
            .unwrap(),
        std::path::PathBuf::from(r"\\WSL$\Ubuntu\root\warp-chip-proof")
    );
}

#[test]
fn leaves_non_home_directory_paths_unchanged() {
    assert_eq!(
        expand_session_home("/tmp/warp-chip-proof", Some("/root"), &['/']),
        TypedPathBuf::from_unix("/tmp/warp-chip-proof")
    );
    assert_eq!(
        expand_session_home("~another", Some("/root"), &['/']),
        TypedPathBuf::from_unix("~another")
    );
    assert_eq!(
        expand_session_home("~/warp-chip-proof", None, &['/']),
        TypedPathBuf::from_unix("~/warp-chip-proof")
    );
}

#[cfg(windows)]
#[derive(Debug, Default)]
struct ListingExecutor {
    commands_executed: AtomicUsize,
}

#[cfg(windows)]
#[async_trait]
impl CommandExecutor for ListingExecutor {
    async fn execute_command(
        &self,
        _command: &str,
        _shell: &Shell,
        _current_directory_path: Option<&str>,
        _environment_variables: Option<HashMap<String, String>>,
        _options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        self.commands_executed.fetch_add(1, Ordering::SeqCst);
        Ok(CommandOutput {
            stdout: b"./guest-only/\0\0".to_vec(),
            stderr: Vec::new(),
            status: CommandExitStatus::Success,
            exit_code: None,
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        true
    }
}

#[cfg(windows)]
#[test]
fn wsl_directory_chip_uses_host_without_replacing_guest_completion_cache() {
    App::test((), |app| async move {
        VirtualFS::test(
            "wsl_directory_chip_uses_host_without_replacing_guest_completion_cache",
            |dirs, mut sandbox| {
                sandbox.touch(vec![Stub::EmptyFile("host-only.txt")]);
                let host_path = dirs.tests().to_string_lossy();
                let guest_path = warp_util::path::convert_windows_path_to_wsl(&host_path);
                let executor = Arc::new(ListingExecutor::default());
                let mut session_info = SessionInfo::new_for_test().with_shell_type(ShellType::Bash);
                session_info.launch_data = Some(ShellLaunchData::WSL {
                    distro: "Ubuntu".to_owned(),
                });
                let session = Session::new(session_info, executor.clone());
                let session_context = app.read(|ctx| {
                    SessionContext::new(
                        session,
                        CommandRegistry::default().into(),
                        TypedPathBuf::from_unix(&guest_path),
                        ctx,
                    )
                });

                let chip_items = warpui::r#async::block_on(DirectoryFetcher::fetch_files_async(
                    &session_context,
                    &guest_path,
                ));
                assert_eq!(
                    chip_items,
                    vec![create_directory_item(
                        "host-only.txt",
                        DirectoryType::TextFile
                    )]
                );
                assert_eq!(executor.commands_executed.load(Ordering::SeqCst), 0);

                let completions = warpui::r#async::block_on(
                    session_context
                        .path_completion_context()
                        .unwrap()
                        .list_directory_entries(TypedPathBuf::from_unix(&guest_path)),
                );
                assert_eq!(
                    completions.as_ref(),
                    &[EngineDirEntry {
                        file_name: "guest-only".to_owned(),
                        file_type: EngineFileType::Directory,
                    }]
                );
                assert_eq!(executor.commands_executed.load(Ordering::SeqCst), 1);
            },
        );
    });
}

fn create_directory_item(name: &str, directory_type: DirectoryType) -> DirectoryItem {
    DirectoryItem {
        name: name.to_string(),
        directory_type,
    }
}

#[test]
fn test_sort_comparison_total_order() {
    // Test that sort_menu_items produces consistent ordering that prevents panics
    // by verifying the expected Directory < TextFile < OtherFile hierarchy

    // Test basic type ordering with different combinations
    let mut items1 = vec![
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("text.txt", DirectoryType::TextFile),
    ];
    sort_menu_items(&mut items1);
    assert_eq!(items1[0].directory_type, DirectoryType::Directory);
    assert_eq!(items1[1].directory_type, DirectoryType::TextFile);

    let mut items2 = vec![
        create_directory_item("text.txt", DirectoryType::TextFile),
        create_directory_item("binary.exe", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut items2);
    assert_eq!(items2[0].directory_type, DirectoryType::TextFile);
    assert_eq!(items2[1].directory_type, DirectoryType::OtherFile);

    let mut items3 = vec![
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("binary.exe", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut items3);
    assert_eq!(items3[0].directory_type, DirectoryType::Directory);
    assert_eq!(items3[1].directory_type, DirectoryType::OtherFile);

    // Test that sort_menu_items is consistent - calling it multiple times
    // on the same data should produce the same result
    let test_items = vec![
        create_directory_item("binary.exe", DirectoryType::OtherFile),
        create_directory_item("folder", DirectoryType::Directory),
        create_directory_item("text.txt", DirectoryType::TextFile),
    ];

    let mut items_copy1 = test_items.clone();
    let mut items_copy2 = test_items.clone();

    sort_menu_items(&mut items_copy1);
    sort_menu_items(&mut items_copy2);

    // Both sorts should produce identical results
    assert_eq!(items_copy1, items_copy2);

    // Verify the expected ordering: Directory, TextFile, OtherFile
    assert_eq!(items_copy1[0].directory_type, DirectoryType::Directory);
    assert_eq!(items_copy1[1].directory_type, DirectoryType::TextFile);
    assert_eq!(items_copy1[2].directory_type, DirectoryType::OtherFile);
}

#[test]
fn test_sort_same_types_alphabetically() {
    let mut dirs = vec![
        create_directory_item("zebra", DirectoryType::Directory),
        create_directory_item("alpha", DirectoryType::Directory),
        create_directory_item("beta", DirectoryType::Directory),
    ];
    sort_menu_items(&mut dirs);
    assert_eq!(dirs[0].name, "alpha");
    assert_eq!(dirs[1].name, "beta");
    assert_eq!(dirs[2].name, "zebra");

    let mut texts = vec![
        create_directory_item("z.txt", DirectoryType::TextFile),
        create_directory_item("a.rs", DirectoryType::TextFile),
        create_directory_item("m.py", DirectoryType::TextFile),
    ];
    sort_menu_items(&mut texts);
    assert_eq!(texts[0].name, "a.rs");
    assert_eq!(texts[1].name, "m.py");
    assert_eq!(texts[2].name, "z.txt");

    let mut others = vec![
        create_directory_item("z.bin", DirectoryType::OtherFile),
        create_directory_item("a.exe", DirectoryType::OtherFile),
        create_directory_item("m.dll", DirectoryType::OtherFile),
    ];
    sort_menu_items(&mut others);
    assert_eq!(others[0].name, "a.exe");
    assert_eq!(others[1].name, "m.dll");
    assert_eq!(others[2].name, "z.bin");
}

#[test]
fn test_sort_single_item() {
    let mut items = vec![create_directory_item("single", DirectoryType::Directory)];
    sort_menu_items(&mut items);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "single");
}
