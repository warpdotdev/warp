use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use warp_core::ui::theme::AnsiColorIdentifier;

pub const FILE_NAME: &str = "agent_tab_styles.yaml";
pub const ANNOTATED_DEFAULT: &str = r#"# Agent-state styling for the vertical-tabs sidebar.
# Colors are Warp's theme-aware ANSI colors: red, green, yellow, blue, magenta, cyan.
version: 1

states:
  idle:
    color: blue
    badge_size: regular
    layers: [tab_bg, badge_icon]
  processing:
    color: yellow
    badge_size: bigger
    layers: [tab_bg, badge_icon]
  success:
    color: green
    badge_size: big
    layers: [tab_bg, tab_text, badge_icon]
  needs_attention:
    color: red
    badge_size: bigger
    layers: [tab_bg, tab_text, badge_icon]

group_outline:
  # Empty disables automatic outlines. The group UUID chooses one entry stably.
  colors: [blue, magenta, cyan, green]
"#;

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTabColor {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
}

impl From<AgentTabColor> for AnsiColorIdentifier {
    fn from(value: AgentTabColor) -> Self {
        match value {
            AgentTabColor::Red => Self::Red,
            AgentTabColor::Green => Self::Green,
            AgentTabColor::Yellow => Self::Yellow,
            AgentTabColor::Blue => Self::Blue,
            AgentTabColor::Magenta => Self::Magenta,
            AgentTabColor::Cyan => Self::Cyan,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTabBadgeSize {
    Regular,
    Big,
    Bigger,
}

impl AgentTabBadgeSize {
    pub fn scale(self) -> f32 {
        match self {
            Self::Regular => 1.,
            Self::Big => 1.25,
            Self::Bigger => 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTabStyleLayer {
    TabBg,
    TabText,
    BadgeIcon,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentTabStateStyle {
    pub color: AgentTabColor,
    pub badge_size: AgentTabBadgeSize,
    pub layers: Vec<AgentTabStyleLayer>,
}

impl AgentTabStateStyle {
    pub fn has_layer(&self, layer: AgentTabStyleLayer) -> bool {
        self.layers.contains(&layer)
    }
}

impl Default for AgentTabStateStyle {
    fn default() -> Self {
        Self {
            color: AgentTabColor::Blue,
            badge_size: AgentTabBadgeSize::Regular,
            layers: vec![AgentTabStyleLayer::TabBg, AgentTabStyleLayer::BadgeIcon],
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentTabStates {
    pub idle: AgentTabStateStyle,
    pub processing: AgentTabStateStyle,
    pub success: AgentTabStateStyle,
    pub needs_attention: AgentTabStateStyle,
}

impl Default for AgentTabStates {
    fn default() -> Self {
        Self {
            idle: AgentTabStateStyle::default(),
            processing: AgentTabStateStyle {
                color: AgentTabColor::Yellow,
                badge_size: AgentTabBadgeSize::Bigger,
                ..AgentTabStateStyle::default()
            },
            success: AgentTabStateStyle {
                color: AgentTabColor::Green,
                badge_size: AgentTabBadgeSize::Big,
                layers: vec![
                    AgentTabStyleLayer::TabBg,
                    AgentTabStyleLayer::TabText,
                    AgentTabStyleLayer::BadgeIcon,
                ],
            },
            needs_attention: AgentTabStateStyle {
                color: AgentTabColor::Red,
                badge_size: AgentTabBadgeSize::Bigger,
                layers: vec![
                    AgentTabStyleLayer::TabBg,
                    AgentTabStyleLayer::TabText,
                    AgentTabStyleLayer::BadgeIcon,
                ],
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentTabGroupOutline {
    pub colors: Vec<AgentTabColor>,
}

impl Default for AgentTabGroupOutline {
    fn default() -> Self {
        Self {
            colors: vec![
                AgentTabColor::Blue,
                AgentTabColor::Magenta,
                AgentTabColor::Cyan,
                AgentTabColor::Green,
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentTabStyles {
    version: u8,
    pub states: AgentTabStates,
    pub group_outline: AgentTabGroupOutline,
}

impl Default for AgentTabStyles {
    fn default() -> Self {
        Self {
            version: 1,
            states: AgentTabStates::default(),
            group_outline: AgentTabGroupOutline::default(),
        }
    }
}

impl AgentTabStyles {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let mut value = serde_yaml::to_value(Self::default()).map_err(|error| error.to_string())?;
        merge_yaml(
            &mut value,
            serde_yaml::from_str(yaml).map_err(|error| error.to_string())?,
        );
        let config: Self = serde_yaml::from_value(value).map_err(|error| error.to_string())?;
        if config.version != 1 {
            return Err(format!(
                "unsupported version {}; expected 1",
                config.version
            ));
        }
        for (name, style) in [
            ("idle", &config.states.idle),
            ("processing", &config.states.processing),
            ("success", &config.states.success),
            ("needs_attention", &config.states.needs_attention),
        ] {
            if style.layers.len() != style.layers.iter().collect::<HashSet<_>>().len() {
                return Err(format!("state {name} contains duplicate layers"));
            }
        }
        if config.group_outline.colors.len()
            != config
                .group_outline
                .colors
                .iter()
                .collect::<HashSet<_>>()
                .len()
        {
            return Err("group_outline contains duplicate colors".to_string());
        }
        Ok(config)
    }
}

fn merge_yaml(base: &mut serde_yaml::Value, overlay: serde_yaml::Value) {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(base), serde_yaml::Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                if let Some(base_value) = base.get_mut(&key) {
                    merge_yaml(base_value, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
#[path = "agent_tab_styles_tests.rs"]
mod tests;
