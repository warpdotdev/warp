use std::sync::Arc;

use warpui::r#async::executor::Background;

use super::is_shell_startup_pending;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::shell::ShellName;
use crate::terminal::{ShellLaunchState, TerminalModel};

#[test]
fn wsl_startup_timeout_is_ignored_after_early_terminal_exit() {
    let mut model = TerminalModel::new(
        None,
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        false,
        false,
        false,
        false,
        ObfuscateSecrets::No,
        false,
        None,
        ShellLaunchState::DeterminingShell {
            available_shell: None,
            display_name: ShellName::blank(),
        },
    );

    assert!(is_shell_startup_pending(&model));
    model.exit(ExitReason::ShellProcessExited);
    assert!(!is_shell_startup_pending(&model));
}
