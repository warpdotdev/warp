use warp_cli::RecoveryMechanism;
use warpui::{AppContext, SingletonEntity as _, ViewContext};

use super::{Workspace, WorkspaceBannerFields};
use crate::crash_recovery::CrashRecovery;

pub fn banner_metadata(ctx: &AppContext) -> Option<WorkspaceBannerFields> {
    let crash_recovery = CrashRecovery::as_ref(ctx);

    let recovery_mechanism = crash_recovery.should_notify_user_about_crash()?;

    match recovery_mechanism {
        #[cfg(target_os = "linux")]
        RecoveryMechanism::X11 => Some(WorkspaceBannerFields {
            banner_type: super::WorkspaceBanner::WaylandCrashRecovery,
            severity: super::BannerSeverity::Warning,
            heading: None,
            description: "We detected a crash during application startup, and adjusted your \
                settings to use Xwayland for windowing. This can result in blurry text if you \
                are using fractional scaling."
                .to_owned(),
            secondary_button: None,
            button: Some(super::WorkspaceBannerButtonDetails {
                text: "Learn More".to_owned(),
                action: super::WorkspaceAction::DismissWaylandCrashRecoveryBannerAndOpenLink,
                variant: super::BannerButtonVariant::Outlined,
                icon: None,
                more_info_button_action: None,
            }),
        }),
        // We're not showing anything to the user when we recover from a crash
        // by switching from preferring integrated to dedicated gpu due to the
        // fact that this recovery mechanism is only used when the user has not
        // explicitly set their preference.
        RecoveryMechanism::DedicatedGpu => None,
        // We don't show any information to the user for the disable OpenGL / force Vulkan recovery
        // mechanisms. These set of crashes occur before there is a visible window, so any
        // information surfaced to the user would be unactionable noise that the user would see on
        // every invocation of Warp.
        RecoveryMechanism::DisableOpenGL | RecoveryMechanism::ForceVulkan => None,
    }
}

#[cfg_attr(all(enable_crash_recovery, not(target_os = "linux")), allow(unused))]
pub fn dismiss_workspace_banner(ctx: &mut ViewContext<Workspace>) {
    CrashRecovery::handle(ctx).update(ctx, |crash_recovery, ctx| {
        crash_recovery.handle_user_acknowledged_crash(ctx);
    });
}
