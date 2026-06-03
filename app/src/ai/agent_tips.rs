use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use markdown_parser::FormattedTextFragment;
use warpui::keymap::Keystroke;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

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
    /// Default implementation adds a localized tip prefix and parses backtick-wrapped text as inline code.
    fn to_formatted_text(&self, _app: &AppContext) -> Vec<FormattedTextFragment> {
        let text = i18n::t("ai.agent_tips.tip_prefix").replace("{description}", self.description());

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
    vec![
        AgentTip {
            description_key: "ai.agent_tips.slash_command_menu",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/slash-commands".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.toggle_natural_language_detection",
            link: Some("https://docs.warp.dev/terminal/input/universal-input#input-modes".to_string()),
            binding_name: Some(SET_INPUT_MODE_AGENT_ACTION_NAME),
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.plan_prompt",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/planning".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.open_command_palette",
            link: Some("https://docs.warp.dev/terminal/command-palette".to_string()),
            binding_name: Some(TOGGLE_COMMAND_PALETTE_KEYBINDING_NAME),
            action: Some(WorkspaceAction::OpenPalette {
                mode: PaletteMode::Command,
                source: PaletteSource::AgentTip,
                query: None,
            }),
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.store_reusable_objects",
            link: Some("https://docs.warp.dev/knowledge-and-collaboration/warp-drive".to_string()),
            binding_name: None,
            action: Some(WorkspaceAction::OpenWarpDrive),
            kind: AgentTipKind::WarpDrive,
        },
        AgentTip {
            description_key: "ai.agent_tips.redirect_running_agent",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.at_add_context",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/using-to-add-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.attach_prior_command_output",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context".to_string()),
            binding_name: Some(SELECT_PREVIOUS_BLOCK_ACTION_NAME),
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.init_index_repo",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/codebase-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::CodebaseContext,
        },
        AgentTip {
            description_key: "ai.agent_tips.agent_profiles",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.right_click_fork",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.right_click_copy_output",
            link: Some("https://docs.warp.dev/terminal/blocks/block-actions#copy-input-output-of-block".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.drag_image_context",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/images-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.control_interactive_tools",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.open_code_review_panel",
            link: Some("https://docs.warp.dev/code/code-review".to_string()),
            binding_name: Some(TOGGLE_RIGHT_PANEL_BINDING_NAME),
            action: None,
            kind: AgentTipKind::Code,
        },
        AgentTip {
            description_key: "ai.agent_tips.add_mcp",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/mcp".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Mcp,
        },
        AgentTip {
            description_key: "ai.agent_tips.open_mcp_servers",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::Mcp,
        },
        AgentTip {
            description_key: "ai.agent_tips.create_environment",
            link: Some("https://docs.warp.dev/reference/cli/integration-setup".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.add_prompt",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::WarpDrive,
        },
        AgentTip {
            description_key: "ai.agent_tips.add_rule",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.fork_conversation",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.open_code_review_command",
            link: None,
            binding_name: None,
            action: Some(WorkspaceAction::ToggleRightPanel),
            kind: AgentTipKind::Code,
        },
        AgentTip {
            description_key: "ai.agent_tips.new_conversation",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.compact_conversation",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.usage",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.oz_headless",
            link: Some("https://docs.warp.dev/reference/cli".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.right_click_selected_text",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.project_rules_files",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules#project-rules-1".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.paste_url_context",
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/urls-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description_key: "ai.agent_tips.warpify_ssh",
            link: Some("https://docs.warp.dev/terminal/warpify".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.switch_agent_profiles",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.init_generate_warp_md",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description_key: "ai.agent_tips.auto_approve_session",
            link: Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use#session-level-approvals".to_string()),
            binding_name: Some(TOGGLE_AUTOEXECUTE_MODE_KEYBINDING),
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.handoff_to_cloud",
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::Handoff,
        },
        AgentTip {
            description_key: "ai.agent_tips.enable_desktop_notifications",
            link: Some("https://docs.warp.dev/agent-platform/cloud-agents/managing-cloud-agents#in-app-agent-notifications".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description_key: "ai.agent_tips.cancel_agent_task",
            link: None,
            binding_name: Some(CANCEL_COMMAND_KEYBINDING),
            action: None,
            kind: AgentTipKind::General,
        },
    ]
});

#[derive(Clone, Debug)]
pub struct AgentTip {
    /// The locale key for the text that will be displayed to the user. This is parsed such that:
    /// a localized tip prefix is added,
    /// "<keybinding>" is replaced with user-defined and platform-specific keybinding referenced by binding_name,
    /// `text` that is wrapped in backticks is formatted as inline code
    pub description_key: &'static str,
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
        let mut text = i18n::t("ai.agent_tips.tip_prefix")
            .replace("{description}", &i18n::t(self.description_key));

        // Replace <keybinding> with the actual keybinding string
        if let Some(keystroke) = self.keystroke(app) {
            text = text.replace("<keybinding>", &keystroke.displayed());
        }

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
        if i18n::t(self.description_key).contains("<keybinding>") && self.keystroke(app).is_none() {
            return false;
        }
        true
    }
}

impl WorkspaceAction {
    pub fn display_text(&self) -> Option<String> {
        match self {
            WorkspaceAction::OpenPalette { .. } => {
                Some(i18n::t("ai.agent_tips.action.open_palette"))
            }
            WorkspaceAction::OpenWarpDrive => Some(i18n::t("ai.agent_tips.action.warp_drive")),
            WorkspaceAction::ToggleRightPanel => {
                Some(i18n::t("ai.agent_tips.action.show_diff_view"))
            }
            _ => None,
        }
    }
}

/// Helper function to build the list of agent tips, including the voice tip if enabled.
pub fn get_agent_tips(ctx: &AppContext) -> Vec<AgentTip> {
    let mut tips = DEFAULT_TIPS.clone();

    if cfg!(feature = "voice_input")
        && UserWorkspaces::as_ref(ctx).is_voice_enabled()
        && AISettings::as_ref(ctx).is_voice_input_enabled(ctx)
    {
        tips.push(AgentTip {
            description_key: "ai.agent_tips.voice_input",
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
