//! ContextFlag flags are for behaviors that need to be conditionally enabled or disabled based
//! on where the app is being run and are a permanent part of the app.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use enum_iterator::{Sequence, cardinality};
use warp_errors::report_error;

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
        overrides::get_override(*self)
            .unwrap_or_else(|| FLAG_STATES[*self as usize].load(Ordering::Relaxed))
    }

    /// Sets a thread-local test override for this flag, lasting until the returned guard is
    /// dropped. Prefer this over [`ContextFlag::set`] in tests: the global flag state is shared
    /// process-wide, so mutating it leaks into every other test in the same process.
    #[cfg(feature = "test-util")]
    pub fn override_enabled(self, enabled: bool) -> overrides::ContextFlagOverrideGuard {
        overrides::override_flag(self, enabled)
    }

    /// Sets a ContextFlag flag. FOR DEBUG USE ONLY.
    pub fn set(&self, value: bool) {
        if !ChannelState::enable_debug_features() {
            report_error!(
                "Tried to set value of ContextFlag in non-dogfood context",
                extra: { "flag" => ?self }
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

#[cfg(not(feature = "test-util"))]
mod overrides {
    #[inline(always)]
    pub fn get_override(_flag: super::ContextFlag) -> Option<bool> {
        None
    }
}

/// Thread-local context flag overrides for unit tests.
#[cfg(feature = "test-util")]
mod overrides {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::ContextFlag;

    thread_local! {
        static FLAG_OVERRIDES: RefCell<HashMap<ContextFlag, bool>> = RefCell::new(HashMap::new());
    }

    /// RAII guard for a thread-local context flag override. Dropping it reverts to the global
    /// flag state.
    #[must_use = "if unused the override will be immediately cleared"]
    pub struct ContextFlagOverrideGuard {
        flag: ContextFlag,
    }

    impl Drop for ContextFlagOverrideGuard {
        fn drop(&mut self) {
            FLAG_OVERRIDES.with(|overrides| {
                overrides.borrow_mut().remove(&self.flag);
            });
        }
    }

    pub fn get_override(flag: ContextFlag) -> Option<bool> {
        FLAG_OVERRIDES.with(|overrides| overrides.borrow().get(&flag).copied())
    }

    pub fn override_flag(flag: ContextFlag, enabled: bool) -> ContextFlagOverrideGuard {
        FLAG_OVERRIDES.with(|overrides| {
            let previous = overrides.borrow_mut().insert(flag, enabled);
            assert!(
                previous.is_none(),
                "Nested overrides of ContextFlag::{flag:?} are not supported"
            );
        });
        ContextFlagOverrideGuard { flag }
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
