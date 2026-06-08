use warpui_core::elements::Icon as WarpUiIcon;

use crate::ui::theme::Fill;

pub enum ExternalProductIcon {
    Heroku,
    Notion,
    Linear,
    Figma,
    Github,
    Slack,
}

impl ExternalProductIcon {
    pub fn from_string(s: &str) -> Option<ExternalProductIcon> {
        let s_lower = s.to_ascii_lowercase();
        match s_lower.as_str() {
            "heroku" => Some(ExternalProductIcon::Heroku),
            "notion" => Some(ExternalProductIcon::Notion),
            "linear" => Some(ExternalProductIcon::Linear),
            "figma" => Some(ExternalProductIcon::Figma),
            "github" => Some(ExternalProductIcon::Github),
            "slack" => Some(ExternalProductIcon::Slack),
            _other => None,
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
        }
    }

    pub fn to_warpui_icon(&self, color: Fill) -> WarpUiIcon {
        let path = self.get_path();
        WarpUiIcon::new(path, color.into_solid())
    }
}
