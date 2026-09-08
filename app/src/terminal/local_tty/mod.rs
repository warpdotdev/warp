pub mod docker_sandbox;
pub mod terminal_manager;
mod terminal_view_adaptor;

pub use terminal_manager::{TerminalManager, get_shell_starter};
#[cfg(feature = "tui")]
pub use terminal_manager::{TerminalManagerInit, TerminalSurfaceInit, TerminalSurfaceResult};
#[cfg(windows)]
pub use terminal_view_adaptor::shutdown_all_pty_event_loops;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
pub(crate) use terminal_view_adaptor::{
    TerminalViewSurfaceConfig, create_terminal_view_surface, terminal_view_restored_blocks,
};
pub use warp_terminal::local_tty::*;

#[cfg(unix)]
pub fn run_terminal_server(args: &warp_cli::TerminalServerArgs) {
    warp_terminal::local_tty::server::run_terminal_server(
        args,
        crate::features::init_feature_flags,
        crate::terminal::platform::init,
    );
}

impl event_loop::ActiveTerminal for crate::terminal::TerminalModel {
    fn exit(&mut self, reason: crate::terminal::model::terminal_model::ExitReason) {
        crate::terminal::TerminalModel::exit(self, reason);
    }
}
