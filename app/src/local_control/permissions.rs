//! Permission checks that map protocol action metadata onto local settings.
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ai::execution_profiles::{AIExecutionProfile, WarpControlPermission};
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::settings::{LocalControlPermissionCategory, LocalControlSettings};
use ::local_control::auth::CredentialGrant;
use ::local_control::{
    Action, ActionKind, ControlError, ErrorCode, InvocationContext, PermissionCategory,
};
use warpui::{ModelContext, SingletonEntity};

use crate::local_control::LocalControlBridge;

#[cfg(test)]
static TEST_ALLOW_INPUT_RUN_POLICY: AtomicBool = AtomicBool::new(false);

pub(super) fn warp_control_cli_enabled() -> bool {
    FeatureFlag::WarpControlCli.is_enabled()
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
pub(crate) fn outside_warp_action_enabled_for_settings(
    settings: &LocalControlSettings,
    action: ActionKind,
) -> bool {
    outside_warp_permission_enabled_for_settings(settings, action.metadata().permission_category)
}

#[cfg(test)]
fn outside_warp_permission_enabled_for_settings(
    settings: &LocalControlSettings,
    permission: PermissionCategory,
) -> bool {
    settings.allows_outside_warp(local_permission(permission))
}

#[cfg(test)]
pub(crate) fn capabilities() -> Vec<ActionKind> {
    ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect()
}

fn local_permission(permission: PermissionCategory) -> LocalControlPermissionCategory {
    match permission {
        PermissionCategory::ReadMetadata => LocalControlPermissionCategory::MetadataReads,
        PermissionCategory::ReadUnderlyingData => {
            LocalControlPermissionCategory::UnderlyingDataReads
        }
        PermissionCategory::MutateAppState => LocalControlPermissionCategory::AppStateMutations,
        PermissionCategory::MutateMetadataConfiguration => {
            LocalControlPermissionCategory::MetadataConfigurationMutations
        }
        PermissionCategory::MutateUnderlyingData => {
            LocalControlPermissionCategory::UnderlyingDataMutations
        }
    }
}

#[allow(dead_code)]
pub(crate) fn agent_profile_permission_for_action(
    profile: &AIExecutionProfile,
    action: ActionKind,
) -> WarpControlPermission {
    agent_profile_permission_for_category(profile, action.metadata().permission_category)
}

#[allow(dead_code)]
pub(crate) fn agent_profile_permission_for_category(
    profile: &AIExecutionProfile,
    permission: PermissionCategory,
) -> WarpControlPermission {
    profile.warp_control_permission_for_category(local_permission(permission))
}

#[allow(dead_code)]
pub(crate) fn ensure_agent_profile_allows_action(
    profile: &AIExecutionProfile,
    action: ActionKind,
) -> Result<(), ControlError> {
    let permission = agent_profile_permission_for_action(profile, action);
    if permission.is_denied() {
        return Err(ControlError::new(
            ErrorCode::InsufficientPermissions,
            format!(
                "agent profile denies the Warp control permission required by {}",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ::local_control::{Action, InstanceId};
    use chrono::Duration;

    use super::*;

    #[test]
    fn input_run_policy_is_fail_closed_without_test_override() {
        let grant = CredentialGrant::new(
            InstanceId("inst".to_owned()),
            ActionKind::InputRun,
            InvocationContext::OutsideWarp,
            Duration::minutes(5),
        );
        let action = Action::new(ActionKind::InputRun);

        let err = ensure_input_run_policy_allows(&grant, &action)
            .expect_err("input.run requires explicit policy approval");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);

        let _guard = allow_input_run_policy_for_test();
        ensure_input_run_policy_allows(&grant, &action)
            .expect("test policy override allows input.run");
    }
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
    let permission = local_permission(action.metadata().permission_category);
    match context {
        InvocationContext::InsideWarp => {
            if !settings.inside_warp_control_enabled() {
                return Err(ControlError::new(
                    ErrorCode::LocalControlDisabled,
                    "local control is disabled for this invocation context",
                ));
            }
            if !settings.inside_warp_permission_enabled(permission) {
                return Err(ControlError::new(
                    ErrorCode::InsufficientPermissions,
                    format!(
                        "{} requires a local-control permission that is disabled",
                        action.as_str()
                    ),
                ));
            }
        }
        InvocationContext::OutsideWarp => {
            if !settings.outside_warp_control_enabled() {
                return Err(ControlError::new(
                    ErrorCode::LocalControlDisabled,
                    "local control is disabled for this invocation context",
                ));
            }
            if !settings.outside_warp_permission_enabled(permission) {
                return Err(ControlError::new(
                    ErrorCode::InsufficientPermissions,
                    format!(
                        "{} requires a local-control permission that is disabled",
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
    AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .map(|uid| Some(uid.as_string()))
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                format!("{} requires a logged-in Warp user", action.as_str()),
            )
        })
}

pub(crate) fn ensure_input_run_policy_allows(
    grant: &CredentialGrant,
    action: &Action,
) -> Result<(), ControlError> {
    if input_run_policy_allows(grant, action) {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::InsufficientPermissions,
        "input.run requires explicit local approval policy before command execution",
    ))
}

#[cfg(not(test))]
fn input_run_policy_allows(_grant: &CredentialGrant, _action: &Action) -> bool {
    false
}

#[cfg(test)]
fn input_run_policy_allows(grant: &CredentialGrant, action: &Action) -> bool {
    grant.action == ActionKind::InputRun
        && action.kind == ActionKind::InputRun
        && TEST_ALLOW_INPUT_RUN_POLICY.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn allow_input_run_policy_for_test() -> TestInputRunPolicyGuard {
    TestInputRunPolicyGuard {
        previous: TEST_ALLOW_INPUT_RUN_POLICY.swap(true, Ordering::SeqCst),
    }
}

#[cfg(test)]
pub(crate) struct TestInputRunPolicyGuard {
    previous: bool,
}

#[cfg(test)]
impl Drop for TestInputRunPolicyGuard {
    fn drop(&mut self) {
        TEST_ALLOW_INPUT_RUN_POLICY.store(self.previous, Ordering::SeqCst);
    }
}

pub(super) fn ensure_authenticated_user_matches(
    grant: &CredentialGrant,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if !grant.authenticated_user.required {
        return Ok(());
    }
    let subject = AuthStateProvider::as_ref(ctx)
        .get()
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
