//! Permission checks that map invocation context onto local settings.
use ::local_control::{ActionKind, ControlError, ErrorCode, InvocationContext, PROTOCOL_VERSION};
use warpui::{ModelContext, SingletonEntity};

use crate::features::FeatureFlag;
use crate::local_control::LocalControlBridge;
use crate::settings::LocalControlSettings;

pub(super) fn warp_control_cli_enabled() -> bool {
    FeatureFlag::WarpControlCli.is_enabled()
}

pub(super) fn ensure_protocol_version(protocol_version: u32) -> Result<(), ControlError> {
    if protocol_version == PROTOCOL_VERSION {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::ProtocolVersionUnsupported,
        format!("unsupported protocol version {protocol_version}"),
    ))
}

pub(super) fn ensure_feature_enabled() -> Result<(), ControlError> {
    if warp_control_cli_enabled() {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "Warp control CLI is disabled by feature flag",
    ))
}

#[cfg(test)]
pub(crate) fn outside_warp_control_enabled_for_settings(settings: &LocalControlSettings) -> bool {
    settings.outside_warp_control_enabled()
}

#[cfg(test)]
pub(crate) fn capabilities() -> Vec<ActionKind> {
    ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect()
}

pub(super) fn ensure_action_allowed(
    context: InvocationContext,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let settings = LocalControlSettings::as_ref(ctx);
    ensure_settings_allow_action(settings, context, action)
}

pub(crate) fn ensure_settings_allow_action(
    settings: &LocalControlSettings,
    context: InvocationContext,
    action: ActionKind,
) -> Result<(), ControlError> {
    match context {
        InvocationContext::InsideWarp => {
            if !settings.inside_warp_control_enabled() {
                return Err(ControlError::new(
                    ErrorCode::LocalControlDisabled,
                    format!(
                        "{} is disabled for inside-Warp local control",
                        action.as_str()
                    ),
                ));
            }
            Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                format!(
                    "{} cannot run from inside-Warp local control until verified terminal proofs are implemented",
                    action.as_str()
                ),
            ))
        }
        InvocationContext::OutsideWarp => {
            if !settings.outside_warp_control_enabled() {
                return Err(ControlError::new(
                    ErrorCode::LocalControlDisabled,
                    format!(
                        "{} is disabled for outside-Warp local control",
                        action.as_str()
                    ),
                ));
            }
            Ok(())
        }
    }
}
