use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::features::FeatureFlag;

use super::Availability;
use crate::search::slash_command_menu::static_commands::Argument;
use crate::search::slash_command_menu::StaticCommand;
use crate::ui_components::color_dot;

pub static AGENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/agent",
    description: "Start a new conversation",
    icon_path: "bundled/svg/oz.svg",
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub static CLOUD_AGENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/cloud-agent",
    description: "Start a new cloud agent conversation",
    icon_path: "bundled/svg/oz-cloud.svg",
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub const ADD_MCP: StaticCommand = StaticCommand {
    name: "/add-mcp",
    description: "Add a new MCP server via the MCP settings page",
    icon_path: "bundled/svg/dataflow.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const PR_COMMENTS: StaticCommand = StaticCommand {
    name: "/pr-comments",
    description: "Pull GitHub PR review comments",
    icon_path: "bundled/svg/github.svg",
    availability: Availability::REPOSITORY.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: true,
    argument: None,
};

pub static CREATE_ENVIRONMENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/create-environment",
    description: "Create an Oz environment (Docker image + repos) via guided setup",
    icon_path: "bundled/svg/dataflow.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: Some(
        Argument::optional()
            .with_hint_text("<optional repo paths or GitHub URLs>")
            .with_execute_on_selection(),
    ),
});

pub const CREATE_DOCKER_SANDBOX: StaticCommand = StaticCommand {
    name: "/docker-sandbox",
    description: "Create a new docker sandbox terminal session",
    icon_path: "bundled/svg/docker.svg",
    availability: Availability::LOCAL.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub static CREATE_NEW_PROJECT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/create-new-project",
    description: "Have Oz walk you through creating a new coding project",
    icon_path: "bundled/svg/plus.svg",
    availability: Availability::LOCAL | Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: Some(Argument::required().with_hint_text("<describe what you want to build>")),
});

pub static EDIT_SKILL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/open-skill",
    description: "Open a skill's markdown file in Warp's built-in editor",
    icon_path: "bundled/svg/file-code-02.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub static INVOKE_SKILL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/skills",
    description: "Invoke a skill",
    icon_path: "bundled/svg/stars-01.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub static ADD_PROMPT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/add-prompt",
    description: "Add new Agent prompt",
    icon_path: if FeatureFlag::AgentView.is_enabled() {
        "bundled/svg/prompt.svg"
    } else {
        "bundled/svg/agentmode.svg"
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub const ADD_RULE: StaticCommand = StaticCommand {
    name: "/add-rule",
    description: "Add a new global rule for the agent",
    icon_path: "bundled/svg/book-open.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static EDIT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/open-file",
    description: "Open a file in Warp's code editor",
    icon_path: "bundled/svg/file-code-02.svg",
    availability: Availability::LOCAL,
    auto_enter_ai_mode: false,
    argument: Some(
        Argument::optional().with_hint_text("<path/to/file[:line[:col]]> or \"@\" to search"),
    ),
});

pub static RENAME_TAB: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/rename-tab",
    description: "Rename the current tab",
    icon_path: "bundled/svg/pencil-line.svg",
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::required().with_hint_text("<tab name>")),
});

pub static RENAME_CONVERSATION: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/rename-conversation",
    description: "Rename the current conversation",
    icon_path: "bundled/svg/pencil-line.svg",
    availability: Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: Some(Argument::required().with_hint_text("<new title>")),
});

static SET_TAB_COLOR_HINT: LazyLock<String> = LazyLock::new(|| {
    let mut hint = String::from("<");
    for color in color_dot::TAB_COLOR_OPTIONS {
        hint.push_str(&color.to_string().to_ascii_lowercase());
        hint.push('|');
    }
    hint.push_str("none>");
    hint
});

pub static SET_TAB_COLOR: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/set-tab-color",
    description: "Set the color of the current tab",
    icon_path: "bundled/svg/ellipse.svg",
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::required().with_hint_text(SET_TAB_COLOR_HINT.as_str())),
});

pub static FORK: LazyLock<StaticCommand> = LazyLock::new(|| {
    let hint_text = "<optional prompt to send in forked conversation>";
    StaticCommand {
        name: "/fork",
        description: "Fork the current conversation in a new pane or a new tab",
        icon_path: "bundled/svg/arrow-split.svg",
        availability: Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::NO_LRC_CONTROL
            | Availability::AI_ENABLED,
        auto_enter_ai_mode: true,
        argument: Some(Argument::optional().with_hint_text(hint_text)),
    }
});

pub static MOVE_TO_CLOUD: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/handoff",
    description: "Hand off this conversation to a cloud agent",
    icon_path: "bundled/svg/upload-cloud-01.svg",
    availability: Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: false,
    argument: Some(
        Argument::optional()
            .with_hint_text("<optional follow-up prompt>")
            .with_execute_on_selection(),
    ),
});

