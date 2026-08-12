use warpui_core::elements::Icon as WarpUiIcon;

use crate::ui::theme::Fill;

#[derive(Clone, Copy)]
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
    /// Product name prefixes, matched case-insensitively against the start of a
    /// server title. Add a new product by adding a row here.
    const PREFIXES: &'static [(&'static str, ExternalProductIcon)] = &[
        ("heroku", ExternalProductIcon::Heroku),
        ("notion", ExternalProductIcon::Notion),
        ("linear", ExternalProductIcon::Linear),
        ("figma", ExternalProductIcon::Figma),
        ("github", ExternalProductIcon::Github),
        ("slack", ExternalProductIcon::Slack),
        ("composio", ExternalProductIcon::Composio),
        ("resend", ExternalProductIcon::Resend),
        ("sentry", ExternalProductIcon::Sentry),
        ("you.com", ExternalProductIcon::YouDotCom),
    ];

    /// Matches when the title starts with a known product name, case-insensitively,
    /// so decorated titles like "Sentry (OAuth)" still resolve to the base
    /// product's icon.
    pub fn from_string(s: &str) -> Option<ExternalProductIcon> {
        let s_lower = s.to_ascii_lowercase();
        Self::PREFIXES
            .iter()
            .find(|(prefix, _)| s_lower.starts_with(prefix))
            .map(|(_, icon)| *icon)
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
