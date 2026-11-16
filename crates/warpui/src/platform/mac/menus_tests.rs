//! Memory-behavior repros for APP-4154 batch 1.C (warpui-platform-nsstring).
//!
//! Covers two kinds of fix in `menus.rs`:
//!
//! 1. Retain → autorelease conversion (line 298, `make_menu_item` standard
//!    action): the key-equivalent NSString used to be `NSString::alloc(nil)
//!    .init_str(...)`, which returns a +1 retained reference. The PR uses
//!    `make_nsstring`, which autoreleases. Covered by
//!    [`make_menu_item_standard_action_memory_behavior`].
//!
//! 2. Local `NSAutoreleasePool` wrapper around `apply_changes` body. The
//!    NSString temporaries produced by `make_nsstring(name)` and inside
//!    `resolve_key_equivalent` used to go into whatever ambient pool AppKit
//!    had set up (or leak if called from a Rust thread with no active pool).
//!    The PR drains them per call. Covered by
//!    [`apply_changes_local_pool_memory_behavior`], which deliberately runs
//!    WITHOUT any outer `NSAutoreleasePool` so the local-pool drain is the
//!    only thing that can release the temporaries.
use cocoa::appkit::NSMenuItem;
use cocoa::base::nil;
use cocoa::foundation::NSAutoreleasePool;
use objc::runtime::Object;
use objc::{msg_send, sel, sel_impl};
use warpui_core::actions::StandardAction;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::menu::{MenuItem, MenuItemPropertyChanges};

use super::{apply_changes, make_menu_item};

/// How many outer pool cycles for the retain → autorelease test.
const MENU_ITEM_OUTER: usize = 40;
/// Inner iterations per outer cycle. Each one allocates one NSMenuItem plus
/// (on master) one retained NSString for the key equivalent.
const MENU_ITEM_INNER: usize = 10_000;

/// Driver for the local-pool wrapper test. `apply_changes` creates a handful
/// of NSString temporaries per call; without an outer pool, master accumulates
/// them all, while the branch drains them per iteration.
const APPLY_CHANGES_ITERS: usize = 200_000;

/// Reproduces the per-call NSString leak fixed by switching the key-equivalent
/// argument to `make_nsstring` on line 298. Each outer cycle gets its own
/// autorelease pool; the branch reclaims everything on drain, master keeps the
/// retained key-equivalent strings alive.
#[test]
fn make_menu_item_standard_action_memory_behavior() {
    unsafe {
        for _ in 0..MENU_ITEM_OUTER {
            let pool = NSAutoreleasePool::new(nil);
            for _ in 0..MENU_ITEM_INNER {
                // `Quit` has a non-empty key equivalent ("q"); `Close Window`
                // has an empty one. Mix the two so we cover both branches.
                let _ = make_menu_item(MenuItem::Standard(StandardAction::Quit));
                let _ = make_menu_item(MenuItem::Standard(StandardAction::Close));
            }
            pool.drain();
        }
    }
}

/// Reproduces the accumulation that the `apply_changes` local pool prevents.
/// Note the deliberate absence of an outer `NSAutoreleasePool` — this is what
/// makes the local-pool wrapper observable.
#[test]
fn apply_changes_local_pool_memory_behavior() {
    unsafe {
        // Hold a single menu item for the entire loop so that the only growth
        // we measure is the NSString temporaries inside `apply_changes`, not
        // the menu item objects themselves.
        let outer_pool = NSAutoreleasePool::new(nil);
        let item: *mut Object = msg_send![NSMenuItem::alloc(nil), init];
        // Retain so we can freely drain the outer pool after constructing it.
        let _: *mut Object = msg_send![item, retain];
        outer_pool.drain();

        for _ in 0..APPLY_CHANGES_ITERS {
            let changes = MenuItemPropertyChanges {
                name: Some("Warp Menu Item".to_string()),
                keystroke: Some(Some(Keystroke {
                    cmd: true,
                    key: "k".to_string(),
                    ..Default::default()
                })),
                disabled: Some(false),
                checked: Some(false),
                submenu: None,
            };
            apply_changes(changes, item);
        }

        // Balance the manual retain above.
        let _: () = msg_send![item, release];
    }
}