pub const OPEN_CODE_REVIEW: StaticCommand = StaticCommand {
    name: "/open-code-review",
    description: "Open code review",
    icon_path: "bundled/svg/diff.svg",
    availability: Availability::REPOSITORY,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const INDEX: StaticCommand = StaticCommand {
    name: "/index",
    description: "Index this codebase",
    icon_path: "bundled/svg/find-all.svg",
    availability: Availability::REPOSITORY
        .union(Availability::CODEBASE_CONTEXT)
        .union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const INIT: StaticCommand = StaticCommand {
    name: "/init",
    description: "Index this codebase and generate an AGENTS.md file",
    icon_path: "bundled/svg/warp-2.svg",
    availability: Availability::REPOSITORY
        .union(Availability::AGENT_VIEW)
        .union(Availability::AI_ENABLED),
    auto_enter_ai_mode: true,
    argument: None,
};

pub const OPEN_PROJECT_RULES: StaticCommand = StaticCommand {
    name: "/open-project-rules",
    description: "Open the project rules file (AGENTS.md)",
    icon_path: "bundled/svg/file-code-02.svg",
    availability: Availability::REPOSITORY.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_MCP_SERVERS: StaticCommand = StaticCommand {
    name: "/open-mcp-servers",
    description: "Open MCP servers",
    icon_path: "bundled/svg/dataflow.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_SETTINGS_FILE: StaticCommand = StaticCommand {
    name: "/open-settings-file",
    description: "Open settings file (TOML)",
    icon_path: "bundled/svg/file-code-02.svg",
    availability: Availability::LOCAL,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const CHANGELOG: StaticCommand = StaticCommand {
    name: "/changelog",
    description: "Open the latest changelog",
    icon_path: "bundled/svg/book-open.svg",
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: None,
};

// Accepts an optional argument so that buffers like `/feedback some text` still parse to
// this command (the trailing text is ignored on execution). Without this, typing any
// argument after `/feedback` would fall through and be treated as plain input.
pub static FEEDBACK: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/feedback",
    description: "Send feedback",
    icon_path: "bundled/svg/feedback.svg",
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub const OPEN_REPO: StaticCommand = StaticCommand {
    name: "/open-repo",
    description: "Switch to another indexed repository",
    icon_path: "bundled/svg/folder.svg",
    availability: Availability::LOCAL.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_RULES: StaticCommand = StaticCommand {
    name: "/open-rules",
    description: "View all of your global and project rules",
    icon_path: "bundled/svg/book-open.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static NEW: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/new",
    description: "Start a new conversation (alias for /agent)",
    icon_path: "bundled/svg/new-conversation.svg",
    availability: Availability::NO_LRC_CONTROL
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub static MODEL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/model",
    description: "Switch the base agent model",
    icon_path: "bundled/svg/oz.svg",
    availability: Availability::AGENT_VIEW | Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static HOST: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/host",
    description: "Switch the cloud agent execution host",
    icon_path: "bundled/svg/oz-cloud.svg",
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static HARNESS: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/harness",
    description: "Switch the cloud agent harness",
    icon_path: "bundled/svg/oz.svg",
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static ENVIRONMENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/environment",
    description: "Switch the cloud agent environment",
    icon_path: "bundled/svg/globe-04.svg",
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static PROFILE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/profile",
    description: "Switch the active execution profile",
    icon_path: "bundled/svg/psychology.svg",
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: true,
    argument: None,
});

pub const PLAN_NAME: &str = "/plan";

pub static PLAN: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: PLAN_NAME,
    description: "Prompt the agent to do some research and create a plan for a task",
    icon_path: "bundled/svg/file-06.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: Some(Argument::optional().with_hint_text("<describe your task>")),
});

pub const ORCHESTRATE_NAME: &str = "/orchestrate";

pub static ORCHESTRATE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: ORCHESTRATE_NAME,
    description: "Break a task into subtasks and run them in parallel with multiple agents",
    icon_path: "bundled/svg/oz.svg",
    availability: Availability::LOCAL | Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: Some(Argument::optional().with_hint_text("<describe your task>")),
});

/// If `query` starts with the given command `name` followed by a space,
/// returns the remainder of the query. Otherwise returns `None`.
pub fn strip_command_prefix(query: &str, name: &str) -> Option<String> {
    query
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix(' '))
        .map(|rest| rest.to_string())
}

pub static COMPACT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/compact",
    description: "Free up context by summarizing convo history",
    icon_path: "bundled/svg/collapse_content.svg",
    availability: Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::NO_LRC_CONTROL
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: true,
    argument: Some(
        Argument::optional().with_hint_text("<optional custom summarization instructions>"),
    ),
});

