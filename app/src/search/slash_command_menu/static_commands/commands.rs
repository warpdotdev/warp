use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::features::FeatureFlag;

use super::{Availability, SlashCommandKind, SlashCommandSurfaces};
use crate::search::slash_command_menu::StaticCommand;
use crate::search::slash_command_menu::static_commands::Argument;
use crate::ui_components::color_dot;

pub static AGENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/agent",
    description: "Start a new conversation",
    kind: SlashCommandKind::Agent,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/oz.svg",
    },
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub static CLOUD_AGENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/cloud-agent",
    description: "Start a new cloud agent conversation",
    kind: SlashCommandKind::CloudAgent,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/oz-cloud.svg",
    },
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub const ADD_MCP: StaticCommand = StaticCommand {
    name: "/add-mcp",
    description: "Add a new MCP server via the MCP settings page",
    kind: SlashCommandKind::AddMcp,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/dataflow.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const AUTO_APPROVE: StaticCommand = StaticCommand {
    name: "/auto-approve",
    description: "Toggle auto approve",
    kind: SlashCommandKind::AutoApprove,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::AGENT_VIEW
        .union(Availability::ACTIVE_CONVERSATION)
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const MCP: StaticCommand = StaticCommand {
    name: "/mcp",
    description: "View and manage MCP servers",
    kind: SlashCommandKind::Mcp,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const VIEW_LOGS: StaticCommand = StaticCommand {
    name: "/view-logs",
    description: "Bundle your logs into a zip archive",
    kind: SlashCommandKind::ViewLogs,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: None,
};

/// Starts the headless TUI voice-input session.
pub const VOICE: StaticCommand = StaticCommand {
    name: "/voice",
    description: "Start voice input (Ctrl-S)",
    kind: SlashCommandKind::Voice,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const NATURAL_LANGUAGE_DETECTION: StaticCommand = StaticCommand {
    name: "/natural-language-detection",
    description: "Toggle natural language detection",
    kind: SlashCommandKind::NaturalLanguageDetection,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const EXIT: StaticCommand = StaticCommand {
    name: "/exit",
    description: "Exit Warp",
    kind: SlashCommandKind::Exit,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const VERSION: StaticCommand = StaticCommand {
    name: "/version",
    description: "Show the Warp version",
    kind: SlashCommandKind::Version,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const LOGOUT: StaticCommand = StaticCommand {
    name: "/logout",
    description: "Log out of Warp",
    kind: SlashCommandKind::Logout,
    supported_surfaces: SlashCommandSurfaces::TuiOnly,
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static CREATE_ENVIRONMENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/create-environment",
    description: "Create an Oz environment (Docker image + repos) via guided setup",
    kind: SlashCommandKind::CreateEnvironment,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/dataflow.svg",
    },
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
    kind: SlashCommandKind::CreateDockerSandbox,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/docker.svg",
    },
    availability: Availability::LOCAL.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub static CREATE_NEW_PROJECT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/create-new-project",
    description: "Have Oz walk you through creating a new coding project",
    kind: SlashCommandKind::CreateNewProject,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/plus.svg",
    },
    availability: Availability::LOCAL | Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: Some(Argument::required().with_hint_text("<describe what you want to build>")),
});

pub static EDIT_SKILL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/open-skill",
    description: "Open a skill's markdown file in Warp's built-in editor",
    kind: SlashCommandKind::EditSkill,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/file-code-02.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub static INVOKE_SKILL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/skills",
    description: "Invoke a skill",
    kind: SlashCommandKind::InvokeSkill,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/stars-01.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub static ADD_PROMPT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/add-prompt",
    description: "Add new Agent prompt",
    kind: SlashCommandKind::AddPrompt,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: if FeatureFlag::AgentView.is_enabled() {
            "bundled/svg/prompt.svg"
        } else {
            "bundled/svg/agentmode.svg"
        },
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub const ADD_RULE: StaticCommand = StaticCommand {
    name: "/add-rule",
    description: "Add a new global rule for the agent",
    kind: SlashCommandKind::AddRule,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/book-open.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static EDIT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/open-file",
    description: "Open a file in Warp's code editor",
    kind: SlashCommandKind::Edit,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/file-code-02.svg",
    },
    availability: Availability::LOCAL,
    auto_enter_ai_mode: false,
    argument: Some(
        Argument::optional().with_hint_text("<path/to/file[:line[:col]]> or \"@\" to search"),
    ),
});

pub static RENAME_TAB: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/rename-tab",
    description: "Rename the current tab",
    kind: SlashCommandKind::RenameTab,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/pencil-line.svg",
    },
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::required().with_hint_text("<tab name>")),
});

