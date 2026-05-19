//! ContextFlag flags are for behaviors that need to be conditionally enabled or disabled based
//! on where the app is being run and are a permanent part of the app.

use std::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use enum_iterator::{cardinality, Sequence};

use crate::channel::ChannelState;

/// All ContextFlag flag are enabled by default. Environments can conditionally disable flags.
///
/// Aside from manually setting specific flags in dogfood contexts, the complete list of contexts
/// this is used in is found in the ContextFlag impl.
#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug, Sequence)]
pub enum ContextFlag {
    CreateSharedSession,
    CreateNewSession,
    CloseWindow,
    ForceSidePanelOpen,
    ShowRewardModal,
    HideOpenOnDesktopButton,
    PromptForVersionUpdates,
    NetworkLogConsole,
    RunWorkflow,
    LaunchConfigurations,
    WarpEssentials,
    AllowSettingsModalToClose,
    ShowSlowShellStartupBanner,
    DynamicBrowserUrl,
    ShowMCPServers,
}

/// The enablement states for context flags.  As mentioned in the documentation
/// for [`ContextFlag`], these are enabled by default.
static FLAG_STATES: [AtomicBool; cardinality::<ContextFlag>()] =
    [const { AtomicBool::new(true) }; { cardinality::<ContextFlag>() }];

fn disable_flag(flag: ContextFlag) {
    FLAG_STATES[flag as usize].store(false, Ordering::Relaxed);
}

impl ContextFlag {
    pub fn is_enabled(&self) -> bool {
        FLAG_STATES[*self as usize].load(Ordering::Relaxed)
    }

    /// Sets a ContextFlag flag. FOR DEBUG USE ONLY.
    pub fn set(&self, value: bool) {
        if !ChannelState::enable_debug_features() {
            log::error!(
                "Tried to set value of `ContextFlag` flag `{self:?}` in non-dogfood context."
            );
        }

        FLAG_STATES[*self as usize].store(value, Ordering::Relaxed);
    }

    pub fn set_warp_home_link_only() {
        disable_flag(Self::ForceSidePanelOpen);
        disable_flag(Self::ShowRewardModal);
        disable_flag(Self::HideOpenOnDesktopButton);
        disable_flag(Self::RunWorkflow);
        disable_flag(Self::CreateSharedSession);
        disable_flag(Self::CreateNewSession);
        disable_flag(Self::CloseWindow);
        disable_flag(Self::PromptForVersionUpdates);
        disable_flag(Self::WarpEssentials);
        disable_flag(Self::NetworkLogConsole);
        disable_flag(Self::ShowMCPServers);
    }

    pub fn set_settings_link_only() {
        disable_flag(Self::ForceSidePanelOpen);
        disable_flag(Self::ShowRewardModal);
        disable_flag(Self::HideOpenOnDesktopButton);
        disable_flag(Self::RunWorkflow);
        disable_flag(Self::CreateSharedSession);
        disable_flag(Self::CreateNewSession);
        disable_flag(Self::CloseWindow);
        disable_flag(Self::PromptForVersionUpdates);
        disable_flag(Self::WarpEssentials);
        disable_flag(Self::NetworkLogConsole);
        disable_flag(Self::AllowSettingsModalToClose);
        disable_flag(Self::ShowSlowShellStartupBanner);
        disable_flag(Self::DynamicBrowserUrl);
        disable_flag(Self::ShowMCPServers);
    }

    pub fn set_warp_drive_link_only() {
        disable_flag(Self::ForceSidePanelOpen);
        disable_flag(Self::ShowRewardModal);
        disable_flag(Self::HideOpenOnDesktopButton);
        disable_flag(Self::RunWorkflow);
        disable_flag(Self::CreateSharedSession);
        disable_flag(Self::CreateNewSession);
        disable_flag(Self::CloseWindow);
        disable_flag(Self::PromptForVersionUpdates);
        disable_flag(Self::WarpEssentials);
        disable_flag(Self::NetworkLogConsole);
        disable_flag(Self::ShowMCPServers);
    }

    // ContextFlag flag sets:
    pub fn set_shared_session_only() {
        disable_flag(Self::CreateSharedSession);
        disable_flag(Self::CreateNewSession);
        disable_flag(Self::CloseWindow);
        disable_flag(Self::ForceSidePanelOpen);
        disable_flag(Self::ShowRewardModal);
        disable_flag(Self::HideOpenOnDesktopButton);
        disable_flag(Self::PromptForVersionUpdates);
        disable_flag(Self::NetworkLogConsole);
        disable_flag(Self::LaunchConfigurations);
        disable_flag(Self::WarpEssentials);
        disable_flag(Self::ShowMCPServers);
    }

    pub fn set_conversation_only() {
        disable_flag(Self::CreateSharedSession);
        disable_flag(Self::CreateNewSession);
        disable_flag(Self::CloseWindow);
        disable_flag(Self::ForceSidePanelOpen);
        disable_flag(Self::ShowRewardModal);
        disable_flag(Self::HideOpenOnDesktopButton);
        disable_flag(Self::PromptForVersionUpdates);
        disable_flag(Self::NetworkLogConsole);
        disable_flag(Self::LaunchConfigurations);
        disable_flag(Self::WarpEssentials);
        disable_flag(Self::ShowMCPServers);
        disable_flag(Self::RunWorkflow);
    }
}

impl FromStr for ContextFlag {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CreateSharedSession" => Ok(Self::CreateSharedSession),
            "CreateNewSession" => Ok(Self::CreateNewSession),
            "CloseWindow" => Ok(Self::CloseWindow),
            "ForceSidePanelOpen" => Ok(Self::ForceSidePanelOpen),
            "ShowRewardModal" => Ok(Self::ShowRewardModal),
            "HideOpenOnDesktopButton" => Ok(Self::HideOpenOnDesktopButton),
            "PromptForVersionUpdates" => Ok(Self::PromptForVersionUpdates),
            "NetworkLogConsole" => Ok(Self::NetworkLogConsole),
            "RunWorkflow" => Ok(Self::RunWorkflow),
            "LaunchConfigurations" => Ok(Self::LaunchConfigurations),
            "WarpEssentials" => Ok(Self::WarpEssentials),
            _ => Err(()),
        }
    }
}
