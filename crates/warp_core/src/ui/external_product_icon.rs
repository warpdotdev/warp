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
    pub fn from_string(s: &str) -> Option<ExternalProductIcon> {
        let normalized = Self::normalize_title(s);
        match normalized.as_str() {
            "heroku" => Some(ExternalProductIcon::Heroku),
            "notion" => Some(ExternalProductIcon::Notion),
            "linear" => Some(ExternalProductIcon::Linear),
            "figma" => Some(ExternalProductIcon::Figma),
            "github" => Some(ExternalProductIcon::Github),
            "slack" => Some(ExternalProductIcon::Slack),
            "composio" => Some(ExternalProductIcon::Composio),
            "resend" => Some(ExternalProductIcon::Resend),
            "sentry" => Some(ExternalProductIcon::Sentry),
            "you.com" => Some(ExternalProductIcon::YouDotCom),
            _other => None,
        }
    }

    /// Strips a single trailing parenthetical qualifier (e.g. "Sentry (OAuth)" ->
    /// "sentry") so decorated server titles still resolve to their base product's
    /// icon, then lowercases the result for matching. A title made up entirely of
    /// a parenthetical (no base text before it) is left untouched, since there is
    /// nothing to safely strip. This intentionally does not strip or match
    /// anywhere else in the title, so unrelated titles that merely contain a known
    /// product name (e.g. "GitHub scraper thing") still fall through to `None`.
    fn normalize_title(s: &str) -> String {
        let trimmed = s.trim();
        let base = match trimmed.rfind('(') {
            Some(paren_idx) if paren_idx > 0 && trimmed.ends_with(')') => {
                trimmed[..paren_idx].trim_end()
            }
            _ => trimmed,
        };
        base.to_ascii_lowercase()
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

#[cfg(test)]
#[path = "external_product_icon_tests.rs"]
mod tests;