pub static RENAME_CONVERSATION: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/rename-conversation",
    description: "Rename the current conversation",
    kind: SlashCommandKind::RenameConversation,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/pencil-line.svg",
    },
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
    kind: SlashCommandKind::SetTabColor,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/ellipse.svg",
    },
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::required().with_hint_text(SET_TAB_COLOR_HINT.as_str())),
});

pub static FORK: LazyLock<StaticCommand> = LazyLock::new(|| {
    let hint_text = "<optional prompt to send in forked conversation>";
    StaticCommand {
        name: "/fork",
        description: "Fork the current conversation in a new pane or a new tab",
        kind: SlashCommandKind::Fork,
        supported_surfaces: SlashCommandSurfaces::GuiOnly {
            icon_path: "bundled/svg/arrow-split.svg",
        },
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
    kind: SlashCommandKind::MoveToCloud,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/upload-cloud-01.svg",
    },
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
    kind: SlashCommandKind::OpenCodeReview,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/diff.svg",
    },
    availability: Availability::REPOSITORY,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const INDEX: StaticCommand = StaticCommand {
    name: "/index",
    description: "Index this codebase",
    kind: SlashCommandKind::Index,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/find-all.svg",
    },
    availability: Availability::REPOSITORY
        .union(Availability::CODEBASE_CONTEXT)
        .union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const INIT: StaticCommand = StaticCommand {
    name: "/init",
    description: "Index this codebase and generate an AGENTS.md file",
    kind: SlashCommandKind::Init,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/warp-2.svg",
    },
    availability: Availability::REPOSITORY
        .union(Availability::AGENT_VIEW)
        .union(Availability::AI_ENABLED),
    auto_enter_ai_mode: true,
    argument: None,
};

pub const OPEN_PROJECT_RULES: StaticCommand = StaticCommand {
    name: "/open-project-rules",
    description: "Open the project rules file (AGENTS.md)",
    kind: SlashCommandKind::OpenProjectRules,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/file-code-02.svg",
    },
    availability: Availability::REPOSITORY.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_MCP_SERVERS: StaticCommand = StaticCommand {
    name: "/open-mcp-servers",
    description: "Open MCP servers",
    kind: SlashCommandKind::OpenMcpServers,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/dataflow.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_SETTINGS_FILE: StaticCommand = StaticCommand {
    name: "/open-settings-file",
    description: "Open settings file (TOML)",
    kind: SlashCommandKind::OpenSettingsFile,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/file-code-02.svg",
    },
    availability: Availability::LOCAL,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const CHANGELOG: StaticCommand = StaticCommand {
    name: "/changelog",
    description: "Open the latest changelog",
    kind: SlashCommandKind::Changelog,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/book-open.svg",
    },
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
    kind: SlashCommandKind::Feedback,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/feedback.svg",
    },
    availability: Availability::ALWAYS,
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub const OPEN_REPO: StaticCommand = StaticCommand {
    name: "/open-repo",
    description: "Switch to another indexed repository",
    kind: SlashCommandKind::OpenRepo,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/folder.svg",
    },
    availability: Availability::LOCAL.union(Availability::AI_ENABLED),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const OPEN_RULES: StaticCommand = StaticCommand {
    name: "/open-rules",
    description: "View all of your global and project rules",
    kind: SlashCommandKind::OpenRules,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/book-open.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static NEW: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/new",
    description: "Start a new conversation (alias for /agent)",
    kind: SlashCommandKind::New,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/new-conversation.svg",
    },
    availability: Availability::NO_LRC_CONTROL
        | Availability::AI_ENABLED
        | Availability::NOT_CLOUD_AGENT,
    auto_enter_ai_mode: false,
    argument: Some(Argument::optional().with_execute_on_selection()),
});

pub static MODEL: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/model",
    description: "Switch the base agent model",
    kind: SlashCommandKind::Model,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/oz.svg",
    },
    availability: Availability::AGENT_VIEW | Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static HOST: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/host",
    description: "Switch the cloud agent execution host",
    kind: SlashCommandKind::Host,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/oz-cloud.svg",
    },
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static HARNESS: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/harness",
    description: "Switch the cloud agent harness",
    kind: SlashCommandKind::Harness,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/oz.svg",
    },
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static ENVIRONMENT: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/environment",
    description: "Switch the cloud agent environment",
    kind: SlashCommandKind::Environment,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/globe-04.svg",
    },
    availability: Availability::AGENT_VIEW
        | Availability::AI_ENABLED
        | Availability::CLOUD_MODE_V2_COMPOSER,
    auto_enter_ai_mode: true,
    argument: None,
});

