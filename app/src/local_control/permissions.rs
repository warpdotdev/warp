//! Permission checks that map invocation context onto local settings.
use ::local_control::{ActionKind, ControlError, ErrorCode, InvocationContext, PROTOCOL_VERSION};
use warpui::{ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
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
    let metadata = action.metadata();
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
        }
        InvocationContext::OutsideWarp => {
            if metadata.requires_authenticated_user {
                return Err(ControlError::new(
                    ErrorCode::ExecutionContextNotAllowed,
                    format!(
                        "{} requires verified Warp-terminal invocation",
                        action.as_str()
                    ),
                ));
            }
            if !settings.outside_warp_control_enabled() {
                return Err(ControlError::new(
                    ErrorCode::LocalControlDisabled,
                    format!(
                        "{} is disabled for outside-Warp local control",
                        action.as_str()
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn authenticated_user_subject_for_action(
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Option<String>, ControlError> {
    if !action.metadata().requires_authenticated_user {
        return Ok(None);
    }
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            format!("{} requires a logged-in Warp user", action.as_str()),
        ));
    }
    auth_state
        .user_id()
        .map(|uid| Some(uid.as_string()))
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                format!("{} requires a logged-in Warp user", action.as_str()),
            )
        })
}

pub(super) fn ensure_authenticated_user_matches(
    grant: &::local_control::auth::CredentialGrant,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if !grant.authenticated_user.required {
        return Ok(());
    }
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            format!("{} requires a logged-in Warp user", grant.action.as_str()),
        ));
    }
    let subject = auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                format!("{} requires a logged-in Warp user", grant.action.as_str()),
            )
        })?;
    if grant.authenticated_user.subject.as_deref() != Some(subject.as_str()) {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserRequired,
            format!(
                "{} credential is bound to a different Warp user",
                grant.action.as_str()
            ),
        ));
    }
    Ok(())
}
