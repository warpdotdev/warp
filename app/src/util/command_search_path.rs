/// Singleton model that computes and caches the system command search
/// PATH at startup.
///
/// On macOS, this reads `/etc/paths` and `/etc/paths.d/*` (the same
/// mechanism `path_helper` uses for login shells) so that child processes
/// can find user-installed binaries like `git-lfs` even when Warp is
/// launched from the Dock with a minimal launchd PATH.
///
/// The computed PATH is stored in [`warp_util::command_search_path`] and
/// consumed by `run_git_command` and other subprocess helpers.
use warpui::{Entity, ModelContext, SingletonEntity};

pub struct CommandSearchPathModel;

impl CommandSearchPathModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(target_os = "macos")]
        if let Some(path) = warp_util::command_search_path::build_macos_system_path() {
            log::info!(
                "CommandSearchPathModel: setting system PATH ({} entries)",
                path.split(':').count()
            );
            warp_util::command_search_path::set_path(path);
        }
        Self
    }
}

impl Entity for CommandSearchPathModel {
    type Event = ();
}

impl SingletonEntity for CommandSearchPathModel {}