pub static COMPACT_AND: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/compact-and",
    description: "Compact conversation and then send a follow-up prompt",
    icon_path: "bundled/svg/collapse_content.svg",
    availability: Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::NO_LRC_CONTROL
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: true,
    argument: Some(Argument::optional().with_hint_text("<prompt to send after compaction>")),
});

pub static QUEUE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/queue",
    description: "Queue a prompt to send after the agent finishes responding",
    icon_path: "bundled/svg/clock-plus.svg",
    availability: Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: true,
    argument: Some(Argument::required().with_hint_text("<prompt to send when agent is done>")),
});

pub static FORK_AND_COMPACT: LazyLock<StaticCommand> = LazyLock::new(|| {
    let hint_text = "<optional prompt to send after compaction>";
    StaticCommand {
        name: "/fork-and-compact",
        description: "Fork current conversation and compact it in the forked copy",
        icon_path: "bundled/svg/fork_and_compact.svg",
        availability: Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::NO_LRC_CONTROL
            | Availability::AI_ENABLED
            | Availability::NOT_CLOUD_AGENT,
        auto_enter_ai_mode: true,
        argument: Some(Argument::optional().with_hint_text(hint_text)),
    }
});

pub const FORK_FROM: StaticCommand = StaticCommand {
    name: "/fork-from",
    description: "Fork conversation from a specific query",
    icon_path: "bundled/svg/arrow-split.svg",
    availability: Availability::AGENT_VIEW
        .union(Availability::NO_LRC_CONTROL)
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: true,
    argument: None,
};

pub static CONTINUE_LOCALLY: LazyLock<StaticCommand> = LazyLock::new(|| {
    let hint_text = "<optional prompt to send in local conversation>";
    StaticCommand {
        name: "/continue-locally",
        description: "Continue this cloud conversation locally",
        icon_path: "bundled/svg/arrow-split.svg",
        availability: Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::CLOUD_AGENT,
        auto_enter_ai_mode: true,
        argument: Some(Argument::optional().with_hint_text(hint_text)),
    }
});

