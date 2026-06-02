pub mod bindings;
pub mod commands;

use bitflags::bitflags;
pub use commands::SlashCommandId;

bitflags! {
    /// Specifies the requirements for a slash command to be available.
    ///
    /// Each flag represents a requirement that the session context must satisfy. The command is
    /// available when the session supports *all* of the command's requirement flags.
    ///
    /// A few common cases:
    /// * If neither [`Self::AGENT_VIEW`] nor [`Self::TERMINAL_VIEW`] is set, the command is available in all modes.
    ///   A command should *not* set both flags to be available in both modes - this results in requirements that cannot be satisfied.
    /// * Most `/fork`-like slash commands require [`Self::NO_LRC_CONTROL`] and [`Self::ACTIVE_CONVERSATION`]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Availability: u16 {
        /// No requirements — always available.
        const ALWAYS = 0;
        /// Requires the agent view.
        const AGENT_VIEW = 1 << 0;
        /// Requires the terminal view.
        const TERMINAL_VIEW = 1 << 1;
        /// Requires a local session (not available in remote/cloud sessions).
        const LOCAL = 1 << 2;
        /// Requires a git repository.
        const REPOSITORY = 1 << 3;
        /// Requires that the agent is not currently in control of a long-running command.
        const NO_LRC_CONTROL = 1 << 4;
        /// Requires an active AI conversation.
        const ACTIVE_CONVERSATION = 1 << 5;
        /// Requires codebase context to be enabled.
        const CODEBASE_CONTEXT = 1 << 6;
        /// Requires AI to be globally enabled.
        const AI_ENABLED = 1 << 7;
        const NOT_CLOUD_AGENT = 1 << 8;
        /// Set on the session context iff the slash command data source was constructed via
        /// `SlashCommandDataSource::for_cloud_mode_v2` *and* `FeatureFlag::CloudModeInputV2`
        /// is enabled. Commands that require this bit are hidden everywhere except the V2
        /// cloud-mode composing input.
        const CLOUD_AGENT_V2 = 1 << 9;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Argument {
    pub hint_text: Option<&'static str>,
    pub is_optional: bool,
    /// If `true`, selecting the slash command from the menu (or via keybinding) will execute the
    /// slash command with no arguments.
    ///
    /// If `false`, selecting the slash command from the menu (or via keybinding) inserts the
    /// slash command into the input.
    ///
    /// Set this based on whether or not you want you think a user should always have the option to
    /// supply an argument.
    pub should_execute_on_selection: bool,
}

impl Argument {
    pub(super) fn optional() -> Self {
        Self {
            is_optional: true,
            ..Default::default()
        }
    }

    pub(super) fn required() -> Self {
        Self {
            is_optional: false,
            ..Default::default()
        }
    }

    pub(super) fn with_hint_text(mut self, text: &'static str) -> Self {
        self.hint_text = Some(text);
        self
    }

    pub(super) fn with_execute_on_selection(mut self) -> Self {
        self.should_execute_on_selection = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub icon_path: &'static str,
    /// Specifies the requirements for this command to be available. See [`Availability`].
    pub availability: Availability,
    /// Whether this command requires AI mode when executed.
    /// If true, AI mode will be activated when the command is accepted.
    pub auto_enter_ai_mode: bool,
    pub argument: Option<Argument>,
}

impl StaticCommand {
    pub fn matches_filter(&self, filter_text: &str) -> bool {
        if filter_text.is_empty() {
            return true;
        }

        let filter_lower = filter_text.to_lowercase();
        self.name
            .to_lowercase()
            .get(1..)
            .unwrap_or("")
            .starts_with(&filter_lower)
    }

    pub fn is_active(&self, session_context: Availability) -> bool {
        session_context.contains(self.availability)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
