use std::ptr::NonNull;

use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::NSBundle;
use warp_core::channel::{Channel, ChannelState};

// Launch Services constants
type LSRolesMask = u32;
type OSStatus = i32;

// https://github.com/kornelski/core-services/blob/5572befea9fae3c31310d875240342229afa14ca/src/launch_services.rs#L33
const K_LS_ROLES_SHELL: LSRolesMask = 0x00000008;

unsafe extern "C" {
    // Launch Services bindings
    fn LSCopyDefaultRoleHandlerForContentType(
        in_content_type: &CFString,
        in_role: LSRolesMask,
    ) -> *mut CFString;

    fn LSSetDefaultRoleHandlerForContentType(
        in_content_type: &CFString,
        in_role: LSRolesMask,
        in_handler_bundle_id: &CFString,
    ) -> OSStatus;
}

pub fn can_become_default_terminal() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some() && ChannelState::channel() != Channel::Local
}

pub fn is_warp_default_terminal() -> bool {
    let unix_executable_content_type = CFString::from_str("public.unix-executable");
    let handler = unsafe {
        LSCopyDefaultRoleHandlerForContentType(&unix_executable_content_type, K_LS_ROLES_SHELL)
    };

    let Some(handler) = NonNull::new(handler) else {
        return false;
    };

    // `LSCopyDefaultRoleHandlerForContentType` follows the Core Foundation
    // create rule, so take ownership of the +1 reference immediately;
    // `CFRetained` releases it on every exit path.
    let handler_string = unsafe { CFRetained::from_raw(handler) };

    let Some(warp_bundle_id) = get_warp_bundle_id() else {
        return false;
    };

    let current_handler = handler_string.to_string();

    current_handler == warp_bundle_id
}

pub fn set_warp_as_default_terminal() -> Result<(), String> {
    log::debug!("Setting Warp as default terminal");

    let bundle_id = get_warp_bundle_id().ok_or("No bundle ID".to_string())?;

    set_default_terminal(&bundle_id)
}

fn set_default_terminal(bundle_id: &str) -> Result<(), String> {
    log::debug!("Setting default terminal to bundle ID: {bundle_id}");

    let unix_executable_content_type = CFString::from_str("public.unix-executable");

    let bundle_id_cf = CFString::from_str(bundle_id);

    let result = unsafe {
        LSSetDefaultRoleHandlerForContentType(
            &unix_executable_content_type,
            K_LS_ROLES_SHELL,
            &bundle_id_cf,
        )
    };

    match result {
        0 => Ok(()),
        _ => Err(format!(
            "LSSetDefaultRoleHandlerForContentType failed with stats: {result}"
        )),
    }
}

/// Gets Warp's bundle identifier. This may be `None` if not running as a bundle, i.e. through
/// `cargo run` without `cargo bundle`.
fn get_warp_bundle_id() -> Option<String> {
    Some(NSBundle::mainBundle().bundleIdentifier()?.to_string())
}