pub const USAGE: StaticCommand = StaticCommand {
    name: "/usage",
    description: "Open billing and usage settings",
    icon_path: "bundled/svg/bar-chart-04.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const REMOTE_CONTROL: StaticCommand = StaticCommand {
    name: "/remote-control",
    description: "Start remote control for this session",
    icon_path: "bundled/svg/phone-01.svg",
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const COST: StaticCommand = StaticCommand {
    name: "/cost",
    description: "Toggle credit usage details",
    icon_path: "bundled/svg/bar-chart-04.svg",
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const CONVERSATIONS: StaticCommand = StaticCommand {
    name: "/conversations",
    description: "Open conversation history",
    icon_path: "bundled/svg/conversation.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static PROMPTS: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/prompts",
    description: "Search saved prompts",
    icon_path: "bundled/svg/prompt.svg",
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub const REWIND: StaticCommand = StaticCommand {
    name: "/rewind",
    description: "Rewind to a previous point in the conversation",
    icon_path: "bundled/svg/clock-rewind.svg",
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: true,
    argument: None,
};

pub const EXPORT_TO_CLIPBOARD: StaticCommand = StaticCommand {
    name: "/export-to-clipboard",
    description: "Export current conversation to clipboard in markdown format",
    icon_path: "bundled/svg/copy.svg",
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: true,
    argument: None,
};

pub static EXPORT_TO_FILE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/export-to-file",
    description: "Export current conversation to a markdown file",
    icon_path: "bundled/svg/download-01.svg",
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: true,
    argument: Some(Argument::optional().with_hint_text("<optional filename>")),
});

pub static COMMAND_REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

/// A unique identifier for a static slash command.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct SlashCommandId(Uuid);

impl SlashCommandId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SlashCommandId {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Registry {
    commands: HashMap<SlashCommandId, StaticCommand>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        let mut commands = HashMap::new();
        for command in all_commands().into_iter() {
            debug_assert!(
                !command
                    .availability
                    .contains(Availability::TERMINAL_VIEW | Availability::AGENT_VIEW),
                "command `{}` sets both TERMINAL_VIEW and AGENT_VIEW, which is unsatisfiable",
                command.name,
            );
            commands.insert(SlashCommandId::new(), command);
        }
        Self { commands }
    }

    pub fn all_commands_by_id(&self) -> impl Iterator<Item = (SlashCommandId, &StaticCommand)> {
        self.commands.iter().map(|(id, cmd)| (*id, cmd))
    }

    pub fn all_commands(&self) -> impl Iterator<Item = &StaticCommand> {
        self.commands.values()
    }

    pub fn get_command(&self, id: &SlashCommandId) -> Option<&StaticCommand> {
        self.commands.get(id)
    }

    pub fn get_command_with_name(&self, name: &str) -> Option<&StaticCommand> {
        self.commands.values().find(|command| command.name == name)
    }

    #[cfg(test)]
    pub fn get_command_id_with_name(&self, name: &str) -> Option<&SlashCommandId> {
        self.commands
            .iter()
            .find(|(_, command)| command.name == name)
            .map(|(id, _)| id)
    }
}

fn all_commands() -> Vec<StaticCommand> {
    let mut commands = vec![
        ADD_MCP,
        ADD_PROMPT.clone(),
        ADD_RULE,
        COST,
        FEEDBACK.clone(),
        INDEX,
        INIT,
        OPEN_PROJECT_RULES,
        OPEN_MCP_SERVERS,
        OPEN_RULES,
        AGENT.clone(),
        NEW.clone(),
        PLAN.clone(),
        RENAME_CONVERSATION.clone(),
        RENAME_TAB.clone(),
        SET_TAB_COLOR.clone(),
        USAGE,
        CONVERSATIONS,
        EXPORT_TO_CLIPBOARD,
        MODEL.clone(),
    ];

    if FeatureFlag::LocalDockerSandbox.is_enabled() {
        commands.push(CREATE_DOCKER_SANDBOX);
    }

    if FeatureFlag::CreatingSharedSessions.is_enabled()
        && FeatureFlag::HOARemoteControl.is_enabled()
    {
        commands.push(REMOTE_CONTROL);
    }

    if FeatureFlag::Changelog.is_enabled() {
        commands.push(CHANGELOG);
    }

    if FeatureFlag::AgentView.is_enabled() {
        commands.push(PROMPTS.clone());
    }

    commands.push(OPEN_CODE_REVIEW);

    if FeatureFlag::CreateEnvironmentSlashCommand.is_enabled() {
        commands.push(CREATE_ENVIRONMENT.clone());
    }

    if FeatureFlag::CreateProjectFlow.is_enabled() {
        commands.push(CREATE_NEW_PROJECT.clone());
    }

    if FeatureFlag::SummarizationConversationCommand.is_enabled() {
        commands.push(COMPACT.clone());
        commands.push(COMPACT_AND.clone());
    }

    if FeatureFlag::QueueSlashCommand.is_enabled() {
        commands.push(QUEUE.clone());
    }

    if !cfg!(target_family = "wasm") {
        commands.extend([
            FORK.clone(),
            FORK_AND_COMPACT.clone(),
            CONTINUE_LOCALLY.clone(),
        ]);

        if FeatureFlag::ForkFromCommand.is_enabled() {
            commands.push(FORK_FROM);
        }
    }

    if !cfg!(target_family = "wasm") {
        commands.extend([EDIT.clone(), EXPORT_TO_FILE.clone()]);
    }

    if FeatureFlag::ListSkills.is_enabled() && !cfg!(target_family = "wasm") {
        commands.push(EDIT_SKILL.clone());
        commands.push(INVOKE_SKILL.clone());
    }

    if FeatureFlag::PRCommentsSlashCommand.is_enabled()
        && !FeatureFlag::PRCommentsSkill.is_enabled()
    {
        commands.push(PR_COMMENTS);
    }

    if FeatureFlag::CloudMode.is_enabled() && FeatureFlag::CloudModeFromLocalSession.is_enabled() {
        commands.push(CLOUD_AGENT.clone());
    }

    if FeatureFlag::OzHandoff.is_enabled()
        && FeatureFlag::HandoffLocalCloud.is_enabled()
        && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
    {
        commands.push(MOVE_TO_CLOUD.clone());
    }

    if FeatureFlag::InlineProfileSelector.is_enabled() {
        commands.push(PROFILE.clone());
    }

    if FeatureFlag::RevertToCheckpoints.is_enabled() && FeatureFlag::RewindSlashCommand.is_enabled()
    {
        commands.push(REWIND);
    }

    if FeatureFlag::InlineRepoMenu.is_enabled() && !cfg!(target_family = "wasm") {
        commands.push(OPEN_REPO);
    }

    commands.push(ORCHESTRATE.clone());

    if FeatureFlag::SettingsFile.is_enabled() && cfg!(feature = "local_fs") {
        commands.push(OPEN_SETTINGS_FILE);
    }

    if FeatureFlag::CloudModeInputV2.is_enabled() {
        commands.push(HOST.clone());
        commands.push(HARNESS.clone());
        commands.push(ENVIRONMENT.clone());
    }

    commands
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
