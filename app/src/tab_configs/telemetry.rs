use serde::Serialize;
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::tab_configs::session_config::SessionType;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingTabConfigOpenMode {
    Direct,
    ParamsModal,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NewWorktreeConfigOpenSource {
    Submenu,
    NewWorktreeModal,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeBranchNamingMode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidedModalSessionType {
    Terminal,
    Oz,
    CliAgent,
}

impl From<&SessionType> for GuidedModalSessionType {
    fn from(value: &SessionType) -> Self {
        match value {
            SessionType::Terminal => Self::Terminal,
            SessionType::Oz => Self::Oz,
            SessionType::CliAgent(_) => Self::CliAgent,
        }
    }
}

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum TabConfigsTelemetryEvent {
    MenuCreateNewTabConfigClicked,
    ExistingConfigOpened {
        open_mode: ExistingTabConfigOpenMode,
        is_worktree_config: bool,
    },
    NewWorktreeConfigOpened {
        source: NewWorktreeConfigOpenSource,
        naming_mode: WorktreeBranchNamingMode,
    },
    GuidedModalOpened,
    GuidedModalSubmitted {
        session_type: GuidedModalSessionType,
        enable_worktree: bool,
        autogenerate_worktree_branch_name: bool,
    },
}

impl TelemetryEvent for TabConfigsTelemetryEvent {
    fn name(&self) -> &'static str {
        TabConfigsTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::MenuCreateNewTabConfigClicked | Self::GuidedModalOpened => None,
            Self::ExistingConfigOpened {
                open_mode,
                is_worktree_config,
            } => Some(json!({
                "open_mode": open_mode,
                "is_worktree_config": is_worktree_config,
            })),
            Self::NewWorktreeConfigOpened {
                source,
                naming_mode,
            } => Some(json!({
                "source": source,
                "naming_mode": naming_mode,
            })),
            Self::GuidedModalSubmitted {
                session_type,
                enable_worktree,
                autogenerate_worktree_branch_name,
            } => Some(json!({
                "session_type": session_type,
                "enable_worktree": enable_worktree,
                "autogenerate_worktree_branch_name": autogenerate_worktree_branch_name,
            })),
        }
    }

    fn description(&self) -> &'static str {
        TabConfigsTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TabConfigsTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::MenuCreateNewTabConfigClicked => false,
            Self::ExistingConfigOpened { .. } => false,
            Self::NewWorktreeConfigOpened { .. } => false,
            Self::GuidedModalOpened => false,
            Self::GuidedModalSubmitted { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TabConfigsTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::MenuCreateNewTabConfigClicked => "TabConfigs.MenuCreateNewTabConfigClicked",
            Self::ExistingConfigOpened => "TabConfigs.ExistingConfigOpened",
            Self::NewWorktreeConfigOpened => "TabConfigs.NewWorktreeConfigOpened",
            Self::GuidedModalOpened => "TabConfigs.GuidedModalOpened",
            Self::GuidedModalSubmitted => "TabConfigs.GuidedModalSubmitted",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::MenuCreateNewTabConfigClicked => {
                "User clicked the New tab config entry from the tab configs menu"
            }
            Self::ExistingConfigOpened => "User opened an existing saved tab config",
            Self::NewWorktreeConfigOpened => {
                "User opened a new worktree config from the submenu or new worktree modal"
            }
            Self::GuidedModalOpened => "User opened the guided Create a tab config modal",
            Self::GuidedModalSubmitted => "User submitted the guided Create a tab config modal",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::MenuCreateNewTabConfigClicked
            | Self::ExistingConfigOpened
            | Self::NewWorktreeConfigOpened
            | Self::GuidedModalOpened
            | Self::GuidedModalSubmitted => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(TabConfigsTelemetryEvent);
