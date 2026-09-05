#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum NativePermission {
    Deny,
    Allow,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum HookPermission {
    Continue,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ComposedPermission {
    DeniedByWarp,
    DeniedByHook,
    Allow,
    Prompt,
}

#[allow(dead_code)]
pub(crate) fn compose_permission(
    native: NativePermission,
    hook: HookPermission,
) -> ComposedPermission {
    match native {
        NativePermission::Deny => ComposedPermission::DeniedByWarp,
        NativePermission::Allow | NativePermission::Prompt if hook == HookPermission::Deny => {
            ComposedPermission::DeniedByHook
        }
        NativePermission::Allow => ComposedPermission::Allow,
        NativePermission::Prompt => ComposedPermission::Prompt,
    }
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
