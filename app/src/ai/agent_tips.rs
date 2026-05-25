use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::palette::PaletteMode;
use crate::server::telemetry::PaletteSource;
use crate::settings::AISettings;
use crate::terminal::input::SET_INPUT_MODE_AGENT_ACTION_NAME;
use crate::terminal::view::init::{
    CANCEL_COMMAND_KEYBINDING, SELECT_PREVIOUS_BLOCK_ACTION_NAME,
    TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
};
use crate::util::bindings::trigger_to_keystroke;
use crate::workspace::view::{
    TOGGLE_COMMAND_PALETTE_KEYBINDING_NAME, TOGGLE_RIGHT_PANEL_BINDING_NAME,
};
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use markdown_parser::FormattedTextFragment;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;
use warpui::keymap::Keystroke;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

/// Trait for tip implementations that can be displayed to users.
/// Tips provide helpful information with optional links and keybindings.
pub trait AITip: Clone {
    /// Returns the keystroke for this tip, if applicable.
    fn keystroke(&self, app: &AppContext) -> Option<Keystroke>;

    /// Returns the documentation link for this tip, if available.
    fn link(&self) -> Option<String>;

    /// Returns the raw description text for this tip.
    fn description(&self) -> &str;

    /// Converts the tip to formatted text fragments for rendering.
    /// Default implementation adds "Tip: " prefix and parses backtick-wrapped text as inline code.
    fn to_formatted_text(&self, app: &AppContext) -> Vec<FormattedTextFragment> {
        let text = format!(
            "{}{}",
            crate::localization::text_for_app(app, "agent.tips.prefix"),
            self.description()
        );

        // Style backtick-wrapped text as inline code
        let parts: Vec<&str> = text.split('`').collect();
        let mut fragments = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i % 2 == 0 {
                fragments.push(FormattedTextFragment::plain_text(part.to_string()));
            } else {
                fragments.push(FormattedTextFragment::inline_code(part.to_string()));
            }
        }
        fragments
    }

    /// Checks if this tip is applicable in the current context.
    /// Default implementation returns true (tip is always applicable).
    fn is_tip_applicable(
        &self,
        _current_working_directory: Option<&str>,
        _app: &AppContext,
    ) -> bool {
        true
    }
}

/// Kinds of agent tips for organizing and filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentTipKind {
    CodebaseContext,
    WarpDrive,
    General,
    Mcp,
    SlashCommands,
    /// Tips about adding context (files, blocks, URLs, images, @-mentions, rules)
    Context,
    /// Tips about code editors, file trees, and code review panes
    Code,
    /// Tips about local-to-cloud handoff
    Handoff,
}

