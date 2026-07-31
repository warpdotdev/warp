use objc2::rc::Retained;
use objc2_foundation::NSString;

use crate::channel::ChannelState;

unsafe extern "C" {
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
/// an `@autoreleasepool` block. `autorelease_return` hands the string to that
/// ambient pool, which takes ownership of it.
#[unsafe(no_mangle)]
extern "C-unwind" fn warp_services_provider_custom_url_scheme() -> *mut NSString {
    Retained::autorelease_return(NSString::from_str(ChannelState::url_scheme()))
}
