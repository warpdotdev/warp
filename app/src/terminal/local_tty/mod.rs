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

    fn on_tmux_control_mode(&mut self, active: bool) {
        self.set_tmux_control_mode(active);
        #[cfg(all(unix, not(feature = "remote_tty")))]
        {
            use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};
            if active
                && !self.is_tmux_presentation()
                && self
                    .tmux_instance_id()
                    .and_then(|id| TmuxRuntime::for_id(TmuxInstanceId::from_u64(id)))
                    .is_none()
            {
                let runtime = TmuxRuntime::new();
                if let Some(shell_type) = self.last_init_shell_type() {
                    runtime.set_authoritative_shell_type(shell_type);
                }
                if let Some(session_id) = self.take_tmux_expected_session_id() {
                    runtime.set_spawned_expected_session(session_id);
                    if let Some(script) = self.take_tmux_retained_zsh_init() {
                        runtime.set_pending_retained_zsh_init(script, session_id);
                    }
                }
                self.set_tmux_instance_id(Some(runtime.id().as_u64()));
            }
        }
    }

    fn on_tmux_pane_output(
        &mut self,
        pane_id: &warp_terminal::tmux::PaneId,
        bytes: &[u8],
    ) -> Vec<Vec<u8>> {
        #[cfg(all(unix, not(feature = "remote_tty")))]
        {
            use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};
            let Some(id) = self.tmux_instance_id() else {
                return Vec::new();
            };
            let Some(runtime) = TmuxRuntime::for_id(TmuxInstanceId::from_u64(id)) else {
                return Vec::new();
            };
            let writes = runtime.take_retained_init_send_keys(pane_id.as_str());
            if !runtime.deliver_output(pane_id, bytes) {
                self.on_tmux_presentation_unready();
            }
            writes
        }
        #[cfg(not(all(unix, not(feature = "remote_tty"))))]
        {
            let _ = (pane_id, bytes);
            Vec::new()
        }
    }

    fn on_tmux_focus(&mut self, pane_id: &warp_terminal::tmux::PaneId) {
        self.set_tmux_focused_pane(Some(pane_id.as_str().to_owned()));
    }

    fn on_tmux_layout(
        &mut self,
        window_id: &warp_terminal::tmux::WindowId,
        layout: &str,
        visible_layout: Option<&str>,
        flags: Option<&str>,
    ) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::LayoutChange {
                window_id: window_id.as_str().to_owned(),
                layout: layout.to_owned(),
                visible_layout: visible_layout.map(str::to_owned),
                flags: flags.map(str::to_owned),
            },
        );
    }

    fn on_tmux_window_add(&mut self, window_id: &warp_terminal::tmux::WindowId) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowAdd {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_window_close(&mut self, window_id: &warp_terminal::tmux::WindowId) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowClose {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_window_renamed(&mut self, window_id: &warp_terminal::tmux::WindowId, name: &str) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowRenamed {
                window_id: window_id.as_str().to_owned(),
                name: name.to_owned(),
            },
        );
    }

    fn on_tmux_session_window_changed(&mut self, window_id: &warp_terminal::tmux::WindowId) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::SessionWindowChanged {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_command_end(
        &mut self,
        number: u64,
        error: bool,
        payload: &[String],
        capture_pane: Option<&warp_terminal::tmux::PaneId>,
    ) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::CommandEnd {
                number,
                error,
                payload: payload.to_vec(),
                capture_pane: capture_pane.map(|pane| pane.as_str().to_owned()),
            },
        );
    }

    fn on_tmux_presentation_unready(&mut self) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::PresentationUnready,
        );
    }
}
