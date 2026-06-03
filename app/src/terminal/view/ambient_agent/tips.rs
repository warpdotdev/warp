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
            i18n::t("terminal.ambient_agent.tips.slack_integration_trigger"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.programmatic_agents_sdk"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.set_secrets"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.view_runs_status"),
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.join_run_realtime"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.recurring_cron"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.linear_fix_bugs"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.ci_failures_fix"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.github_actions_agent_action"),
            Some("https://github.com/warpdotdev/oz-agent-action"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.rest_api_trigger"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.reusable_environments"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.share_session_links"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.share_flag"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.fork_completed_session"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/viewing-cloud-agent-runs"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.internal_tools_databases"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.scheduled_feature_flags"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.linear_tag_oz"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/linear"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.remote_dev_boxes"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.mcp_servers_access"),
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.oz_agent_run"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/platform"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.teammates_runs"),
            Some("https://oz.warp.dev"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.triage_github_issues"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.daily_issue_summaries"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.review_prs"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/github-actions"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.oz_environment_create"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/environments"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.webhooks_incidents"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.restart_services_alerts"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.personal_secrets"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.team_secrets"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/secrets"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.nightly_dependency_updates"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.format_lint_schedule"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.oz_schedule_create"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.oz_schedule_pause"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/triggers/scheduled-agents"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.oz_mcp_list"),
            Some("https://docs.warp.dev/agent-platform/capabilities/mcp"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.internal_slack_bot"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.slack_mentions"),
            Some("https://docs.warp.dev/agent-platform/cloud-agents/integrations/slack"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.typescript_sdk"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.python_sdk"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.monitor_success_rates"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
        CloudModeTip::new(
            i18n::t("terminal.ambient_agent.tips.dashboard_agent_activity"),
            Some("https://docs.warp.dev/reference/api-and-sdk"),
        ),
    ]
}
