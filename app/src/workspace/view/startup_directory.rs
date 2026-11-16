//! Logic to determine the working directory for new terminal sessions.

use std::path::PathBuf;

use warpui::{AppContext, SingletonEntity, ViewContext, WindowId};

use super::Workspace;
use crate::terminal::ShellLaunchData;
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells::AvailableShells;
use crate::terminal::session_settings::{NewSessionSource, SessionSettings};

impl Workspace {
    /// Helper function to compute the initial directory for a new session
    /// that is inheriting its initial directory from the active session in
    /// the given workspace.
    fn initial_directory_from_active_session(&self, ctx: &AppContext) -> Option<PathBuf> {
        (!self.tabs.is_empty())
            .then(|| {
                self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                    pane_group.active_session_id(ctx).and_then(|base_pane_id| {
                        pane_group.startup_path_for_new_session(Some(base_pane_id), ctx)
                    })
                })
            })
            .flatten()
    }

    /// Helper function to retrieve the shell launch data of the active session,
    /// which tells us whether it's a native or WSL session.
    fn shell_launch_info_from_active_session(&self, ctx: &AppContext) -> Option<ShellLaunchData> {
        (!self.tabs.is_empty())
            .then(|| {
                self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                    pane_group.active_session_id(ctx).and_then(|base_pane_id| {
                        pane_group.launch_data_for_session(base_pane_id, ctx)
                    })
                })
            })
            .flatten()
    }

    /// Helper function to compute the initial directory for a new session.
    /// Returns Some(path) if inheriting the initial directory from an active
    /// session or using the user's custom path setting,
    /// and None if the default startup directory (the user's home directory) should be used.
    pub(super) fn get_new_tab_startup_directory(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<&AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PathBuf> {
        // Get the Workspace from the window that hosted the previously-active
        // session.
        let active_session_info = match previous_session_window_id {
            // If the previous window is the one hosting this workspace, don't
            // do any indirection through AppContext.
            Some(window_id) if window_id == ctx.window_id() => Some((
                self.initial_directory_from_active_session(ctx),
                self.shell_launch_info_from_active_session(ctx),
            )),
            // Otherwise, lookup the Workspace in that window and query it.
            Some(window_id) => {
                let workspace_handle = ctx
                    .views_of_type::<Workspace>(window_id)
                    .and_then(|views| views.first().cloned());
                workspace_handle.map(|workspace| {
                    workspace.read(ctx, |workspace, ctx| {
                        (
                            workspace.initial_directory_from_active_session(ctx),
                            workspace.shell_launch_info_from_active_session(ctx),
                        )
                    })
                })
            }
            None => None,
        };

        let (prev_session_working_directory, prev_session_shell) =
            active_session_info.unwrap_or_default();

        cfg_if::cfg_if! {
            if #[cfg(feature = "local_tty")] {
                let is_wsl = new_session_shell(chosen_shell, ctx)
                .wsl_distro()
                .is_some();
            } else {
                let is_wsl = false;
            }
        }

        let is_same_system = same_system(prev_session_shell.as_ref(), chosen_shell, ctx);

        compute_startup_directory_from_prev_session(
            new_session_source,
            if is_same_system {
                prev_session_working_directory
            } else {
                None
            },
            is_wsl,
            ctx,
        )
    }
}

/// The shell to be used in the new session,
/// based on the shell explicitly chosen by the user or
/// the default startup shell specified in settings.
#[cfg(feature = "local_tty")]
fn new_session_shell(chosen_shell: Option<&AvailableShell>, ctx: &AppContext) -> AvailableShell {
    chosen_shell.cloned().unwrap_or_else(move || {
        AvailableShells::handle(ctx).read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx))
    })
}

/// Windows-specific helper function to determine whether the old and
/// new shell sessions will exist in the same system, i.e. whether
/// they're both on native Windows or both in the same WSL distribution.
///
/// Returns `true` if `old_session_launch_data` is `None`.
#[cfg(feature = "local_tty")]
fn same_system(
    old_session_launch_data: Option<&ShellLaunchData>,
    chosen_shell: Option<&AvailableShell>,
    ctx: &AppContext,
) -> bool {
    // If there's no prior session, there is no prior system.
    // We're not crossing a system boundary, so return true.
    let Some(old_launch_data) = old_session_launch_data else {
        return true;
    };

    let wsl_distro = new_session_shell(chosen_shell, ctx).wsl_distro();
    match old_launch_data {
        ShellLaunchData::WSL { distro: old_distro } => {
            wsl_distro.is_some_and(|new_distro| new_distro == *old_distro)
        }
        _ => wsl_distro.is_none(),
    }
}

#[cfg(not(feature = "local_tty"))]
const fn same_system(
    _old_session_launch_data: Option<&ShellLaunchData>,
    _chosen_shell: Option<&AvailableShell>,
    _ctx: &AppContext,
) -> bool {
    true
}

/// Helper function to compute the actual startup directory for the
/// new session based on the user's settings.
fn compute_startup_directory_from_prev_session(
    new_session_source: NewSessionSource,
    initial_directory_from_prev_session: Option<PathBuf>,
    ignore_custom_directory: bool,
    ctx: &ViewContext<Workspace>,
) -> Option<PathBuf> {
    SessionSettings::handle(ctx).read(ctx, |settings, _ctx| {
        settings
            .working_directory_config
            .initial_directory_for_new_session(
                new_session_source,
                initial_directory_from_prev_session,
                ignore_custom_directory,
            )
    })
}
