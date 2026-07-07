use objc2::rc::autoreleasepool;
use objc2_foundation::NSString;

use super::*;

// Functions implemented in objC files.
extern "C" {
    fn startSentry(
        sentryUrl: &NSString,
        environment: &NSString,
        version: &NSString,
        isDogfood: bool,
    );
    fn stopSentry();
    #[allow(dead_code)] // Only gets called when built in debug mode.
    fn crashSentry();
    fn setUser(userId: &NSString);
    fn recordBreadcrumb(
        message: &NSString,
        category: &NSString,
        level: &NSString,
        seconds_since_epoch: f64,
    );
    fn setTag(key: &NSString, value: &NSString);
}

pub fn init_cocoa_sentry() {
    let endpoint = ChannelState::sentry_url();
    let environment = super::get_environment();

    log::info!("Initializing Sentry for cocoa app with endpoint {endpoint}");
    // This runs during early init from `init_sentry`, before the AppKit event
    // loop drains its ambient pool, so open a local pool to bound the bridge
    // NSStrings.
    autoreleasepool(|_| {
        let dsn = NSString::from_str(endpoint.as_ref());
        let environment_name: &str = environment.as_ref();
        let environment = NSString::from_str(environment_name);
        let release = NSString::from_str(release_version());
        unsafe {
            startSentry(
                &dsn,
                &environment,
                &release,
                ChannelState::channel().is_dogfood(),
            );
        }
    });
}

pub fn uninit_cocoa_sentry() {
    unsafe {
        stopSentry();
    }
}

pub fn crash() {
    unsafe {
        crashSentry();
    }
}

pub fn set_user_id(user_id: &str) {
    // Invoked from `set_optional_user_information` on auth state changes and
    // init, whose thread of origin varies, so open a local pool to bound the
    // bridge NSString.
    autoreleasepool(|_| {
        let user_id = NSString::from_str(user_id);
        unsafe {
            setUser(&user_id);
        }
    });
}

pub fn forward_breadcrumb(rust_breadcrumb: &sentry::Breadcrumb) {
    let message = rust_breadcrumb.message.as_deref().unwrap_or("");
    let category = rust_breadcrumb.category.as_deref().unwrap_or("");
    let level = rust_breadcrumb.level.to_string();
    let unix_timestamp = rust_breadcrumb
        .timestamp
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0., |n| n.as_secs_f64());
    // Runs on whichever Rust thread emitted the breadcrumb (Sentry's
    // `before_breadcrumb`), which has no ambient pool, so bound the bridge
    // NSStrings in a local pool.
    autoreleasepool(|_| {
        let message = NSString::from_str(message);
        let category = NSString::from_str(category);
        let level = NSString::from_str(level.as_str());
        unsafe {
            recordBreadcrumb(&message, &category, &level, unix_timestamp);
        }
    });
}

pub fn set_tag(key: &str, value: &str) {
    // Called from `init_cocoa_sentry`'s tag loop and the `set_tag` wrapper in
    // `mod.rs` on Rust threads, so open a local pool to bound the bridge
    // NSStrings.
    autoreleasepool(|_| {
        let key = NSString::from_str(key);
        let value = NSString::from_str(value);
        unsafe {
            setTag(&key, &value);
        }
    });
}
