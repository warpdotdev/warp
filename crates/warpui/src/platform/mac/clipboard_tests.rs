//! Memory-behavior repro for APP-4154 batch 1.C (warpui-platform-nsstring).
//!
//! Exercises the `NSString::alloc(nil).init_str(...)` → `make_nsstring(...)` conversions
//! applied to `pasteboard_type_for_image_mime_type` and related clipboard
//! helpers. The helper is shared by every retained-NSString site in this file
//! (6 in `read_image_data_from_pasteboard`, 2 in `Clipboard::write`, and this
//! one in `pasteboard_type_for_image_mime_type`), so it is representative for
//! the whole file.
//!
//! On master the raw `NSString::alloc(nil).init_str(...)` returns a string with
//! a +1 retain count that no pool drain can clean up, so even inside an outer
//! `NSAutoreleasePool` the string survives past `pool.drain()` and memory grows
//! linearly with iteration count.
//!
//! On the PR branch `make_nsstring` autoreleases, so every string returned here
//! is released when the surrounding pool drains and peak RSS stays flat across
//! outer iterations.
//!
//! Run as:
//!   cargo test --release -p warpui \
//!       pasteboard_type_for_image_mime_type_memory_behavior -- --nocapture --ignored
//! and measure peak RSS with `/usr/bin/time -l`.
use cocoa::base::nil;
use cocoa::foundation::NSAutoreleasePool;
use objc2::msg_send;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
use objc2_foundation::{NSArray, NSData, NSString, ns_string};
use warpui_core::clipboard::ClipboardContent;

use super::{Clipboard, pasteboard_type_for_image_mime_type};
use crate::Clipboard as _;

/// Number of outer pool cycles. Each cycle creates an `NSAutoreleasePool`,
/// runs the inner loop, then drains. On master the retained NSStrings survive
/// the drain, so memory usage grows proportionally to OUTER * INNER.
const OUTER: usize = 60;

/// Number of inner iterations per pool cycle. Must be large enough to produce
/// a measurable RSS delta but small enough to fit easily in memory on the
/// branch side (where strings are reclaimed per cycle).
const INNER: usize = 20_000;

const MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
];

#[test]
#[ignore = "memory repro; run with --ignored --nocapture in release mode"]
fn pasteboard_type_for_image_mime_type_memory_behavior() {
    unsafe {
        for _ in 0..OUTER {
            let pool = NSAutoreleasePool::new(nil);
            for _ in 0..INNER {
                for mime in MIME_TYPES {
                    let ns = pasteboard_type_for_image_mime_type(mime);
                    assert!(ns.is_some(), "mime {mime} should be mapped");
                }
            }
            pool.drain();
        }
    }
}

/// A pasteboard private to the test, so it doesn't touch the user's clipboard.
fn unique_pasteboard(types: &[&NSString]) -> Clipboard {
    let pasteboard = NSPasteboard::pasteboardWithUniqueName();
    unsafe { pasteboard.declareTypes_owner(&NSArray::from_slice(types), None) };
    Clipboard(pasteboard)
}

/// Reads the pasteboard, then frees it in the pasteboard server.
fn read_and_release(mut clipboard: Clipboard) -> ClipboardContent {
    let content = clipboard.read();
    let _: () = unsafe { msg_send![&*clipboard.0, releaseGlobally] };
    content
}

fn set_tiff(clipboard: &Clipboard) {
    let tiff = NSData::with_bytes(b"MM\x00\x2a");
    clipboard
        .0
        .setData_forType(Some(&tiff), ns_string!("public.tiff"));
}

#[test]
fn finder_copy_reads_paths_not_icon_image() {
    // Finder puts the file URL and the file's icon (as TIFF) on the pasteboard.
    let clipboard = unique_pasteboard(&[
        unsafe { NSPasteboardTypeFileURL },
        ns_string!("public.tiff"),
    ]);
    unsafe {
        clipboard.0.setString_forType(
            ns_string!("file:///Users/alice/Projects"),
            NSPasteboardTypeFileURL,
        );
    }
    set_tiff(&clipboard);

    let content = read_and_release(clipboard);

    assert_eq!(
        content.paths,
        Some(vec!["/Users/alice/Projects".to_string()])
    );
    assert!(!content.has_image_data());
}

#[test]
fn tiff_without_paths_reads_image() {
    let clipboard = unique_pasteboard(&[ns_string!("public.tiff")]);
    set_tiff(&clipboard);

    let content = read_and_release(clipboard);

    assert_eq!(content.num_paths(), 0);
    let images = content
        .images
        .expect("TIFF data should be read as an image");
    assert_eq!(images[0].mime_type, "image/tiff");
}
