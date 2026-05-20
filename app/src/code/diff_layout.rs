use serde::{Deserialize, Serialize};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "Layout used to render Code Review diffs.",
    rename_all = "snake_case"
)]
pub enum DiffLayout {
    #[default]
    Inline,
    SideBySide,
}

impl DiffLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inline => "Inline",
            Self::SideBySide => "Side by side",
        }
    }

    pub fn is_side_by_side(self) -> bool {
        matches!(self, Self::SideBySide)
    }
}
