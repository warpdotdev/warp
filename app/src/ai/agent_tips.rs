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
use warpui::r#async::SpawnedFutureHandle;
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
    fn to_formatted_text(&self, _app: &AppContext) -> Vec<FormattedTextFragment> {
        let text = format!("Tip: {}", self.description());

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
            description: "`/` to open the slash-command menu and access quick agent actions.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/slash-commands".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "<keybinding> to toggle natural language detection and switch between agent and terminal input.".to_string(),
            link: Some("https://docs.warp.dev/terminal/input/universal-input#input-modes".to_string()),
            binding_name: Some(SET_INPUT_MODE_AGENT_ACTION_NAME),
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "`/plan` <prompt> to create a plan for the agent before executing.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/planning".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "<keybinding> to open the Command Palette and access Warp actions and shortcuts.".to_string(),
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
            description: "Store reusable workflows, notebooks, and prompts in your".to_string(),
            link: Some("https://docs.warp.dev/knowledge-and-collaboration/warp-drive".to_string()),
            binding_name: None,
            action: Some(WorkspaceAction::OpenWarpDrive),
            kind: AgentTipKind::WarpDrive,
        },
        AgentTip {
            description: "Enter a new prompt to redirect the agent while it's running.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "`@` to add context from files, blocks, or Warp Drive objects to your prompt.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/using-to-add-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "<keybinding> to attach the prior command output as agent context.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context".to_string()),
            binding_name: Some(SELECT_PREVIOUS_BLOCK_ACTION_NAME),
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "`/init` to index the repo so the agent can understand your codebase.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/codebase-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::CodebaseContext,
        },
        AgentTip {
            description: "Add agent profiles to customize permissions and models per session.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Right-click a block to fork the conversation from that point.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Right-click a block to copy a conversation's output.".to_string(),
            link: Some("https://docs.warp.dev/terminal/blocks/block-actions#copy-input-output-of-block".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Drag an image into the pane to attach it as agent context.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/images-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "Prompt the agent to control interactive tools like node, python, postgres, gdb, or vim.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "<keybinding> to open the code review panel and review the agent's changes.".to_string(),
            link: Some("https://docs.warp.dev/code/code-review".to_string()),
            binding_name: Some(TOGGLE_RIGHT_PANEL_BINDING_NAME),
            action: None,
            kind: AgentTipKind::Code,
        },
        AgentTip {
            description: "`/add-mcp` to add an MCP server to your workspace.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/mcp".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Mcp,
        },
        AgentTip {
            description: "`/open-mcp-servers` to view and share MCP servers with your team.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::Mcp,
        },
        AgentTip {
            description: "`/create-environment` to turn a repo into a remote docker environment an agent can run in.".to_string(),
            link: Some("https://docs.warp.dev/reference/cli/integration-setup".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "`/add-prompt` to create a reusable prompt for repeatable workflows.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::WarpDrive,
        },
        AgentTip {
            description: "`/add-rule` to create a global agent rule.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "`/fork` to create a fresh copy of the current conversation, optionally with a new prompt.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents/conversation-forking".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "`/open-code-review` to open the code review panel and inspect agent-generated diffs.".to_string(),
            link: None,
            binding_name: None,
            action: Some(WorkspaceAction::ToggleRightPanel),
            kind: AgentTipKind::Code,
        },
        AgentTip {
            description: "`/new` to start a new agent conversation with clean context.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/interacting-with-agents".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "`/compact` to summarize the current conversation and free up space in the context window.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "`/usage` to show your current AI credits usage.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Use the `oz` command to run an Oz agent in headless mode, useful for remote machines.".to_string(),
            link: Some("https://docs.warp.dev/reference/cli".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Right-click selected text to attach it as agent context.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/blocks-as-context#attaching-blocks-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "Use `AGENTS.md` or `CLAUDE.md` to apply project-scoped rules.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules#project-rules-1".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "Paste a URL to attach that webpage as context for the agent.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/local-agents/agent-context/urls-as-context".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::Context,
        },
        AgentTip {
            description: "Warpify a remote SSH session to enable Oz inside that environment.".to_string(),
            link: Some("https://docs.warp.dev/terminal/warpify".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Switch agent profiles to quickly change models and agent permissions.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/agent-profiles-permissions".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "`/init` to generate a `WARP.md` file and define project rules for the agent.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/rules".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::SlashCommands,
        },
        AgentTip {
            description: "<keybinding> to auto-approve the agent's commands and diffs for the rest of the session.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/capabilities/full-terminal-use#session-level-approvals".to_string()),
            binding_name: Some(TOGGLE_AUTOEXECUTE_MODE_KEYBINDING),
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "Type `&` or use the handoff chip to move a local conversation to the cloud.".to_string(),
            link: None,
            binding_name: None,
            action: None,
            kind: AgentTipKind::Handoff,
        },
        AgentTip {
            description: "Enable desktop notifications to get an alert when an agent needs your attention.".to_string(),
            link: Some("https://docs.warp.dev/agent-platform/cloud-agents/managing-cloud-agents#in-app-agent-notifications".to_string()),
            binding_name: None,
            action: None,
            kind: AgentTipKind::General,
        },
        AgentTip {
            description: "<keybinding> to cancel the current agent task.".to_string(),
            link: None,
            binding_name: Some(CANCEL_COMMAND_KEYBINDING),
            action: None,
            kind: AgentTipKind::General,
        },
    ]
});

#[derive(Clone, Debug)]
pub struct AgentTip {
    /// The text that will be displayed to the user. This is parsed such that:
    /// "Tip: " is added as a prefix,
    /// "<keybinding>" is replaced with user-defined and platform-specific keybinding referenced by binding_name,
    /// `text` that is wrapped in backticks is formatted as inline code
    pub description: String,
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
        &self.description
    }

    fn to_formatted_text(&self, app: &AppContext) -> Vec<FormattedTextFragment> {
        let mut text = format!("Tip: {}", self.description);

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
        true
    }
}

impl WorkspaceAction {
    pub fn display_text(&self) -> Option<String> {
        match self {
            WorkspaceAction::OpenPalette { .. } => Some("Open palette".to_string()),
            WorkspaceAction::OpenWarpDrive => Some("Warp Drive.".to_string()),
            WorkspaceAction::ToggleRightPanel => Some("Show diff view".to_string()),
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
            description: "Hold <keybinding> to speak your prompt directly to the agent."
                .to_string(),
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
        Self::new(tips)
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

        use rand::seq::SliceRandom;

        // Filter applicable tips based on working directory
        let available_tips: Vec<AgentTip> = self
            .tips
            .iter()
            .filter(|tip| tip.is_tip_applicable(current_working_directory, ctx))
            .cloned()
            .collect();

        // Select a random tip
        let mut rng = rand::thread_rng();
        self.current_tip = available_tips.choose(&mut rng).cloned();

        // Start 60-second cooldown
        let handle = ctx.spawn(
            async {
                warpui::r#async::Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
        ctx.notify();
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
                warpui::r#async::Timer::after(Duration::from_secs(60)).await;
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
                warpui::r#async::Timer::after(Duration::from_secs(60)).await;
            },
            |me, _, _| {
                me.cooldown_handle = None;
            },
        );
        self.cooldown_handle = Some(handle);
    }
}
