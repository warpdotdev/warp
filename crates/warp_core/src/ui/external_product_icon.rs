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

    /// Strips a single, fully-balanced trailing parenthetical qualifier (e.g.
    /// "Sentry (OAuth)" -> "sentry", "Sentry (OAuth (work))" -> "sentry") so
    /// decorated server titles still resolve to their base product's icon, then
    /// lowercases the result for matching. This intentionally does not strip or
    /// match anywhere else in the title, so unrelated titles that merely contain
    /// a known product name (e.g. "GitHub scraper thing") still fall through to
    /// `None`.
    fn normalize_title(s: &str) -> String {
        let trimmed = s.trim();
        let base = Self::strip_trailing_parenthetical(trimmed).unwrap_or(trimmed);
        base.to_ascii_lowercase()
    }

    /// Returns the text preceding a fully-balanced trailing parenthetical group,
    /// e.g. "Sentry (OAuth)" -> Some("Sentry"), "Sentry (OAuth (work))" ->
    /// Some("Sentry"). Returns `None` when the title doesn't end with a balanced
    /// group (an unmatched or extra closing parenthesis, such as
    /// "Sentry (OAuth))") or when there is no base text before it (e.g.
    /// "(OAuth)"), leaving those titles unmodified rather than guessing.
    fn strip_trailing_parenthetical(s: &str) -> Option<&str> {
        if !s.ends_with(')') {
            return None;
        }

        let mut depth = 0i32;
        let mut open_idx = None;
        for (idx, ch) in s.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        open_idx = Some(idx);
                        break;
                    }
                }
                _ => {}
            }
        }

        let base = s[..open_idx?].trim_end();
        if base.is_empty() { None } else { Some(base) }
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