static DEFAULT_TIPS: LazyLock<Vec<AgentTip>> = LazyLock::new(|| {
    fn tip(
        description_key: &'static str,
        link: Option<&'static str>,
        binding_name: Option<&'static str>,
        action: Option<WorkspaceAction>,
        kind: AgentTipKind,
    ) -> AgentTip {
        AgentTip {
            description_key,
            link: link.map(str::to_string),
            binding_name,
            action,
            kind,
        }
    }

    vec![
        tip(
            "agent.tips.default.01",
            Some("https://docs.warp.dev/agent-platform/capabilities/slash-commands"),
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.02",
            Some("https://docs.warp.dev/terminal/input/universal-input#input-modes"),
            Some(SET_INPUT_MODE_AGENT_ACTION_NAME),
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.03",
            Some("https://docs.warp.dev/agent-platform/capabilities/planning"),
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.04",
            Some("https://docs.warp.dev/terminal/command-palette"),
            Some(TOGGLE_COMMAND_PALETTE_KEYBINDING_NAME),
            Some(WorkspaceAction::OpenPalette {
                mode: PaletteMode::Command,
                source: PaletteSource::AgentTip,
                query: None,
            }),
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.05",
            Some("https://docs.warp.dev/knowledge-and-collaboration/warp-drive"),
            None,
            Some(WorkspaceAction::OpenWarpDrive),
            AgentTipKind::WarpDrive,
        ),
        tip(
            "agent.tips.default.06",
            None,
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.07",
            Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/using-to-add-context"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.08",
            Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context"),
            Some(SELECT_PREVIOUS_BLOCK_ACTION_NAME),
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.09",
            Some("https://docs.warp.dev/agent-platform/capabilities/codebase-context"),
            None,
            None,
            AgentTipKind::CodebaseContext,
        ),
        tip(
            "agent.tips.default.10",
            Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.11",
            Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.12",
            Some("https://docs.warp.dev/terminal/blocks/block-actions#copy-input-output-of-block"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.13",
            Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/images-as-context"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.14",
            Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.15",
            Some("https://docs.warp.dev/code/code-review"),
            Some(TOGGLE_RIGHT_PANEL_BINDING_NAME),
            None,
            AgentTipKind::Code,
        ),
        tip(
            "agent.tips.default.16",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
            None,
            None,
            AgentTipKind::Mcp,
        ),
        tip(
            "agent.tips.default.17",
            None,
            None,
            None,
            AgentTipKind::Mcp,
        ),
        tip(
            "agent.tips.default.18",
            Some("https://docs.warp.dev/reference/cli/integration-setup"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.19",
            None,
            None,
            None,
            AgentTipKind::WarpDrive,
        ),
        tip(
            "agent.tips.default.20",
            Some("https://docs.warp.dev/agent-platform/capabilities/rules"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.21",
            Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking"),
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.22",
            None,
            None,
            Some(WorkspaceAction::ToggleRightPanel),
            AgentTipKind::Code,
        ),
        tip(
            "agent.tips.default.23",
            Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents"),
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.24",
            None,
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.25",
            None,
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.26",
            Some("https://docs.warp.dev/reference/cli"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.27",
            Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.28",
            Some("https://docs.warp.dev/agent-platform/capabilities/rules#project-rules-1"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.29",
            Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/urls-as-context"),
            None,
            None,
            AgentTipKind::Context,
        ),
        tip(
            "agent.tips.default.30",
            Some("https://docs.warp.dev/terminal/warpify"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.31",
            Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.32",
            Some("https://docs.warp.dev/agent-platform/capabilities/rules"),
            None,
            None,
            AgentTipKind::SlashCommands,
        ),
        tip(
            "agent.tips.default.33",
            Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use#session-level-approvals"),
            Some(TOGGLE_AUTOEXECUTE_MODE_KEYBINDING),
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.34",
            None,
            None,
            None,
            AgentTipKind::Handoff,
        ),
        tip(
            "agent.tips.default.35",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/managing-cloud-agents#in-app-agent-notifications"),
            None,
            None,
            AgentTipKind::General,
        ),
        tip(
            "agent.tips.default.36",
            None,
            Some(CANCEL_COMMAND_KEYBINDING),
            None,
            AgentTipKind::General,
        ),
    ]
});

#[derive(Clone, Debug)]
pub struct AgentTip {
    pub description_key: &'static str,
    /// Catalog key for the text displayed to the user. The localized value is parsed such that:
    /// "Tip: " is added as a prefix,
    /// "<keybinding>" is replaced with user-defined and platform-specific keybinding referenced by binding_name,
    /// `text` that is wrapped in backticks is formatted as inline code
    pub link: Option<String>,
    pub binding_name: Option<&'static str>,
    pub action: Option<WorkspaceAction>,
    /// The kind of the tip, used for filtering and organization
    pub kind: AgentTipKind,
}

impl AITip for AgentTip {
    fn keystroke(&self, app: &AppContext) -> Option<Keystroke> {
        let binding_name = self.binding_name?;

        // Special case: voice input uses settings, not editable bindings
        if binding_name == "FN" {
            return AISettings::as_ref(app).voice_input_toggle_key.keystroke();
        }

        if let Some(binding) = app.editable_bindings().find(|b| b.name == binding_name) {
            return trigger_to_keystroke(binding.trigger);
        }
        None
    }

    fn link(&self) -> Option<String> {
        self.link.clone()
    }

    fn description(&self) -> &str {
        self.description_key
    }

    fn to_formatted_text(&self, app: &AppContext) -> Vec<FormattedTextFragment> {
        let mut description = crate::localization::text_for_app(app, self.description_key);

        // Replace <keybinding> with the actual keybinding string
        if let Some(keystroke) = self.keystroke(app) {
            description = description.replace("<keybinding>", &keystroke.displayed());
        }

        let text = format!(
            "{}{}",
            crate::localization::text_for_app(app, "agent.tips.prefix"),
            description
        );

        // Style backtick-wrapped text as inline code
        let parts: Vec<&str> = text.split('`').collect();
        let mut fragments = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i % 2 == 0 {
                fragments.push(FormattedTextFragment::plain_text(part.to_string()));
            } else {
                fragments.push(FormattedTextFragment::inline_code(part.to_string()));
            }
        }

        fragments
    }

    fn is_tip_applicable(&self, current_working_directory: Option<&str>, app: &AppContext) -> bool {
        // Tips about indexing the repo are only applicable if the current directory is not already indexed.
        if matches!(self.kind, AgentTipKind::CodebaseContext) {
            let Some(cwd) = current_working_directory else {
                return true;
            };
            let Some(root) = PersistedWorkspace::as_ref(app).root_for_workspace(Path::new(cwd))
            else {
                return true;
            };
            return CodebaseIndexManager::as_ref(app)
                .get_codebase_index_status_for_path(root, app)
                .is_none();
        }
        // Handoff tips only apply when the feature is available and enabled.
        if matches!(self.kind, AgentTipKind::Handoff) {
            return AISettings::as_ref(app).is_cloud_handoff_enabled(app);
        }
        // Tips whose description references a keybinding placeholder should only be shown
        // when the keybinding is actually configured, so we never display the raw
        // "<keybinding>" string to users.
        if self.binding_name.is_some() && self.keystroke(app).is_none() {
            return false;
        }
        true
    }
}

impl WorkspaceAction {
    pub fn display_text(&self, app: &AppContext) -> Option<String> {
        match self {
            WorkspaceAction::OpenPalette { .. } => Some(crate::localization::text_for_app(
                app,
                "agent.tips.action.open_palette",
            )),
            WorkspaceAction::OpenWarpDrive => Some(crate::localization::text_for_app(
                app,
                "agent.tips.action.open_warp_drive",
            )),
            WorkspaceAction::ToggleRightPanel => Some(crate::localization::text_for_app(
                app,
                "agent.tips.action.show_diff_view",
            )),
            _ => None,
        }
    }
}

pub fn tip_text_fragments(text: String) -> Vec<FormattedTextFragment> {
    let parts: Vec<&str> = text.split('`').collect();
    let mut fragments = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 0 {
            fragments.push(FormattedTextFragment::plain_text(part.to_string()));
        } else {
            fragments.push(FormattedTextFragment::inline_code(part.to_string()));
        }
    }
    fragments
}

/// Helper function to build the list of agent tips, including the voice tip if enabled.
pub fn get_agent_tips(ctx: &AppContext) -> Vec<AgentTip> {
    let mut tips = DEFAULT_TIPS.clone();

    if cfg!(feature = "voice_input")
        && UserWorkspaces::as_ref(ctx).is_voice_enabled()
        && AISettings::as_ref(ctx).is_voice_input_enabled(ctx)
    {
        tips.push(AgentTip {
            description_key: "agent.tips.voice",
            link: Some(
                "https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/voice"
                    .to_string(),
            ),
            binding_name: Some("FN"),
            action: None,
            kind: AgentTipKind::General,
        });
    }

    tips
}

/// A model for managing tips with cooldown logic.
/// Generic over any type implementing the AITip trait.
pub struct AITipModel<T: AITip> {
    tips: Vec<T>,
    current_tip: Option<T>,
    cooldown_handle: Option<SpawnedFutureHandle>,
}

impl<T: AITip + 'static> AITipModel<T> {
    /// Creates a new AITipModel with the given tips.
    /// Selects a random initial tip from the provided tips.
    ///
    /// # Panics
    /// Panics if the tips vector is empty.
    pub fn new(tips: Vec<T>) -> Self {
        use rand::seq::SliceRandom;
        debug_assert!(!tips.is_empty(), "AITipModel must have at least one tip");

        let mut rng = rand::thread_rng();
        let current_tip = tips.choose(&mut rng).cloned();

        Self {
            tips,
            current_tip,
            cooldown_handle: None,
        }
    }

    /// Returns the current tip, if one has been selected.
    pub fn current_tip(&self) -> Option<&T> {
        self.current_tip.as_ref()
    }
}

impl<T: AITip + 'static> Entity for AITipModel<T> {
    type Event = ();
}

// Specific implementation for AgentTip
impl AITipModel<AgentTip> {
    /// Creates a new AITipModel for AgentTips.
    /// This is the constructor used for the singleton model.
    pub fn new_for_agent_tips(ctx: &AppContext) -> Self {
        let tips = get_agent_tips(ctx);
        // Pick an applicable tip so we never show a raw "<keybinding>" placeholder on first render.
        let current_tip = Self::pick_random_applicable_tip(&tips, None, ctx);

        Self {
            tips,
            current_tip,
            cooldown_handle: None,
        }
    }

    /// Rebuilds the tip pool from current settings and invalidates the current tip
    /// if it is no longer applicable. Resets the cooldown timer so the revalidated
    /// tip is shown for the full cooldown period before the next rotation.
    pub fn revalidate_tips(&mut self, ctx: &mut ModelContext<Self>) {
        self.tips = get_agent_tips(ctx);

        // If the current tip is no longer in the pool or no longer applicable, pick a new one.
        let should_replace = self
            .current_tip
            .as_ref()
            .map(|current_tip| {
                let still_in_pool = self
                    .tips
                    .iter()
                    .any(|tip| tip.description_key == current_tip.description_key);

                !still_in_pool || !current_tip.is_tip_applicable(None, ctx)
            })
            .unwrap_or(true);

        if should_replace {
            let new_tip = Self::pick_random_applicable_tip(&self.tips, None, ctx);
            if new_tip.is_some() || self.current_tip.is_some() {
                self.current_tip = new_tip;
                self.reset_cooldown(ctx);
                ctx.notify();
            }
        }
    }

    /// Refreshes the current tip with a new random selection that is applicable
    /// for the given working directory.
    /// Only updates if not in cooldown period (60 seconds).
    pub fn maybe_refresh_tip(
        &mut self,
        current_working_directory: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Don't update if cooldown is active
        if self.cooldown_handle.is_some() {
            return;
        }

        // Rebuild tips from current settings so changes are picked up.
        self.tips = get_agent_tips(ctx);

        self.current_tip =
            Self::pick_random_applicable_tip(&self.tips, current_working_directory, ctx);

        // Start 60-second cooldown
        let handle = ctx.spawn(
            async {
                Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
        ctx.notify();
    }

    /// Picks a random applicable tip from the given pool, filtered by working directory.
    /// Returns `None` if no tips are applicable.
    fn pick_random_applicable_tip(
        tips: &[AgentTip],
        current_working_directory: Option<&str>,
        ctx: &AppContext,
    ) -> Option<AgentTip> {
        use rand::seq::SliceRandom;
        let available: Vec<&AgentTip> = tips
            .iter()
            .filter(|tip| tip.is_tip_applicable(current_working_directory, ctx))
            .collect();
        let mut rng = rand::thread_rng();
        available.choose(&mut rng).copied().cloned()
    }

    /// Resets the cooldown timer so the current tip is shown for the full
    /// cooldown period before the next rotation.
    fn reset_cooldown(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.cooldown_handle.take() {
            handle.abort();
        }
        let handle = ctx.spawn(
            async {
                Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
    }
}

impl SingletonEntity for AITipModel<AgentTip> {}

// Specific implementation for CloudModeTip
impl AITipModel<crate::terminal::view::ambient_agent::CloudModeTip> {
    /// Refreshes the current tip with a new random selection.
    /// Only updates if not in cooldown period (60 seconds).
    pub fn maybe_refresh_tip(&mut self, ctx: &mut ModelContext<Self>) {
        // Don't update if cooldown is active
        if self.cooldown_handle.is_some() {
            return;
        }

        use rand::seq::SliceRandom;

        // Select a random tip
        let mut rng = rand::thread_rng();
        self.current_tip = self.tips.choose(&mut rng).cloned();

        // Start 60-second cooldown
        let handle = ctx.spawn(
            async {
                Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
        ctx.notify();
    }

    /// Resets the cooldown timer without changing the current tip.
    /// This ensures the current tip will be shown for the full cooldown period.
    pub fn reset_cooldown(&mut self, ctx: &mut ModelContext<Self>) {
        // Cancel any existing cooldown
        if let Some(handle) = self.cooldown_handle.take() {
            handle.abort();
        }

        // Start a new 60-second cooldown
        let handle = ctx.spawn(
            async {
                Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
    }
}
