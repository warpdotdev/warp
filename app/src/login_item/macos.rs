//! macOS implementation of login-item registration via `SMAppService`.
//!
//! Requires macOS 13 (Ventura) or later. When `SMAppService` isn't available
//! at runtime, registration silently no-ops — the user-facing setting still
//! updates, but we don't try to register against a class that isn't there.

use ::settings::Setting;
use warp_errors::report_if_error;
use warpui::{AppContext, SingletonEntity};

use crate::terminal::general_settings::GeneralSettings;

pub(super) fn maybe_register_app_as_login_item(ctx: &mut AppContext) {
    GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
        let add_app_as_login_item = *settings.add_app_as_login_item;
        if add_app_as_login_item && *settings.app_added_as_login_item {
            // App has already been added as a login item, so we don't need to do anything.
            // We don't want to re-run the adding logic because it breaks the case where
            // a user manually unregisters the app as a login item in System Preferences >
            // Users & Groups > Login Items by causing it to re-register.
            return;
        }

        // This can be slow, so we run it in a background thread.
        ctx.spawn(
            async move {
                use objc2::runtime::{AnyClass, AnyObject};
                use objc2::{class, msg_send};

                // `[NSBundle mainBundle]` is nil when we're not running from a
                // bundle, so skip registration in that case.
                let bundle: *mut AnyObject = unsafe { msg_send![class!(NSBundle), mainBundle] };
                if bundle.is_null() {
                    log::debug!("Not running in a bundle, so not registering as a login item");
                    return false;
                }

                // Note this only works on macOS 13+ (Ventura and later) so we check for the presence of the class.
                if let Some(sm_app_service_class) = AnyClass::get(c"SMAppService") {
                    let app_service: *mut AnyObject =
                        unsafe { msg_send![sm_app_service_class, mainAppService] };
                    let mut error: *mut AnyObject = std::ptr::null_mut();
                    if add_app_as_login_item {
                        let result: bool =
                            unsafe { msg_send![app_service, registerAndReturnError: &mut error] };
                        if !result && !error.is_null() {
                            log::warn!("Failed to register app as login item.");
                        } else {
                            return true;
                        }
                    } else {
                        let result: bool =
                            unsafe { msg_send![app_service, unregisterAndReturnError: &mut error] };
                        if !result && !error.is_null() {
                            // Note that this can happen if the user has already unregistered the app as a login item
                            // manually in the System Preferences > Users & Groups > Login Items list.
                            log::warn!("Failed to unregister app as login item.");
                        }
                    }
                }
                false
            },
            |settings, app_added_as_login_item, ctx| {
                report_if_error!(
                    settings
                        .app_added_as_login_item
                        .set_value(app_added_as_login_item, ctx)
                );
            },
        );
    });
}
