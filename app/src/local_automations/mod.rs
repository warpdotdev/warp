//! Local automations: personal, user-scoped jobs defined as TOML files under
//! the user's `automations/` data directory (e.g. `~/.warp/automations/` on
//! stable).
//!
//! Supports defining automations on disk, listing them, **Run now**, and
//! in-app cron scheduling while Warp is running (Slice B). Schedules do not
//! fire when Warp is quit; catch-up within 6 hours runs once on wake, older
//! gaps are marked missed.
//!
//! Gated on `FeatureFlag::LocalAutomations`.

pub mod list_view;
pub mod local_automation;
pub mod run_state;
pub mod schedule;
pub mod scheduler;

pub use list_view::LocalAutomationsView;
pub use local_automation::{LocalAutomation, LocalAutomationError, LocalAutomationRunner};
pub use scheduler::{LocalAutomationsScheduler, LocalAutomationsSchedulerEvent};

/// Prompt submitted to a fresh Warp agent conversation by the list view's
/// "New → Warp agent" action. Relies on the bundled `create-local-automation`
/// skill for schema details.
pub fn new_automation_agent_prompt() -> String {
    format!(
        "Create a new Warp local automation for me using the create-local-automation skill. \
         Ask me what it should do, roughly when it should run, and where it should run, then \
         write the TOML file to {}. Local automations are on-disk files — do not use cloud/Oz \
         scheduling. When you're done, remind me that schedules fire only while Warp is open \
         and the machine is awake, and that I can also use Run now in Settings → Automations.",
        warp_core::paths::home_relative_path(&crate::user_config::automations_dir())
    )
}

/// Self-contained creation prompt for the list view's "New → Copy prompt"
/// action, aimed at agents outside Warp (Claude Code, Codex, ...) that don't
/// have the bundled skills. Includes the schema inline.
pub fn new_automation_external_prompt() -> String {
    let dir = warp_core::paths::home_relative_path(&crate::user_config::automations_dir());
    format!(
        r#"Create a Warp local automation for me: one TOML file saved in {dir} (create the directory if needed) with a snake_case filename ending in .toml.

First ask me: what the automation should do, roughly when it should run (cron string), and which directory it should run in.

Schema (unknown fields are rejected; exactly one of `cwd` or `[worktree]`):

name = "Morning repo brief"        # required display name
enabled = true                     # optional, default true
schedule = "0 9 * * 1-5"           # required cron/preset; fires while Warp is open
cwd = "~/code/project"             # directory must exist at run time
# [worktree]                       # or run in a git worktree instead of cwd
# repo = "~/code/project"
# name = "automation-branch"
# base_branch = "main"             # optional

[runner]                           # required: warp_agent (with prompt) or shell (with command)
type = "warp_agent"
prompt = "Summarize commits on main from the last 24h."
# type = "shell"
# command = "gh pr list --author @me"

After writing the file, remind me: schedules fire only while Warp is open and the machine is awake (catch-up within ~6 hours after reopen; older gaps are marked missed). I can also run it immediately via Settings → Automations → Run now, or Command Palette → "Open Settings: Automations"."#
    )
}
