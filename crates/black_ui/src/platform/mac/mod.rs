#![allow(deprecated)]

mod app;
pub mod clipboard;
pub mod delegate;
mod event;
pub(crate) mod fonts;
mod geometry;
mod keycode;
mod menus;
mod notification;
pub(super) mod rendering;
mod text_layout;
pub mod utils;
mod window;

pub use app::{App, AppExt};
use clipboard::*;
use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSString};
pub use delegate::{AppDelegate, IntegrationTestDelegate};
pub use fonts::FontDB;
use geometry::*;
use objc::{msg_send, sel, sel_impl};
pub use rendering::is_low_power_gpu_available;
pub use window::{Window, WindowExt};

/// Create an autoreleased NSString from a string reference.
pub fn make_nsstring<S>(s: S) -> id
where
    S: AsRef<str>,
{
    unsafe { NSString::alloc(nil).init_str(s.as_ref()).autorelease() }
}

/// Holds a Cocoa autorelease pool and drains it when the guard is dropped.
///
/// Many Cocoa APIs temporarily hold on to objects that only get freed when an
/// enclosing autorelease pool is drained. AppKit's main event loop and GCD
/// blocks create one of these pools around each callback, so most code doesn't
/// have to think about it. But code that runs during app startup, on a thread
/// Rust created itself, or in a tight loop inside a single event can't rely on
/// the outer pool: objects accumulate in memory until that outer pool drains,
/// which can be a long time.
///
/// Create a `AutoreleasePoolGuard` in that scope to open your own pool. The
/// guard drains the pool automatically when it goes out of scope, whether the
/// function returns normally, returns early via `?`, or unwinds due to a
/// panic.
pub struct AutoreleasePoolGuard(id);

impl AutoreleasePoolGuard {
    /// Creates a fresh `NSAutoreleasePool` whose lifetime is tied to the guard.
    pub fn new() -> Self {
        // SAFETY: `NSAutoreleasePool::new` is infallible and produces a pool
        // that is valid for the current thread until the guard drains it on
        // `Drop`.
        Self(unsafe { NSAutoreleasePool::new(nil) })
    }
}

impl Default for AutoreleasePoolGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AutoreleasePoolGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` was produced by `NSAutoreleasePool::new` in
        // `Self::new` and is drained at most once here.
        unsafe {
            let _: () = msg_send![self.0, drain];
        }
    }
}
