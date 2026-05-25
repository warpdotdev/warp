//! Tips for cloud mode loading screen.

use crate::ai::agent_tips::{tip_text_fragments, AITip};
use crate::localization;
use warpui::keymap::Keystroke;
use warpui::AppContext;

/// A cloud mode tip with text and optional link.
#[derive(Clone, Debug)]
pub struct CloudModeTip {
    text_key: &'static str,
    link: Option<String>,
}

impl CloudModeTip {
    pub fn new(text_key: &'static str, link: Option<impl Into<String>>) -> Self {
        Self {
            text_key,
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
        self.text_key
    }

    fn to_formatted_text(&self, app: &AppContext) -> Vec<markdown_parser::FormattedTextFragment> {
        let description = localization::text_for_app(app, self.text_key);
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
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.02",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.03",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new("terminal.ambient_agent.tip.04", Some("https://oz.warp.dev")),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.05",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.06",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.07",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.08",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.09",
            Some("https://github.com/warpdotdev/oz-agent-action"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.10",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.11",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.12",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.13",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.14",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.15",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.16",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.17",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.18",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.19",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.20",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new("terminal.ambient_agent.tip.21", Some("https://oz.warp.dev")),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.22",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.23",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.24",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.25",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.26",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.27",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.28",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.29",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.30",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.31",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.32",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.33",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.34",
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.35",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.36",
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.37",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.38",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.39",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            "terminal.ambient_agent.tip.40",
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
    ]
}
