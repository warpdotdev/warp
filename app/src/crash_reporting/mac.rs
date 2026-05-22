#![allow(deprecated)]

use cocoa::base::{id, nil, BOOL};
use cocoa::foundation::NSAutoreleasePool;
use warpui::platform::mac::make_nsstring;

use super::*;

// Functions implemented in objC files.
extern "C" {
    fn startSentry(sentryUrl: id, environment: id, version: id, isDogfood: BOOL);
    fn stopSentry();
    #[allow(dead_code)] // Only gets called when built in debug mode.
    fn crashSentry();
    fn setUser(userId: id);
    fn recordBreadcrumb(message: id, category: id, level: id, seconds_since_epoch: f64);
    fn setTag(key: id, value: id);
}

pub fn init_cocoa_sentry() {
    let endpoint = ChannelState::sentry_url();
    let environment = super::get_environment();

    log::info!("Initializing Sentry for cocoa app with endpoint {endpoint}");
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let dsn = make_nsstring(endpoint);
        let environment_name: &str = environment.as_ref();
        let environment = make_nsstring(environment_name);
        let release = make_nsstring(release_version());
        startSentry(
            dsn,
            environment,
            release,
            ChannelState::channel().is_dogfood() as BOOL,
        );
        pool.drain();
    }
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
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let user_id = make_nsstring(user_id);
        setUser(user_id);
        pool.drain();
    }
}

pub fn forward_breadcrumb(rust_breadcrumb: &sentry::Breadcrumb) {
    let message = rust_breadcrumb.message.as_deref().unwrap_or("");
    let category = rust_breadcrumb.category.as_deref().unwrap_or("");
    let level = rust_breadcrumb.level.to_string();
    let unix_timestamp = rust_breadcrumb
        .timestamp
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0., |n| n.as_secs_f64());
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        recordBreadcrumb(
            make_nsstring(message),
            make_nsstring(category),
            make_nsstring(level.as_str()),
            unix_timestamp,
        );
        pool.drain();
    }
}

pub fn set_tag(key: &str, value: &str) {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        setTag(make_nsstring(key), make_nsstring(value));
        pool.drain();
    }
}
