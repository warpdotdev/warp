//! Tips for cloud mode loading screen.

use warpui::keymap::Keystroke;
use warpui::AppContext;

use crate::ai::agent_tips::AITip;

/// A cloud mode tip with text and optional link.
#[derive(Clone, Debug)]
pub struct CloudModeTip {
    text: String,
    link: Option<String>,
}

impl CloudModeTip {
    pub fn new(text: impl Into<String>, link: Option<impl Into<String>>) -> Self {
        Self {
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

    // Uses the default implementation which adds "Tip: " prefix and parses backticks as inline code
}

/// Returns a collection of tips for the cloud mode loading screen.
pub fn get_cloud_mode_tips() -> Vec<CloudModeTip> {
    vec![
        CloudModeTip::new(
            "Install the Oz Slack integration to trigger agents from any channel or DM.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "Build programmatic agents using Oz's TypeScript and Python SDKs.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Set team or personal secrets for agents using the `oz secret` command.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "View all your agent runs and their status in the Oz web app.",
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            "Join any Oz cloud agent run in real-time using Agent Session Sharing.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "Set up recurring agents that run on cron schedules for automated maintenance.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Create agents that automatically fix bugs when issues are filed in Linear.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "Build agents that respond to CI failures and attempt automatic fixes.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "Run agents from GitHub Actions using the `oz-agent-action`.",
            Some("https://github.com/warpdotdev/oz-agent-action"),
        ),
        CloudModeTip::new(
            "Call the Oz REST API to trigger agents from any backend service or internal tool.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Create reusable environments with Docker images for consistent agent execution.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "Share agent session links with your team for collaborative debugging.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "Use the `--share` flag with the Oz CLI to enable session sharing from anywhere.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "Fork a completed Oz cloud agent session into Warp to continue the work locally.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "Build internal tools that use agents to answer questions from your databases.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations"),
        ),
        CloudModeTip::new(
            "Create a scheduled agent to clean up stale feature flags every week.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Tag @Oz in Linear issues to automatically investigate and propose fixes.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "Run agents on remote dev boxes or CI runners using the Oz CLI.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "Configure MCP servers to give Oz cloud agents access to GitHub, Linear, and Sentry.",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "Use `oz agent run` to kick off tasks without opening the Warp terminal.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "View your teammates' agent runs in the Oz web app for shared visibility.",
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            "Build agents that automatically triage and label incoming GitHub issues.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "Set up an agent to generate daily summaries of newly opened issues.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "Create an agent that automatically reviews PRs and suggests improvements.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "Use `oz environment create` to define reproducible execution contexts.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "Trigger agents from webhooks to respond to production incidents.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Build an agent that restarts services or scales deployments when alerts fire.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers"),
        ),
        CloudModeTip::new(
            "Use personal secrets for credentials that should only be used by your agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "Use team secrets for shared infrastructure credentials across all agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "Create an agent that runs nightly to check for dependency updates.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Build an agent that automatically formats and lints code on a schedule.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Use `oz schedule create` to set up cron-triggered agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Pause and resume scheduled agents without deleting them using `oz schedule pause`.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "Use `oz mcp list` to see which MCP servers are available to your agents.",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "Build an internal Slack bot that delegates coding tasks to Oz agents.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "Create an agent that responds to @mentions in Slack threads with full context.",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "Use the Oz TypeScript SDK to build custom automation pipelines.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Use the Oz Python SDK to integrate agents into your data pipelines.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Monitor agent success rates and runtimes using the Oz API.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "Build a dashboard that tracks all agent activity across your team.",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
    ]
}
