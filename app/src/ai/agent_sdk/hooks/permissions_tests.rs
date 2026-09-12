use super::*;

#[test]
fn oz_hooks_permissions_never_upgrade_a_native_denial() {
    assert_eq!(
        compose_permission(NativePermission::Deny, HookPermission::Continue),
        ComposedPermission::DeniedByWarp
    );
    assert_eq!(
        compose_permission(NativePermission::Deny, HookPermission::Deny),
        ComposedPermission::DeniedByWarp
    );
}

#[test]
fn oz_hooks_permissions_preserve_prompt_after_hook_continuation() {
    assert_eq!(
        compose_permission(NativePermission::Prompt, HookPermission::Continue),
        ComposedPermission::Prompt
    );
    assert_eq!(
        compose_permission(NativePermission::Prompt, HookPermission::Deny),
        ComposedPermission::DeniedByHook
    );
}
