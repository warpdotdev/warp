use warpui_core::elements::Icon as WarpUiIcon;

use crate::ui::theme::Fill;

pub enum ExternalProductIcon {
    Heroku,
    Notion,
    Linear,
    Figma,
    Github,
    Slack,
    Composio,
    Resend,
    Sentry,
    YouDotCom,
}

impl ExternalProductIcon {
    /// Matches when the title starts with a known product name, case-insensitively,
    /// so decorated titles like "Sentry (OAuth)" still resolve to the base
    /// product's icon.
    pub fn from_string(s: &str) -> Option<ExternalProductIcon> {
        let s_lower = s.to_ascii_lowercase();
        if s_lower.starts_with("heroku") {
            Some(ExternalProductIcon::Heroku)
        } else if s_lower.starts_with("notion") {
            Some(ExternalProductIcon::Notion)
        } else if s_lower.starts_with("linear") {
            Some(ExternalProductIcon::Linear)
        } else if s_lower.starts_with("figma") {
            Some(ExternalProductIcon::Figma)
        } else if s_lower.starts_with("github") {
            Some(ExternalProductIcon::Github)
        } else if s_lower.starts_with("slack") {
            Some(ExternalProductIcon::Slack)
        } else if s_lower.starts_with("composio") {
            Some(ExternalProductIcon::Composio)
        } else if s_lower.starts_with("resend") {
            Some(ExternalProductIcon::Resend)
        } else if s_lower.starts_with("sentry") {
            Some(ExternalProductIcon::Sentry)
        } else if s_lower.starts_with("you.com") {
            Some(ExternalProductIcon::YouDotCom)
        } else {
            None
        }
    }

    pub fn get_path(&self) -> &'static str {
        match self {
            ExternalProductIcon::Heroku => "bundled/svg/heroku.svg",
            ExternalProductIcon::Notion => "bundled/svg/notion.svg",
            ExternalProductIcon::Linear => "bundled/svg/linear.svg",
            ExternalProductIcon::Figma => "bundled/svg/figma.svg",
            ExternalProductIcon::Github => "bundled/svg/github.svg",
            ExternalProductIcon::Slack => "bundled/svg/slack-logo.svg",
            ExternalProductIcon::Composio => "bundled/svg/composio.svg",
            ExternalProductIcon::Resend => "bundled/svg/resend.svg",
            ExternalProductIcon::Sentry => "bundled/svg/sentry.svg",
            ExternalProductIcon::YouDotCom => "bundled/svg/you-com.svg",
        }
    }

    pub fn to_warpui_icon(&self, color: Fill) -> WarpUiIcon {
        let path = self.get_path();
        WarpUiIcon::new(path, color.into_solid())
    }
}
