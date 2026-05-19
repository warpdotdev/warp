//! Tips for cloud mode loading screen.

use crate::ai::agent_tips::{tip_text_fragments, AITip};
use crate::localization;
use warpui::keymap::Keystroke;
use warpui::AppContext;

/// A cloud mode tip with text and optional link.
#[derive(Clone, Debug)]
pub struct CloudModeTip {
    text_key: &'static str,
    text: String,
    link: Option<String>,
}

impl CloudModeTip {
    pub fn new(
        text_key: &'static str,
        text: impl Into<String>,
        link: Option<impl Into<String>>,
    ) -> Self {
        Self {
            text_key,
            text: text.into(),
            link: link.map(|l| l.into()),
        }
    }
}

impl AITip for CloudModeTip {
    fn keystroke(&self, _app: &AppContext) -> Option<Keystroke> {
        None
    }

    fn link(&self) -> Option<String> {
        self.link.clone()
    }

    fn description(&self) -> &str {
        &self.text
    }

    fn to_formatted_text(&self, app: &AppContext) -> Vec<markdown_parser::FormattedTextFragment> {
        let description = localization::text_for_app_or(app, self.text_key, &self.text);
        tip_text_fragments(format!(
            "{}{}",
            localization::text_for_app(app, "agent.tips.prefix"),
            description
        ))
    }
}

/// Returns a collection of tips for the cloud mode loading screen.
pub fn get_cloud_mode_tips() -> Vec<CloudModeTip> {
    vec![
        CloudModeTip::new(
            "terminal.ambient_agent.tip.01",
            "Install the Oz Slack integration to trigger agents from any channel or DM.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.02",
            "Build programmatic agents using Oz's TypeScript and Python SDKs.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.03",
            "Set team or personal secrets for agents using the `oz secret` command.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.04",
            "View all your agent runs and their status in the Oz web app.",
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.05",
            "Join any Oz cloud agent run in real-time using Agent Session Sharing.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.06",
            "Set up recurring agents that run on cron schedules for automated maintenance.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.07",
            "Create agents that automatically fix bugs when issues are filed in Linear.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.08",
            "Build agents that respond to CI failures and attempt automatic fixes.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.09",
            "Run agents from GitHub Actions using the `oz-agent-action`.",
            Some("https://github.com/warpdotdev/oz-agent-action"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.10",
            "Call the Oz REST API to trigger agents from any backend service or internal tool.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.11",
            "Create reusable environments with Docker images for consistent agent execution.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.12",
            "Share agent session links with your team for collaborative debugging.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.13",
            "Use the `--share` flag with the Oz CLI to enable session sharing from anywhere.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.14",
            "Fork a completed Oz cloud agent session into Warp to continue the work locally.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.15",
            "Build internal tools that use agents to answer questions from your databases.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.16",
            "Create a scheduled agent to clean up stale feature flags every week.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.17",
            "Tag @Oz in Linear issues to automatically investigate and propose fixes.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.18",
            "Run agents on remote dev boxes or CI runners using the Oz CLI.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.19",
            "Configure MCP servers to give Oz cloud agents access to GitHub, Linear, and Sentry.",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.20",
            "Use `oz agent run` to kick off tasks without opening the Warp terminal.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.21",
            "View your teammates' agent runs in the Oz web app for shared visibility.",
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.22",
            "Build agents that automatically triage and label incoming GitHub issues.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.23",
            "Set up an agent to generate daily summaries of newly opened issues.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.24",
            "Create an agent that automatically reviews PRs and suggests improvements.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.25",
            "Use `oz environment create` to define reproducible execution contexts.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.26",
            "Trigger agents from webhooks to respond to production incidents.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.27",
            "Build an agent that restarts services or scales deployments when alerts fire.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.28",
            "Use personal secrets for credentials that should only be used by your agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.29",
            "Use team secrets for shared infrastructure credentials across all agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.30",
            "Create an agent that runs nightly to check for dependency updates.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.31",
            "Build an agent that automatically formats and lints code on a schedule.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.32",
            "Use `oz schedule create` to set up cron-triggered agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.33",
            "Pause and resume scheduled agents without deleting them using `oz schedule pause`.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.34",
            "Use `oz mcp list` to see which MCP servers are available to your agents.",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.35",
            "Build an internal Slack bot that delegates coding tasks to Oz agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.36",
            "Create an agent that responds to @mentions in Slack threads with full context.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.37",
            "Use the Oz TypeScript SDK to build custom automation pipelines.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.38",
            "Use the Oz Python SDK to integrate agents into your data pipelines.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.39",
            "Monitor agent success rates and runtimes using the Oz API.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.40",
            "Build a dashboard that tracks all agent activity across your team.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
    ]
}
