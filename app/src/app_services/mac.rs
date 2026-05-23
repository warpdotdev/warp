#[allow(deprecated)]
use cocoa::base::id;
use black_ui::platform::mac::make_nsstring;

use crate::channel::ChannelState;

extern "C" {
    /// ObjC function to create and register the NSServices provider for the
    /// application.
    fn warp_register_services_provider();
}

/// Initializes application services.
pub fn init() {
    unsafe {
        warp_register_services_provider();
    }
}

/// Returns an NSString containing the custom URL scheme that this build of the
/// application will respond to.
///
/// Called synchronously from the NSServices dispatch path in
/// `services.m::forFilesFromPasteboard:performAction:`, which wraps the body in
/// an `@autoreleasepool` block. That ambient pool owns the returned NSString.
#[allow(deprecated)]
#[no_mangle]
extern "C-unwind" fn warp_services_provider_custom_url_scheme() -> id {
    make_nsstring(ChannelState::url_scheme())
}
