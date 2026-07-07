use serde::{Deserialize, Serialize};
use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::workspace::tab_settings::TabSettings;

/// A configurable item in the vertical tabs header toolbar.
///
/// Each variant represents a panel toggle button that can be placed on either
/// the left or right side of the toolbar. The side determines which side of the
/// main content area the panel opens on.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(rename_all = "snake_case")]
pub enum HeaderToolbarItemKind {
    TabsPanel,
    ToolsPanel,
    AgentManagement,
    CodeReview,
    NotificationsMailbox,
}

impl HeaderToolbarItemKind {
    pub fn display_label(&self) -> &'static str {
        match self {
            Self::TabsPanel => "Tabs Panel",
            Self::ToolsPanel => "Tools Panel",
            Self::AgentManagement => "Agent Management",
            Self::CodeReview => "Code Review",
            Self::NotificationsMailbox => "Notifications",
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Self::TabsPanel => Icon::Menu,
            Self::ToolsPanel => Icon::Tool2,
            Self::AgentManagement => Icon::Grid,
            Self::CodeReview => Icon::Diff,
            Self::NotificationsMailbox => Icon::Inbox,
        }
    }

    /// Whether this item is supported on the current platform/configuration
    /// (feature flags, compile-time features, AI enabled, auth state).
    /// Does not check user show/hide preferences — use `is_available` for that.
    pub fn is_supported(&self, app: &AppContext) -> bool {
        match self {
            Self::TabsPanel => {
                FeatureFlag::VerticalTabs.is_enabled()
                    && *TabSettings::as_ref(app).use_vertical_tabs
            }
            Self::ToolsPanel => true,
            Self::AgentManagement => {
                let is_web_anonymous_user = AuthStateProvider::as_ref(app)
                    .get()
                    .is_user_web_anonymous_user()
                    .unwrap_or_default();
                AISettings::as_ref(app).is_any_ai_enabled(app)
                    && FeatureFlag::AgentManagementView.is_enabled()
                    && !is_web_anonymous_user
            }
            Self::CodeReview => cfg!(feature = "local_fs"),
            Self::NotificationsMailbox => FeatureFlag::HOANotifications.is_enabled(),
        }
    }

    /// Whether this item should be shown in the toolbar.
    /// Checks both `is_supported` and user show/hide preferences.
    pub fn is_available(&self, app: &AppContext) -> bool {
        if !self.is_supported(app) {
            return false;
        }
        match self {
            Self::CodeReview => *TabSettings::as_ref(app).show_code_review_button.value(),
            Self::NotificationsMailbox => *AISettings::as_ref(app).show_agent_notifications,
            _ => true,
        }
    }

    /// Whether this item opens a side panel (as opposed to replacing the content
    /// area or opening a popover).
    pub fn is_panel(&self) -> bool {
        matches!(self, Self::TabsPanel | Self::ToolsPanel | Self::CodeReview)
    }

    pub fn default_left() -> Vec<Self> {
        vec![Self::TabsPanel, Self::ToolsPanel, Self::AgentManagement]
    }

    pub fn default_right() -> Vec<Self> {
        vec![Self::CodeReview, Self::NotificationsMailbox]
    }

    /// All toolbar item variants (availability filtering is done at the call site).
    pub fn all_items() -> Vec<Self> {
        vec![
            Self::TabsPanel,
            Self::ToolsPanel,
            Self::AgentManagement,
            Self::CodeReview,
            Self::NotificationsMailbox,
        ]
    }
}
