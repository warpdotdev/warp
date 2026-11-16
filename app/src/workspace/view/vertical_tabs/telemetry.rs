use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::workspace::tab_settings::{
    VerticalTabsCompactSubtitle, VerticalTabsDisplayGranularity, VerticalTabsPrimaryInfo,
    VerticalTabsTabItemMode, VerticalTabsViewMode,
};

/// Which display option on the vertical tabs settings popup the user changed,
/// along with the new value they picked.
#[derive(Clone, Copy, Debug)]
pub enum VerticalTabsDisplayOption {
    DisplayGranularity(VerticalTabsDisplayGranularity),
    TabItemMode(VerticalTabsTabItemMode),
    ViewMode(VerticalTabsViewMode),
    PrimaryInfo(VerticalTabsPrimaryInfo),
    CompactSubtitle(VerticalTabsCompactSubtitle),
    ShowPrLink(bool),
    ShowDiffStats(bool),
    ShowDetailsOnHover(bool),
}

impl VerticalTabsDisplayOption {
    fn option_name(&self) -> &'static str {
        match self {
            Self::DisplayGranularity(_) => "display_granularity",
            Self::TabItemMode(_) => "tab_item_mode",
            Self::ViewMode(_) => "view_mode",
            Self::PrimaryInfo(_) => "primary_info",
            Self::CompactSubtitle(_) => "compact_subtitle",
            Self::ShowPrLink(_) => "show_pr_link",
            Self::ShowDiffStats(_) => "show_diff_stats",
            Self::ShowDetailsOnHover(_) => "show_details_on_hover",
        }
    }

    fn serialized_value(&self) -> Value {
        match self {
            Self::DisplayGranularity(VerticalTabsDisplayGranularity::Panes) => json!("panes"),
            Self::DisplayGranularity(VerticalTabsDisplayGranularity::Tabs) => json!("tabs"),
            Self::TabItemMode(VerticalTabsTabItemMode::FocusedSession) => json!("focused_session"),
            Self::TabItemMode(VerticalTabsTabItemMode::Summary) => json!("summary"),
            Self::ViewMode(VerticalTabsViewMode::Compact) => json!("compact"),
            Self::ViewMode(VerticalTabsViewMode::Expanded) => json!("expanded"),
            Self::PrimaryInfo(VerticalTabsPrimaryInfo::Command) => json!("command"),
            Self::PrimaryInfo(VerticalTabsPrimaryInfo::WorkingDirectory) => {
                json!("working_directory")
            }
            Self::PrimaryInfo(VerticalTabsPrimaryInfo::Branch) => json!("branch"),
            Self::CompactSubtitle(VerticalTabsCompactSubtitle::Branch) => json!("branch"),
            Self::CompactSubtitle(VerticalTabsCompactSubtitle::WorkingDirectory) => {
                json!("working_directory")
            }
            Self::CompactSubtitle(VerticalTabsCompactSubtitle::Command) => json!("command"),
            Self::ShowPrLink(value) => json!(value),
            Self::ShowDiffStats(value) => json!(value),
            Self::ShowDetailsOnHover(value) => json!(value),
        }
    }
}

/// Where in the vertical tabs UI a clickable diff-stats or GitHub PR chip
/// was rendered when the user clicked it.
#[derive(Clone, Copy, Debug)]
pub enum VerticalTabsChipEntrypoint {
    /// The chip was rendered on a row representing a single pane
    /// (display granularity: Panes).
    Pane,
    /// The chip was rendered on a row representing a tab group
    /// (display granularity: Tabs).
    Tab,
    /// The chip was rendered inside the detail sidecar that appears on row hover.
    DetailsSidecar,
}

impl VerticalTabsChipEntrypoint {
    fn serialized(&self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Tab => "tab",
            Self::DetailsSidecar => "details_sidecar",
        }
    }
}

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum VerticalTabsTelemetryEvent {
    /// The user updated a display option in the vertical tabs settings popup.
    DisplayOptionChanged(VerticalTabsDisplayOption),
    /// The user clicked the diff stats chip on a vertical tabs row or the detail sidecar.
    DiffStatsChipClicked {
        entrypoint: VerticalTabsChipEntrypoint,
    },
    /// The user clicked the GitHub PR chip on a vertical tabs row or the detail sidecar.
    PrChipClicked {
        entrypoint: VerticalTabsChipEntrypoint,
    },
}

impl TelemetryEvent for VerticalTabsTelemetryEvent {
    fn name(&self) -> &'static str {
        VerticalTabsTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::DisplayOptionChanged(option) => Some(json!({
                "option": option.option_name(),
                "value": option.serialized_value(),
            })),
            Self::DiffStatsChipClicked { entrypoint } => Some(json!({
                "entrypoint": entrypoint.serialized(),
            })),
            Self::PrChipClicked { entrypoint } => Some(json!({
                "entrypoint": entrypoint.serialized(),
            })),
        }
    }

    fn description(&self) -> &'static str {
        VerticalTabsTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        VerticalTabsTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for VerticalTabsTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::DisplayOptionChanged => "VerticalTabs.DisplayOptionChanged",
            Self::DiffStatsChipClicked => "VerticalTabs.DiffStatsChipClicked",
            Self::PrChipClicked => "VerticalTabs.PrChipClicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::DisplayOptionChanged => {
                "User updated a display option in the vertical tabs settings popup"
            }
            Self::DiffStatsChipClicked => {
                "User clicked a diff stats chip in the vertical tabs panel or detail sidecar"
            }
            Self::PrChipClicked => {
                "User clicked a GitHub PR chip in the vertical tabs panel or detail sidecar"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Flag(FeatureFlag::VerticalTabs)
    }
}

warp_core::register_telemetry_event!(VerticalTabsTelemetryEvent);