pub static PROFILE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/profile",
    description: "Switch the active execution profile",
    kind: SlashCommandKind::Profile,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/psychology.svg",
    },
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
    kind: SlashCommandKind::Plan,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/file-06.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: true,
    argument: Some(Argument::optional().with_hint_text("<describe your task>")),
});

pub const ORCHESTRATE_NAME: &str = "/orchestrate";

pub static ORCHESTRATE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: ORCHESTRATE_NAME,
    description: "Break a task into subtasks and run them in parallel with multiple agents",
    kind: SlashCommandKind::Orchestrate,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/oz.svg",
    },
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
    kind: SlashCommandKind::Compact,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/collapse_content.svg",
    },
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
    kind: SlashCommandKind::CompactAnd,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/collapse_content.svg",
    },
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
    kind: SlashCommandKind::Queue,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/clock-plus.svg",
    },
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
        kind: SlashCommandKind::ForkAndCompact,
        supported_surfaces: SlashCommandSurfaces::GuiOnly {
            icon_path: "bundled/svg/fork_and_compact.svg",
        },
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
    kind: SlashCommandKind::ForkFrom,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/arrow-split.svg",
    },
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
        kind: SlashCommandKind::ContinueLocally,
        supported_surfaces: SlashCommandSurfaces::GuiOnly {
            icon_path: "bundled/svg/arrow-split.svg",
        },
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
    kind: SlashCommandKind::Usage,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/bar-chart-04.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub const REMOTE_CONTROL: StaticCommand = StaticCommand {
    name: "/remote-control",
    description: "Start remote control for this session",
    kind: SlashCommandKind::RemoteControl,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/phone-01.svg",
    },
    availability: Availability::AI_ENABLED.union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const COST: StaticCommand = StaticCommand {
    name: "/cost",
    description: "Toggle credit usage details",
    kind: SlashCommandKind::Cost,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/bar-chart-04.svg",
    },
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: false,
    argument: None,
};

pub const CONVERSATIONS: StaticCommand = StaticCommand {
    name: "/conversations",
    description: "Open conversation history",
    kind: SlashCommandKind::Conversations,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/conversation.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
};

pub static PROMPTS: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/prompts",
    description: "Search saved prompts",
    kind: SlashCommandKind::Prompts,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/prompt.svg",
    },
    availability: Availability::AI_ENABLED,
    auto_enter_ai_mode: false,
    argument: None,
});

pub const REWIND: StaticCommand = StaticCommand {
    name: "/rewind",
    description: "Rewind to a previous point in the conversation",
    kind: SlashCommandKind::Rewind,
    supported_surfaces: SlashCommandSurfaces::GuiOnly {
        icon_path: "bundled/svg/clock-rewind.svg",
    },
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: true,
    argument: None,
};

pub const EXPORT_TO_CLIPBOARD: StaticCommand = StaticCommand {
    name: "/export-to-clipboard",
    description: "Export current conversation to clipboard in markdown format",
    kind: SlashCommandKind::ExportToClipboard,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/copy.svg",
    },
    availability: Availability::AGENT_VIEW
        .union(Availability::AI_ENABLED)
        .union(Availability::NOT_CLOUD_AGENT),
    auto_enter_ai_mode: true,
    argument: None,
};

pub static EXPORT_TO_FILE: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
    name: "/export-to-file",
    description: "Export current conversation to a markdown file",
    kind: SlashCommandKind::ExportToFile,
    supported_surfaces: SlashCommandSurfaces::GuiAndTui {
        icon_path: "bundled/svg/download-01.svg",
    },
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
        for command in all_commands(settings::settings_mode()) {
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

fn all_commands(settings_mode: settings::SettingsMode) -> Vec<StaticCommand> {
    let mut commands = vec![
        ADD_MCP,
        ADD_PROMPT.clone(),
        ADD_RULE,
        AUTO_APPROVE,
        COST,
        EXIT,
        FEEDBACK.clone(),
        INDEX,
        INIT,
        LOGOUT,
        MCP,
        OPEN_PROJECT_RULES,
        OPEN_MCP_SERVERS,
        OPEN_RULES,
        AGENT.clone(),
        NEW.clone(),
        PLAN.clone(),
        RENAME_CONVERSATION.clone(),
        RENAME_TAB.clone(),
        SET_TAB_COLOR.clone(),
        NATURAL_LANGUAGE_DETECTION,
        USAGE,
        CONVERSATIONS,
        EXPORT_TO_CLIPBOARD,
        MODEL.clone(),
        VERSION,
        VIEW_LOGS,
        VOICE,
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
    commands.retain(|command| command.supports_surface(settings_mode));

    commands
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
